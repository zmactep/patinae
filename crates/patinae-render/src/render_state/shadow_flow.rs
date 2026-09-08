use super::math::{
    build_shadow_frame_for_direction, fibonacci_sphere_direction, normalize3, transform_dir,
};
use super::state::{RenderState, SceneBounds, ShadowPassMode};
use crate::passes::atlas_ao::AtlasAoKey;
use crate::passes::lighting::{LightingOcclusionUniforms, MAX_ATLAS_DIRECTIONS};
use crate::picking::RepKind;
use crate::representations::catalog;
#[cfg(feature = "stats")]
use crate::stats::Pass as StatsPass;

impl RenderState {
    pub(super) fn record_shadow_pass(&mut self, encoder: &mut wgpu::CommandEncoder) {
        if self.lighting.shadow_mode == ShadowPassMode::Disabled {
            return;
        }
        let Some(bounds) = self.scene.scene_bounds else {
            return;
        };
        if self.scene.draw_order.is_empty() {
            return;
        }

        match self.lighting.shadow_mode {
            ShadowPassMode::Disabled => {}
            ShadowPassMode::Directional => self.record_directional_shadow_pass(encoder, bounds),
            ShadowPassMode::AtlasAo => self.record_atlas_ao_shadow_pass(encoder, bounds),
        }
    }

    fn record_directional_shadow_pass(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        bounds: SceneBounds,
    ) {
        if self.lighting.shadow_intensity <= 0.0 {
            return;
        }
        let light_view = normalize3([
            -self.uniforms.light_dirs[0][0],
            -self.uniforms.light_dirs[0][1],
            -self.uniforms.light_dirs[0][2],
        ])
        .unwrap_or([0.4, 0.4, 1.0]);
        let light_world =
            normalize3(transform_dir(&self.uniforms.view_inv, light_view)).unwrap_or(light_view);
        let shadow_frame = build_shadow_frame_for_direction(
            &self.uniforms,
            bounds,
            light_world,
            self.lighting.directional_shadow.size,
        );
        let shadow_uniforms = LightingOcclusionUniforms::directional(
            shadow_frame.view_proj,
            self.lighting.shadow_bias,
            self.lighting.shadow_intensity,
            self.lighting.directional_shadow.size,
            self.lighting.shadow_pcf_radius,
        );
        self.prepare_shadow_geometry(encoder);
        self.lighting
            .directional_shadow
            .upload_frame(&self.ctx.queue, &shadow_frame);
        self.ctx
            .lighting
            .upload_uniforms(&self.ctx.queue, &shadow_uniforms);
        self.ctx.lighting.set_depth_view(
            &self.ctx.device,
            &self.lighting.directional_shadow.depth_view,
        );

        #[cfg(feature = "stats")]
        let shadow_ts = self.stats.render_pass_timestamp_writes(StatsPass::Shadow);
        #[cfg(not(feature = "stats"))]
        let shadow_ts = None;
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("patinae.shadow.directional_pass"),
            color_attachments: &[],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &self.lighting.directional_shadow.depth_view,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(1.0),
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: shadow_ts,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_bind_group(0, &self.lighting.directional_shadow.frame_bind_group, &[]);
        pass.set_bind_group(1, &self.ctx.lighting.depth_pass_bind_group, &[]);
        self.draw_shadow_geometry(&mut pass);
    }

    fn record_atlas_ao_shadow_pass(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        bounds: SceneBounds,
    ) {
        if self.lighting.skripkin_intensity <= 0.0 {
            return;
        }
        let count = self
            .lighting
            .skripkin_directions
            .clamp(1, MAX_ATLAS_DIRECTIONS as u32);
        let (grid, tile_size) = self.lighting.atlas_ao.ensure_atlas(
            &self.ctx.device,
            count,
            self.lighting.skripkin_map_size,
        );
        if self.scene.scene_dirty {
            self.lighting.atlas_ao.cache.invalidate();
        }
        let cache_key = AtlasAoKey {
            bounds_center: bounds.center.map(f32::to_bits),
            bounds_radius: bounds.radius.to_bits(),
            shadow_signature: self.shadow_signature(),
            directions: count,
            map_size: self.lighting.skripkin_map_size,
            bias: self.lighting.skripkin_bias.to_bits(),
        };
        let rebuild = self.lighting.atlas_ao.cache.should_rebuild(cache_key);
        let mut matrices: Vec<[[f32; 4]; 4]> = Vec::with_capacity(count as usize);
        for i in 0..count {
            let direction = fibonacci_sphere_direction(i, count);
            let shadow_frame =
                build_shadow_frame_for_direction(&self.uniforms, bounds, direction, tile_size);
            matrices.push(shadow_frame.view_proj);
            self.lighting
                .atlas_ao
                .upload_atlas_frame(&self.ctx.queue, i as usize, &shadow_frame);
        }
        let shadow_uniforms = LightingOcclusionUniforms::atlas_ao(
            &matrices,
            grid,
            tile_size,
            self.lighting.skripkin_bias,
            self.lighting.skripkin_intensity,
        );
        self.ctx
            .lighting
            .upload_uniforms(&self.ctx.queue, &shadow_uniforms);
        self.ctx
            .lighting
            .set_depth_view(&self.ctx.device, &self.lighting.atlas_ao.depth_view);
        #[cfg(feature = "stats")]
        self.stats.record_atlas_ao_cache(rebuild, count, tile_size);

        if !rebuild {
            return;
        }

        self.prepare_shadow_geometry(encoder);

        #[cfg(feature = "stats")]
        let shadow_ts = self.stats.render_pass_timestamp_writes(StatsPass::AtlasAo);
        #[cfg(not(feature = "stats"))]
        let shadow_ts = None;
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("patinae.shadow.atlas_ao_pass"),
            color_attachments: &[],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &self.lighting.atlas_ao.depth_view,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(1.0),
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: shadow_ts,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_bind_group(1, &self.ctx.lighting.depth_pass_bind_group, &[]);
        for i in 0..count {
            let col = i % grid;
            let row = i / grid;
            let x = col * tile_size;
            let y = row * tile_size;
            pass.set_viewport(
                x as f32,
                y as f32,
                tile_size as f32,
                tile_size as f32,
                0.0,
                1.0,
            );
            pass.set_scissor_rect(x, y, tile_size, tile_size);
            let Some(frame_bg) = self.lighting.atlas_ao.atlas_frame_bind_group(i as usize) else {
                continue;
            };
            pass.set_bind_group(0, frame_bg, &[]);
            self.draw_shadow_geometry(&mut pass);
        }
    }

    fn prepare_shadow_geometry(&self, encoder: &mut wgpu::CommandEncoder) {
        for key in &self.scene.draw_order {
            let Some(entry) = self.scene.reps.get(key) else {
                continue;
            };
            if entry.rep.casts_shadow() {
                entry.rep.prepare_shadow_depth(encoder, &self.ctx.queue);
            }
        }
    }

    fn draw_shadow_geometry<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>) {
        let mut current: Option<RepKind> = None;
        for key in &self.scene.draw_order {
            let Some(entry) = self.scene.reps.get(key) else {
                continue;
            };
            if !entry.rep.casts_shadow() {
                continue;
            }
            let kind = entry.rep.kind();
            if current != Some(kind) {
                let Some(pipeline) =
                    catalog::entry(kind).and_then(|rep| rep.shadow_pipeline(&self.geometry))
                else {
                    continue;
                };
                pass.set_pipeline(pipeline);
                current = Some(kind);
            }
            let store = &self.scene.scene_store;
            let Some(bg) = store.bind_group() else {
                continue;
            };
            for &offset in store.draw_offsets(crate::picking::ObjectId(key.0)) {
                pass.set_bind_group(2, bg, &[offset]);
                entry.rep.record_shadow_depth(pass);
            }
        }
    }
}
