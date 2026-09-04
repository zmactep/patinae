// Multi-light Phong shading, fog, depth-cue, directional depth occlusion,
// and multi-directional atlas ambient occlusion.
// Reads `frame.*` from group 0 and occlusion resources from group 1. Every
// consumer must include `// {{INCLUDE_FRAME}}` before `// {{INCLUDE_LIGHTING}}`.
//
// The model:
//   - Headlight (camera-attached) controlled by `direct` / `spec_direct`.
//   - First `light_count - 1` positional lights from `frame.light_dirs[]`,
//     diffuse weighted by `reflect`, specular weighted by `specular`.
//   - `spec_count` clamps how many positional lights contribute specular.
//   - Ambient applied unconditionally as `base * ambient`.
//
// All lighting vectors live in view space: camera at origin, fragments at
// `view_pos`, view_dir = -view_pos. Shadow lookup uses world-space positions.

// Keep in sync with passes::lighting::MAX_ATLAS_DIRECTIONS.
const MAX_ATLAS_DIRECTIONS: u32 = 254u;

struct LightingOcclusionUniforms {
    view_proj: mat4x4<f32>,
    params: vec4<f32>,
    atlas: vec4<u32>,
    matrices: array<mat4x4<f32>, MAX_ATLAS_DIRECTIONS>,
};

@group(1) @binding(0) var occlusion_depth: texture_depth_2d;
@group(1) @binding(1) var occlusion_sampler: sampler_comparison;
@group(1) @binding(2) var<uniform> occlusion: LightingOcclusionUniforms;

const OCCLUSION_MODE_DISABLED: u32 = 0u;
const OCCLUSION_MODE_ATLAS_AO: u32 = 1u;
const OCCLUSION_MODE_DIRECTIONAL: u32 = 2u;

fn classic_lighting(normal: vec3<f32>, view_dir: vec3<f32>, base_color: vec3<f32>) -> vec3<f32> {
    let n = normalize(normal);
    let v = normalize(view_dir);

    let ambient = frame.light_intensity.x;
    let direct = frame.light_intensity.y;
    let reflect = frame.light_intensity.z;
    let specular = frame.light_intensity.w;

    let shininess = frame.light_spec.x;
    let spec_direct = frame.light_spec.y;
    let spec_direct_power = frame.light_spec.z;

    let light_count = i32(frame.light_counts.x);
    let spec_count = i32(frame.light_counts.y);

    var color = base_color * ambient;

    // Headlight: view_dir already points from fragment to camera.
    let headlight_ndotl = max(dot(n, v), 0.0);
    color = color + base_color * headlight_ndotl * direct;
    if spec_direct > 0.0 && headlight_ndotl > 0.0 {
        let spec = pow(headlight_ndotl, spec_direct_power);
        color = color + vec3<f32>(1.0) * spec * spec_direct;
    }

    let num_pos_lights = max(light_count - 1, 0);
    let effective_spec_count =
        select(spec_count, num_pos_lights, spec_count < 0);

    for (var i: i32 = 0; i < num_pos_lights; i = i + 1) {
        // Host light vectors point toward the light; negate to get the
        // fragment-to-light shading vector.
        let l = normalize(-frame.light_dirs[i].xyz);
        let ndotl = max(dot(n, l), 0.0);
        color = color + base_color * ndotl * reflect;

        if specular > 0.0 && ndotl > 0.0 && i < effective_spec_count {
            let h = normalize(l + v);
            let ndoth = max(dot(n, h), 0.0);
            let spec = pow(ndoth, shininess);
            color = color + vec3<f32>(1.0) * spec * specular;
        }
    }

    return color;
}

fn shadow_visibility(world_pos: vec3<f32>) -> f32 {
    let intensity = occlusion.params.y;
    if intensity <= 0.0 || occlusion.atlas.z != OCCLUSION_MODE_DIRECTIONAL {
        return 1.0;
    }

    let clip = occlusion.view_proj * vec4<f32>(world_pos, 1.0);
    if clip.w <= 0.0 {
        return 1.0;
    }
    let ndc = clip.xyz / clip.w;
    if ndc.z < 0.0 || ndc.z > 1.0 {
        return 1.0;
    }

    let uv = vec2<f32>(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5);
    if uv.x < 0.0 || uv.x > 1.0 || uv.y < 0.0 || uv.y > 1.0 {
        return 1.0;
    }

    let radius = i32(clamp(ceil(occlusion.params.w), 0.0, 4.0));
    let texel = max(occlusion.params.z, 1e-6);
    let reference_depth = ndc.z - occlusion.params.x;
    var sum = 0.0;
    var count = 0.0;
    for (var y: i32 = -4; y <= 4; y = y + 1) {
        if abs(y) > radius {
            continue;
        }
        for (var x: i32 = -4; x <= 4; x = x + 1) {
            if abs(x) > radius {
                continue;
            }
            let offset = vec2<f32>(f32(x), f32(y)) * texel;
            sum = sum + textureSampleCompareLevel(
                occlusion_depth,
                occlusion_sampler,
                uv + offset,
                reference_depth,
            );
            count = count + 1.0;
        }
    }
    let lit = sum / max(count, 1.0);
    return mix(1.0 - intensity, 1.0, lit);
}

fn apply_shadow(color: vec3<f32>, world_pos: vec3<f32>) -> vec3<f32> {
    if occlusion.atlas.z == OCCLUSION_MODE_ATLAS_AO {
        return color * atlas_ambient_occlusion(world_pos);
    }
    return color * shadow_visibility(world_pos);
}

fn atlas_tile_depth(world_pos: vec3<f32>, index: u32) -> vec2<f32> {
    let shadow_pos = occlusion.matrices[index] * vec4<f32>(world_pos, 1.0);
    let ndc = shadow_pos.xyz / shadow_pos.w;
    if ndc.x < -1.0 || ndc.x > 1.0 || ndc.y < -1.0 || ndc.y > 1.0 ||
        ndc.z < 0.0 || ndc.z > 1.0 {
        return vec2<f32>(1.0, 1.0);
    }

    let grid = max(occlusion.atlas.y, 1u);
    let tile_size = max(occlusion.atlas.w, 1u);
    let col = index % grid;
    let row = index / grid;
    let uv = vec2<f32>(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5);
    let local = vec2<u32>(
        clamp(u32(uv.x * f32(tile_size)), 0u, tile_size - 1u),
        clamp(u32(uv.y * f32(tile_size)), 0u, tile_size - 1u),
    );
    let coord = vec2<i32>(
        i32(col * tile_size + local.x),
        i32(row * tile_size + local.y),
    );
    return vec2<f32>(textureLoad(occlusion_depth, coord, 0), ndc.z);
}

fn atlas_ambient_occlusion(world_pos: vec3<f32>) -> f32 {
    let count = min(occlusion.atlas.x, MAX_ATLAS_DIRECTIONS);
    let intensity = occlusion.params.y;
    if count == 0u || intensity <= 0.0 {
        return 1.0;
    }

    var lit_count = 0.0;
    for (var i = 0u; i < count; i = i + 1u) {
        let depths = atlas_tile_depth(world_pos, i);
        let occluder_depth = depths.x;
        let fragment_depth = depths.y;
        if fragment_depth <= occluder_depth + occlusion.params.x {
            lit_count = lit_count + 1.0;
        }
    }
    let fraction_lit = lit_count / f32(count);
    return clamp(mix(1.0, fraction_lit, intensity), 0.0, 1.0);
}

// Linear fog blend. `depth` is view-space distance to the fragment.
fn apply_fog(color: vec3<f32>, depth: f32) -> vec3<f32> {
    let density = frame.fog.z;
    if density <= 0.0 {
        return color;
    }
    let start = frame.fog.x;
    let end = frame.fog.y;
    if end <= start {
        return color;
    }
    let fog_factor = clamp((end - depth) / (end - start), 0.0, 1.0);
    return mix(frame.fog_color.rgb, color, fog_factor);
}

// Depth cue scales colour by `1 - factor * normalized_depth * 0.5`.
fn apply_depth_cue(color: vec3<f32>, depth: f32) -> vec3<f32> {
    let factor = frame.light_spec.w;
    if factor <= 0.0 {
        return color;
    }
    let near = frame.clip.x;
    let far = frame.clip.y;
    if far <= near {
        return color;
    }
    let normalized_depth = clamp((depth - near) / (far - near), 0.0, 1.0);
    let cue = 1.0 - factor * normalized_depth * 0.5;
    return color * cue;
}
