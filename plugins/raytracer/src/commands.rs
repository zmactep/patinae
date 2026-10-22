//! Command implementations for the raytracer plugin.
//!
//! Provides the `ray` command (perform raytracing).

use patinae_plugin::prelude::*;

use crate::scene::{raytrace_scene, RaytraceSceneOutput, RaytraceSceneTarget};
use crate::settings::read_ray_settings;

// ---------------------------------------------------------------------------
// RayCommand
// ---------------------------------------------------------------------------

pub(crate) struct RayCommand;

impl Command for RayCommand {
    fn name(&self) -> &str {
        "ray"
    }

    fn runtime_requirements(&self) -> CommandRuntimeRequirements {
        CommandRuntimeRequirements::GPU_COMMANDS.union(CommandRuntimeRequirements::RENDER_ARTIFACTS)
    }

    command_help! {
        CMD "ray"
        DESCRIPTION [
            "performs ray-tracing and saves the resulting image to a file.",
            "Ray tracing produces high-quality images with proper shadows, lighting,",
            "and transparency effects.",
        ]
        REQUIRED []
        OPTIONAL [
            { "width", "integer", "width in pixels", "current window width" },
            { "height", "integer", "height in pixels", "current window height" },
            { "antialias", "integer", "antialiasing level 1-4", "antialias setting" } => [
                "1 = no antialiasing",
                "2 = 2x2 supersampling",
                "3 = 3x3 supersampling",
                "4 = 4x4 supersampling",
            ],
            { "filename", "string", "output file path", "none (returns to display)" },
            { "quiet", "0/1", "suppress feedback", "0" },
        ]
        EXAMPLES [
            "ray                          # Raytrace at current resolution",
            "ray 1920, 1080               # Raytrace at 1080p",
            "ray 1920, 1080, 2            # Raytrace at 1080p with 2x2 AA",
            "ray width=1920, height=1080, filename=output.png",
        ]
        SEE ALSO [
            "png",
        ]
    }

    fn execute<'v, 'r>(
        &self,
        ctx: &mut CommandContext<'v, 'r, dyn ViewerLike + 'v>,
        args: &ParsedCommand,
    ) -> CmdResult {
        use std::time::Instant;

        let width = args
            .get_int(0)
            .or_else(|| args.get_named_int("width"))
            .map(|v| v as u32);

        let height = args
            .get_int(1)
            .or_else(|| args.get_named_int("height"))
            .map(|v| v as u32);

        let antialias =
            args.get_int(2)
                .or_else(|| args.get_named_int("antialias"))
                .unwrap_or_else(|| ctx.viewer.settings().ui.antialias as i64) as u32;

        let filename = args.get_str(3).or_else(|| args.get_named_str("filename"));
        let export_mode = filename.is_some();

        let quiet = args
            .get_bool(4)
            .or_else(|| args.get_named_bool("quiet"))
            .unwrap_or(false);

        // Ensure representations are built (mutable borrow, released immediately)
        ctx.viewer.prepare_render();

        let ray_settings = read_ray_settings(
            |name, default| ctx.setting_bool(name, default),
            |name, default| ctx.setting_int(name, default),
            |name, default| ctx.setting_float(name, default),
            ctx.setting_value("ray_trace_color")
                .and_then(|v| v.as_color())
                .unwrap_or_default(),
        );

        // Determine output dimensions
        let viewport = ctx.viewer.viewport_size();
        let final_width = width.unwrap_or(viewport.0.max(1024));
        let final_height = height.unwrap_or(viewport.1.max(768));

        let start = Instant::now();

        // Perform raytracing (handles borrow-checker dance internally)
        let output = raytrace_scene(
            ctx.viewer,
            &ray_settings,
            Some(final_width),
            Some(final_height),
            antialias,
            if export_mode {
                RaytraceSceneTarget::Export
            } else {
                RaytraceSceneTarget::Viewport
            },
        )
        .map_err(|e| CmdError::execution(format!("Ray tracing failed: {e}")))?;

        let elapsed = start.elapsed();

        match (filename, output) {
            (
                Some(path),
                RaytraceSceneOutput::CpuImage {
                    data: image_data,
                    width: w,
                    height: h,
                    profile_lines,
                },
            ) => {
                let path = expand_path(path);
                let path = if path.extension().map(|e| e.to_ascii_lowercase()) != Some("png".into())
                {
                    path.with_extension("png")
                } else {
                    path.to_path_buf()
                };

                image::save_buffer(&path, &image_data, w, h, image::ColorType::Rgba8)
                    .map_err(|e| CmdError::execution(format!("Failed to save PNG: {e}")))?;

                if !quiet {
                    for line in &profile_lines {
                        ctx.print(line);
                    }
                    ctx.print(&format!(
                        " Ray: render time {:02}:{:02}:{:02}.{:02}  ({}x{})",
                        elapsed.as_secs() / 3600,
                        (elapsed.as_secs() % 3600) / 60,
                        elapsed.as_secs() % 60,
                        elapsed.subsec_millis() / 10,
                        w,
                        h
                    ));
                    ctx.print(&format!(" Saved \"{}\"", path.display()));
                }
            }
            (
                None,
                RaytraceSceneOutput::GpuViewport {
                    width: w,
                    height: h,
                    profile_lines,
                },
            ) => {
                ctx.viewer.request_redraw();
                if !quiet {
                    for line in &profile_lines {
                        ctx.print(line);
                    }
                    ctx.print(&format!(
                        " Ray: render time {:02}:{:02}:{:02}.{:02}  ({}x{})",
                        elapsed.as_secs() / 3600,
                        (elapsed.as_secs() % 3600) / 60,
                        elapsed.as_secs() % 60,
                        elapsed.subsec_millis() / 10,
                        w,
                        h
                    ));
                    ctx.print(
                        " Ray trace complete. Use 'png filename' to save, or interact to dismiss.",
                    );
                }
            }
            (Some(_), RaytraceSceneOutput::GpuViewport { .. }) => {
                return Err(CmdError::execution(
                    "ray export unexpectedly produced a GPU viewport result".to_string(),
                ));
            }
            (None, RaytraceSceneOutput::CpuImage { .. }) => {
                return Err(CmdError::execution(
                    "viewport ray unexpectedly produced CPU image data".to_string(),
                ));
            }
        }

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Helper: expand ~ in paths
// ---------------------------------------------------------------------------

fn expand_path(path: &str) -> std::path::PathBuf {
    if path.starts_with('~') {
        if let Some(home) = dirs_hint() {
            return std::path::PathBuf::from(path.replacen('~', &home, 1));
        }
    }
    std::path::PathBuf::from(path)
}

fn dirs_hint() -> Option<String> {
    std::env::var("HOME").ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ray_command_requests_native_gpu_artifacts() {
        let requirements = RayCommand.runtime_requirements();

        assert!(requirements.contains(CommandRuntimeRequirements::GPU_COMMANDS));
        assert!(requirements.contains(CommandRuntimeRequirements::RENDER_ARTIFACTS));
        assert!(!requirements.contains(CommandRuntimeRequirements::TRACE_GEOMETRY_STREAM));
        assert!(!requirements.contains(CommandRuntimeRequirements::DISPLAYED_GEOMETRY));
        assert!(!requirements.contains(CommandRuntimeRequirements::FULL_SESSION));
    }
}
