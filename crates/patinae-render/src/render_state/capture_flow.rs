//! Offscreen attachments are private to one export size and effect configuration.
//! Geometry, pipelines, scene buffers and lighting stay on the live renderer.

use std::sync::Weak;

use super::state::{FrameBindings, FramePurpose, RenderFrame, RenderState};
use crate::capture::{CaptureError, TextureReadback};
use crate::context::FrameResources;
use crate::frame::{
    FrameTargets, ACCUM_FORMAT, DEPTH_FORMAT, PICKING_FORMAT, REVEAL_FORMAT, SSAO_FORMAT,
};
use crate::memory::{
    buffer_usage, estimate_texture_2d_bytes, FrameTargetPolicy, GpuMemoryCategory, GpuMemoryLedger,
};
use crate::FrameUniforms;

#[derive(Clone, Copy, PartialEq, Eq)]
struct CaptureLayout {
    width: u32,
    height: u32,
    format: wgpu::TextureFormat,
    fxaa: bool,
    ssao: bool,
    silhouette: bool,
}

impl CaptureLayout {
    fn color_bytes(self) -> u64 {
        estimate_texture_2d_bytes(self.width, self.height, self.format)
    }

    fn texture_bytes(self) -> u64 {
        let bytes = |format| estimate_texture_2d_bytes(self.width, self.height, format);
        let mut total =
            self.color_bytes() + bytes(ACCUM_FORMAT) + bytes(REVEAL_FORMAT) + bytes(DEPTH_FORMAT);
        if self.fxaa {
            total += self.color_bytes();
        }
        if self.ssao {
            total += 2 * bytes(SSAO_FORMAT);
        }
        if self.silhouette {
            total += bytes(PICKING_FORMAT) + bytes(DEPTH_FORMAT);
        }
        total
    }

    fn resident_bytes(self) -> u64 {
        self.texture_bytes() + FrameUniforms::SIZE
    }

    fn allocate(self, state: &RenderState) -> CaptureTargets {
        let device = &state.ctx.device;
        let mut frame = FrameTargets::new_with_policy(
            device,
            self.width,
            self.height,
            false,
            self.format,
            FrameTargetPolicy {
                ssao_eager: self.ssao,
                color_scratch_eager: self.fxaa,
                ..state.memory.policy.frame_targets
            },
            state.memory.policy.picking.scale,
        );
        let silhouette_bind_group = if self.silhouette {
            frame.ensure_overlay_id_targets(device);
            state
                .screen
                .silhouette
                .as_ref()
                .zip(frame.overlay_id.as_ref())
                .map(|(pass, view)| pass.make_bind_group(device, view))
        } else {
            None
        };
        debug_assert_eq!(
            self.texture_bytes(),
            self.color_bytes() + frame.memory_usage().capacity_bytes
        );

        let color = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("patinae.png.color"),
            size: wgpu::Extent3d {
                width: self.width,
                height: self.height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: self.format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = color.create_view(&Default::default());
        let uniforms = FrameResources::with_layout(device, &state.ctx.frame.bind_group_layout);
        let bindings = FrameBindings {
            composite_bind_group: state.screen.composite.make_bind_group(
                device,
                &frame.accum,
                &frame.reveal,
            ),
            silhouette_bind_group,
            marking_bind_groups: None,
            fxaa_bind_group: frame
                .color_scratch_view
                .as_ref()
                .map(|view| state.screen.fxaa_pass.make_bind_group(device, view)),
            fxaa_overlay_bind_group: None,
            ssao: state.ssao_bindings(&frame, &uniforms.buffer),
        };
        CaptureTargets {
            uniforms,
            layout: self,
            frame,
            bindings,
            color,
            view,
        }
    }
}

struct CaptureTargets {
    uniforms: FrameResources,
    layout: CaptureLayout,
    frame: FrameTargets,
    bindings: FrameBindings,
    color: wgpu::Texture,
    view: wgpu::TextureView,
}

#[derive(Default)]
pub(super) struct CaptureCache {
    targets: Option<CaptureTargets>,
    // Mapping/unpacking owns the staging allocation, including during Web awaits.
    staging: Weak<wgpu::Buffer>,
}

impl CaptureCache {
    pub(super) fn resident_bytes(&self) -> u64 {
        self.targets
            .as_ref()
            .map_or(0, |target| target.layout.resident_bytes())
    }

    pub(super) fn staging_bytes(&self) -> u64 {
        self.staging.upgrade().map_or(0, |buffer| buffer.size())
    }

    pub(super) fn record_memory(&self, ledger: &mut GpuMemoryLedger) {
        if let Some(targets) = self.targets.as_ref() {
            targets.frame.record_memory(ledger);
            ledger.add_usage(
                GpuMemoryCategory::FrameTargets,
                buffer_usage(&targets.uniforms.buffer),
            );
            ledger.add_allocation(
                GpuMemoryCategory::FrameTargets,
                targets.layout.color_bytes(),
            );
        }
        if let Some(buffer) = self.staging.upgrade() {
            ledger.add_usage(GpuMemoryCategory::Readback, buffer_usage(&buffer));
        }
    }
}

impl RenderState {
    /// Give interactive geometry and picking priority over retained export attachments.
    pub(super) fn reserve_viewport_bytes(&mut self, bytes_without_capture_targets: u64) {
        if self.memory.policy.budget_bytes.is_some_and(|budget| {
            bytes_without_capture_targets.saturating_add(self.capture.resident_bytes()) > budget
        }) {
            self.discard_capture_targets();
        }
    }

    /// Release reusable export attachments, for example when recovering from GPU memory pressure.
    pub fn discard_capture_targets(&mut self) {
        self.capture.targets = None;
    }

    /// Submit a PNG frame using the synchronized scene and active visual effects.
    /// Live viewport attachments, hit testing and editor overlays stay independent.
    /// The returned staging buffer is ready for asynchronous mapping.
    ///
    /// # Errors
    /// Returns an error for overlapping readbacks, unsupported dimensions or a GPU budget overrun.
    pub fn submit_capture(
        &mut self,
        (width, height): (u32, u32),
        mut uniforms: FrameUniforms,
        background: wgpu::Color,
    ) -> Result<TextureReadback, CaptureError> {
        if let Some(error) = &self.memory.sync_error {
            return Err(CaptureError::Gpu(error.to_string()));
        }
        if self.capture.staging.strong_count() != 0 {
            return Err(CaptureError::Gpu(
                "A PNG readback is still in progress".into(),
            ));
        }
        let limits = self.ctx.device.limits();
        if width.max(height) > limits.max_texture_dimension_2d {
            return Err(CaptureError::Gpu(format!(
                "PNG {width}x{height} exceeds the device texture dimension limit of {}",
                limits.max_texture_dimension_2d
            )));
        }
        let (_, staging_bytes) = TextureReadback::layout(width, height)?;
        if staging_bytes > limits.max_buffer_size {
            return Err(CaptureError::Gpu(format!(
                "PNG {width}x{height} requires {staging_bytes} staging bytes, exceeding the device buffer limit of {}",
                limits.max_buffer_size
            )));
        }
        let layout = CaptureLayout {
            width,
            height,
            format: self.ctx.color_format,
            fxaa: self.screen.fxaa_enabled,
            ssao: self.screen.ssao_enabled,
            silhouette: self.screen.silhouette_params.is_some(),
        };
        if layout.silhouette {
            self.ensure_id_pass();
        }
        // The previous export is already fenced by map completion. Drop an obsolete
        // set before allocation and include a matching set exactly once in the budget.
        if self
            .capture
            .targets
            .as_ref()
            .is_some_and(|targets| targets.layout != layout)
        {
            self.discard_capture_targets();
        }
        if let Some(budget) = self.memory.policy.budget_bytes {
            let new_resource_bytes = if self.capture.targets.is_none() {
                layout.resident_bytes()
            } else {
                0
            };
            let peak = self
                .memory_snapshot()
                .total_capacity_bytes()
                .saturating_add(new_resource_bytes)
                .saturating_add(staging_bytes);
            if peak > budget {
                return Err(CaptureError::Gpu(format!(
                    "PNG {width}x{height} needs an estimated {peak} GPU bytes, exceeding the render budget of {budget} bytes"
                )));
            }
        }

        if self.capture.targets.is_none() {
            self.capture.targets = Some(layout.allocate(self));
        }
        uniforms.set_viewport(width, height);
        let mut encoder = self
            .ctx
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("patinae.png"),
            });
        self.prepare_frame_geometry(&mut encoder, &uniforms, FramePurpose::Capture);
        let targets = self
            .capture
            .targets
            .as_ref()
            .expect("capture targets allocated above");
        targets.uniforms.upload(&self.ctx.queue, &uniforms);
        if layout.silhouette {
            self.record_overlay_id_pass(&mut encoder, &targets.frame, &targets.uniforms.bind_group);
        }
        self.render_frame(
            &mut encoder,
            RenderFrame {
                targets: &targets.frame,
                bindings: &targets.bindings,
                uniform_bind_group: &targets.uniforms.bind_group,
                output: &targets.view,
                background,
                purpose: FramePurpose::Capture,
                ssao_phase: 0.0,
            },
        );
        let readback = TextureReadback::record(
            &self.ctx.device,
            &mut encoder,
            &targets.color,
            width,
            height,
            "patinae.png.readback",
        );
        #[cfg(feature = "stats")]
        self.finish_frame_stats(&mut encoder);
        // Submit shared geometry builds even if the readback could not be recorded.
        self.ctx.queue.submit([encoder.finish()]);
        let readback = readback?;
        self.capture.staging = readback.weak_buffer();
        Ok(readback)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn estimates_only_capture_attachments_and_padded_staging() {
        let mut layout = CaptureLayout {
            width: 65,
            height: 3,
            format: wgpu::TextureFormat::Bgra8Unorm,
            fxaa: false,
            ssao: false,
            silhouette: false,
        };
        let pixels = 65 * 3;
        assert_eq!(layout.texture_bytes(), pixels * (4 + 8 + 2 + 4));
        layout.fxaa = true;
        layout.ssao = true;
        layout.silhouette = true;
        assert_eq!(layout.texture_bytes(), pixels * (18 + 4 + 8 + 12));
        assert_eq!(TextureReadback::layout(65, 3).unwrap(), (512, 1536));
    }
    #[test]
    #[ignore = "requires a real GPU"]
    fn gpu_capture_preserves_viewport_and_charges_cache_and_staging_once() {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = pollster::block_on(instance.request_adapter(&Default::default())).unwrap();
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            required_limits: crate::required_device_limits(&adapter.limits()).unwrap(),
            ..Default::default()
        }))
        .unwrap();
        let mut state = RenderState::new(
            device.into(),
            queue.into(),
            wgpu::TextureFormat::Rgba8Unorm,
            (32, 24),
        );
        let live_depth = state.targets.depth_texture.clone();
        let live_pick = state.targets.picking_texture.clone();
        state.set_ssao(true, 2.0, 1.0, 0.02);
        state.set_silhouette(true, 1.0, [0.0, 0.0, 0.0, 1.0]);
        state.set_fxaa(true);
        let live = state.memory_snapshot().total_capacity_bytes();
        let (width, height) = (65, 73);
        let layout = CaptureLayout {
            width,
            height,
            format: state.ctx.color_format,
            fxaa: true,
            ssao: true,
            silhouette: true,
        };
        let (_, staging) = TextureReadback::layout(width, height).unwrap();
        let peak = live + layout.resident_bytes() + staging;
        let capture = |state: &mut RenderState| {
            state.submit_capture((width, height), state.uniforms, wgpu::Color::TRANSPARENT)
        };

        state.memory.policy.budget_bytes = Some(peak - 1);
        assert!(capture(&mut state)
            .unwrap_err()
            .to_string()
            .contains("render budget"));
        assert_eq!(state.memory_snapshot().total_capacity_bytes(), live);
        state.memory.policy.budget_bytes = Some(peak);
        let first = capture(&mut state).unwrap();
        let export = state.capture.targets.as_ref().unwrap();
        assert_ne!(export.uniforms.buffer, state.ctx.frame.buffer);
        assert_ne!(export.uniforms.bind_group, state.ctx.frame.bind_group);
        assert_eq!(state.memory_snapshot().total_capacity_bytes(), peak);
        assert_eq!(state.targets.depth_texture, live_depth);
        assert_eq!(state.targets.picking_texture, live_pick);
        assert!(capture(&mut state).is_err());
        drop(first);
        let target = state.capture.targets.as_ref().unwrap().color.clone();
        assert_eq!(
            state.memory_snapshot().total_capacity_bytes(),
            peak - staging
        );
        drop(capture(&mut state).unwrap());
        assert_eq!(state.capture.targets.as_ref().unwrap().color, target);
        state.reserve_viewport_bytes(peak);
        assert_eq!(state.memory_snapshot().total_capacity_bytes(), live);
    }
}
