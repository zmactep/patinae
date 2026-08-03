//! Shader source loading + lightweight `{{INCLUDE_*}}` preprocessor.
//!
//! Each `.wgsl` file lives in `src/shaders/...` and is embedded with
//! `include_str!`. Representation/postprocess shaders that need the shared
//! includes mark themselves with these directives (one per line, must appear
//! before any other code):
//!
//! ```text
//! // {{INCLUDE_FRAME}}
//! // {{INCLUDE_WBOIT}}
//! // {{INCLUDE_LIGHTING}}
//! // {{INCLUDE_SCENE}}
//! // {{INCLUDE_PICKING}}
//! ```
//!
//! `expand` substitutes the directive line with the corresponding source.
//!
//! `INCLUDE_LIGHTING` requires `INCLUDE_FRAME` to come first — the
//! lighting helpers reference `frame.light_intensity` etc.

const FRAME_WGSL: &str = include_str!("shaders/common/frame.wgsl");
const WBOIT_WGSL: &str = include_str!("shaders/common/wboit.wgsl");
const LIGHTING_WGSL: &str = include_str!("shaders/common/lighting.wgsl");
const SCENE_WGSL: &str = include_str!("shaders/common/scene.wgsl");
const PICKING_WGSL: &str = include_str!("shaders/picking/common.wgsl");
const OCTAHEDRAL_WGSL: &str = include_str!("shaders/common/octahedral.wgsl");
const CULL_COMMON_WGSL: &str = include_str!("shaders/compute/cull_common.wgsl");

pub const SPHERE_WGSL: &str = include_str!("shaders/representations/sphere.wgsl");
pub const SPHERE_PICKING_WGSL: &str = include_str!("shaders/picking/sphere.wgsl");
pub const STICK_WGSL: &str = include_str!("shaders/representations/stick.wgsl");
pub const STICK_PICKING_WGSL: &str = include_str!("shaders/picking/stick.wgsl");
pub const LINE_WGSL: &str = include_str!("shaders/representations/line.wgsl");
pub const LINE_PICKING_WGSL: &str = include_str!("shaders/picking/line.wgsl");
pub const DOT_WGSL: &str = include_str!("shaders/representations/dot.wgsl");
pub const DOT_PICKING_WGSL: &str = include_str!("shaders/picking/dot.wgsl");
pub const MESH_WGSL: &str = include_str!("shaders/representations/mesh.wgsl");
pub const MAP_WGSL: &str = include_str!("shaders/representations/map.wgsl");
pub const STROKE_WGSL: &str = include_str!("shaders/representations/stroke.wgsl");
pub const STD_VERTEX_PICKING_WGSL: &str = include_str!("shaders/picking/std_vertex.wgsl");
pub const CARTOON_WGSL: &str = include_str!("shaders/representations/cartoon.wgsl");
pub const SURFACE_WGSL: &str = include_str!("shaders/representations/surface.wgsl");
pub const ELLIPSOID_WGSL: &str = include_str!("shaders/representations/ellipsoid.wgsl");
pub const ELLIPSOID_PICKING_WGSL: &str = include_str!("shaders/picking/ellipsoid.wgsl");
pub const WBOIT_COMPOSITE_WGSL: &str = include_str!("shaders/postprocess/wboit_composite.wgsl");
pub const SILHOUETTE_WGSL: &str = include_str!("shaders/postprocess/silhouette.wgsl");

// Run-aware extrusion kernel. Reads `ExtrudePoint[]` + `RunDescriptor[]`
// produced by `tessellation`, branches on `RunDescriptor.car_type`, emits
// `StdVertex` triangles per the per-CartoonType emission rules.
pub const CARTOON_EXTRUDE_WGSL: &str = include_str!("shaders/compute/cartoon_extrude.wgsl");

pub const BUILD_SPHERE_WGSL: &str = include_str!("shaders/compute/build_sphere.wgsl");
pub const SPHERE_LOD_COUNT_WGSL: &str = include_str!("shaders/compute/sphere_lod_count.wgsl");
pub const BUILD_STICK_WGSL: &str = include_str!("shaders/compute/build_stick.wgsl");
pub const STICK_LOD_COUNT_WGSL: &str = include_str!("shaders/compute/stick_lod_count.wgsl");
pub const BUILD_LINE_WGSL: &str = include_str!("shaders/compute/build_line.wgsl");
pub const BUILD_DOT_WGSL: &str = include_str!("shaders/compute/build_dot.wgsl");
pub const BUILD_ELLIPSOID_WGSL: &str = include_str!("shaders/compute/build_ellipsoid.wgsl");

pub const SURFACE_DENSITY_WGSL: &str = include_str!("shaders/compute/surface_density.wgsl");
pub const SURFACE_VDW_SDF_WGSL: &str = include_str!("shaders/compute/surface_vdw_sdf.wgsl");
pub const SURFACE_SES_MORPH_WGSL: &str = include_str!("shaders/compute/surface_ses_morph.wgsl");
pub const SURFACE_MC_WGSL: &str = include_str!("shaders/compute/surface_mc.wgsl");
pub const PICKING_REPROJECT_WGSL: &str = include_str!("shaders/compute/picking_reproject.wgsl");
pub const RAY_BUFFER_TO_TEXTURE_WGSL: &str =
    include_str!("shaders/compute/ray_buffer_to_texture.wgsl");
pub const SSAO_WGSL: &str = include_str!("shaders/compute/ssao.wgsl");
pub const SSAO_BLUR_WGSL: &str = include_str!("shaders/compute/ssao_blur.wgsl");
pub const SSAO_COMPOSE_WGSL: &str = include_str!("shaders/postprocess/ssao_compose.wgsl");
pub const FXAA_WGSL: &str = include_str!("shaders/postprocess/fxaa.wgsl");
pub const MARKING_MASK_WGSL: &str = include_str!("shaders/postprocess/marking.wgsl");
pub const MARKING_COMPOSITE_WGSL: &str = include_str!("shaders/postprocess/marking_composite.wgsl");
pub const SELECTION_DOTS_WGSL: &str = include_str!("shaders/postprocess/selection_dots.wgsl");
pub const CULL_INSTANCES_WGSL: &str = include_str!("shaders/compute/cull_instances.wgsl");
pub const CULL_STICK_WGSL: &str = include_str!("shaders/compute/cull_stick.wgsl");
pub const CULL_LINE_WGSL: &str = include_str!("shaders/compute/cull_line.wgsl");
pub const CULL_DOT_WGSL: &str = include_str!("shaders/compute/cull_dot.wgsl");
pub const CULL_ELLIPSOID_WGSL: &str = include_str!("shaders/compute/cull_ellipsoid.wgsl");

/// Expand `{{INCLUDE_*}}` directives. Unknown directives pass through
/// unchanged so future additions don't silently break.
pub fn expand(src: &str) -> String {
    let cap =
        src.len() + FRAME_WGSL.len() + WBOIT_WGSL.len() + LIGHTING_WGSL.len() + SCENE_WGSL.len();
    let mut out = String::with_capacity(cap);
    for line in src.lines() {
        let trimmed = line.trim();
        match trimmed {
            "// {{INCLUDE_FRAME}}" => {
                out.push_str(FRAME_WGSL);
                out.push('\n');
            }
            "// {{INCLUDE_WBOIT}}" => {
                out.push_str(WBOIT_WGSL);
                out.push('\n');
            }
            "// {{INCLUDE_LIGHTING}}" => {
                out.push_str(LIGHTING_WGSL);
                out.push('\n');
            }
            "// {{INCLUDE_SCENE}}" => {
                out.push_str(SCENE_WGSL);
                out.push('\n');
            }
            "// {{INCLUDE_PICKING}}" => {
                out.push_str(PICKING_WGSL);
                out.push('\n');
            }
            "// {{INCLUDE_OCTAHEDRAL}}" => {
                out.push_str(OCTAHEDRAL_WGSL);
                out.push('\n');
            }
            "// {{INCLUDE_CULL_COMMON}}" => {
                out.push_str(CULL_COMMON_WGSL);
                out.push('\n');
            }
            _ => {
                out.push_str(line);
                out.push('\n');
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expanded_wgsl_modules_parse_and_validate() {
        let modules = [
            ("patinae.sphere", SPHERE_WGSL),
            ("patinae.sphere_picking", SPHERE_PICKING_WGSL),
            ("patinae.sphere_lod_count", SPHERE_LOD_COUNT_WGSL),
            ("patinae.stick", STICK_WGSL),
            ("patinae.stick_picking", STICK_PICKING_WGSL),
            ("patinae.line", LINE_WGSL),
            ("patinae.line_picking", LINE_PICKING_WGSL),
            ("patinae.dot", DOT_WGSL),
            ("patinae.dot_picking", DOT_PICKING_WGSL),
            ("patinae.mesh", MESH_WGSL),
            ("patinae.map", MAP_WGSL),
            ("patinae.stroke", STROKE_WGSL),
            ("patinae.std_vertex_picking", STD_VERTEX_PICKING_WGSL),
            ("patinae.cartoon", CARTOON_WGSL),
            ("patinae.surface", SURFACE_WGSL),
            ("patinae.ellipsoid", ELLIPSOID_WGSL),
            ("patinae.ellipsoid_picking", ELLIPSOID_PICKING_WGSL),
            ("patinae.wboit_composite", WBOIT_COMPOSITE_WGSL),
            ("patinae.silhouette", SILHOUETTE_WGSL),
            ("patinae.cartoon_extrude", CARTOON_EXTRUDE_WGSL),
            ("patinae.build_sphere", BUILD_SPHERE_WGSL),
            ("patinae.build_stick", BUILD_STICK_WGSL),
            ("patinae.stick_lod_count", STICK_LOD_COUNT_WGSL),
            ("patinae.build_line", BUILD_LINE_WGSL),
            ("patinae.build_dot", BUILD_DOT_WGSL),
            ("patinae.build_ellipsoid", BUILD_ELLIPSOID_WGSL),
            ("patinae.surface_density", SURFACE_DENSITY_WGSL),
            ("patinae.surface_vdw_sdf", SURFACE_VDW_SDF_WGSL),
            ("patinae.surface_ses_morph", SURFACE_SES_MORPH_WGSL),
            ("patinae.surface_mc", SURFACE_MC_WGSL),
            ("patinae.picking_reproject", PICKING_REPROJECT_WGSL),
            ("patinae.ray_buffer_to_texture", RAY_BUFFER_TO_TEXTURE_WGSL),
            ("patinae.ssao", SSAO_WGSL),
            ("patinae.ssao_blur", SSAO_BLUR_WGSL),
            ("patinae.ssao_compose", SSAO_COMPOSE_WGSL),
            ("patinae.fxaa", FXAA_WGSL),
            ("patinae.marking_mask", MARKING_MASK_WGSL),
            ("patinae.marking_composite", MARKING_COMPOSITE_WGSL),
            ("patinae.selection_dots", SELECTION_DOTS_WGSL),
            ("patinae.cull_instances", CULL_INSTANCES_WGSL),
            ("patinae.cull_stick", CULL_STICK_WGSL),
            ("patinae.cull_line", CULL_LINE_WGSL),
            ("patinae.cull_dot", CULL_DOT_WGSL),
            ("patinae.cull_ellipsoid", CULL_ELLIPSOID_WGSL),
        ];

        let mut failures = Vec::new();

        for (label, source) in modules {
            let expanded = expand(source);
            assert!(
                !expanded.contains("textureSampleCompare("),
                "{label}: use textureSampleCompareLevel in shared shaders; \
                 Chrome/Dawn rejects textureSampleCompare from non-uniform fragment control flow"
            );
            let module = match naga::front::wgsl::parse_str(&expanded) {
                Ok(module) => module,
                Err(err) => {
                    failures.push(format!(
                        "{label}: WGSL parse failed\n{}",
                        err.emit_to_string_with_path(&expanded, label)
                    ));
                    continue;
                }
            };

            let mut validator = naga::valid::Validator::new(
                naga::valid::ValidationFlags::all(),
                naga::valid::Capabilities::all(),
            );
            if let Err(err) = validator.validate(&module) {
                failures.push(format!("{label}: WGSL validation failed\n{err:#?}"));
            }
        }

        assert!(
            failures.is_empty(),
            "invalid expanded WGSL modules:\n{}",
            failures.join("\n\n")
        );
    }
}
