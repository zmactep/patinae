// Compact selected-atom dots and recent-atom sphere impostors.

// {{INCLUDE_FRAME}}
// {{INCLUDE_SCENE}}

struct AtomMarkerParams {
    radius_px: f32,
    view_bias: f32,
    recent_shell: f32,
    _pad0: f32,
};

@group(3) @binding(0) var<uniform> atom_markers: AtomMarkerParams;

struct MarkerInstance {
    atom_index: u32,
    kind: u32,
    radius_scale: f32,
    _pad0: f32,
};

@group(3) @binding(1) var<storage, read> marker_instances: array<MarkerInstance>;

const MARKER_SELECTED: u32 = 1u << 0u;
const MARKER_RECENT: u32 = 1u << 2u;
const MARKER_KIND_SELECTED: u32 = 0u;
const MARKER_KIND_RECENT: u32 = 1u;
const REP_SPHERES: u32 = 1u << 1u;
const SELECTED_FILL: vec4<f32> = vec4<f32>(1.0, 0.0, 0.7843, 0.82);
const SELECTED_RIM: vec4<f32> = vec4<f32>(1.0, 0.82, 1.0, 0.96);
const RECENT_COLOR: vec3<f32> = vec3<f32>(0.02, 0.82, 1.0);
// Compact enough for stick and line modes while remaining atom-sized.
const RECENT_ATOM_RADIUS_SCALE: f32 = 0.7;

struct VsOut {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) @interpolate(flat) enabled: u32,
    @location(2) center_view: vec3<f32>,
    @location(3) ray_origin: vec3<f32>,
    @location(4) ray_dir: vec3<f32>,
    @location(5) radius: f32,
    @location(6) @interpolate(flat) kind: u32,
};

fn quad_corner(vertex_index: u32) -> vec2<f32> {
    let v = vertex_index % 6u;
    if v == 0u { return vec2<f32>(-1.0, -1.0); }
    if v == 1u { return vec2<f32>( 1.0, -1.0); }
    if v == 2u { return vec2<f32>( 1.0,  1.0); }
    if v == 3u { return vec2<f32>(-1.0, -1.0); }
    if v == 4u { return vec2<f32>( 1.0,  1.0); }
    return vec2<f32>(-1.0, 1.0);
}

fn inactive_out() -> VsOut {
    var out: VsOut;
    out.clip_position = vec4<f32>(2.0, 2.0, 1.0, 1.0);
    out.uv = vec2<f32>(2.0, 2.0);
    out.enabled = 0u;
    out.center_view = vec3<f32>(0.0);
    out.ray_origin = vec3<f32>(0.0);
    out.ray_dir = vec3<f32>(0.0);
    out.radius = 0.0;
    out.kind = MARKER_KIND_SELECTED;
    return out;
}

@vertex
fn vs_main(
    @builtin(vertex_index) vertex_index: u32,
    @builtin(instance_index) instance_index: u32,
) -> VsOut {
    if instance_index >= arrayLength(&marker_instances) {
        return inactive_out();
    }

    let marker_instance = marker_instances[instance_index];
    let local_id = marker_instance.atom_index;
    if local_id >= obj.atom_count {
        return inactive_out();
    }

    let global_id = obj.atom_offset + local_id;
    if !scene_visible(global_id) {
        return inactive_out();
    }
    let required_marker = select(MARKER_SELECTED, MARKER_RECENT, marker_instance.kind == MARKER_KIND_RECENT);
    if (scene_marker(global_id) & required_marker) == 0u {
        return inactive_out();
    }

    var view_pos = (frame.view * vec4<f32>(scene_coord(global_id), 1.0)).xyz;
    let corner = quad_corner(vertex_index);
    var clip: vec4<f32>;
    var ray_origin = vec3<f32>(0.0);
    var ray_dir = vec3<f32>(0.0);
    var radius = 0.0;
    if marker_instance.kind == MARKER_KIND_RECENT {
        let sphere_visible = (scene_atom(global_id).repr_flags & REP_SPHERES) != 0u;
        let radius_scale = select(RECENT_ATOM_RADIUS_SCALE, marker_instance.radius_scale, sphere_visible);
        radius = scene_atom(global_id).vdw * radius_scale + atom_markers.recent_shell;
        let billboard_pos = view_pos + vec3<f32>(corner * radius * 1.5, 0.0);
        clip = frame.proj * vec4<f32>(billboard_pos, 1.0);
        let ndc_xy = clip.xy / clip.w;
        let near_h = frame.proj_inv * vec4<f32>(ndc_xy, 0.0, 1.0);
        let far_h = frame.proj_inv * vec4<f32>(ndc_xy, 1.0, 1.0);
        ray_origin = near_h.xyz / near_h.w;
        ray_dir = far_h.xyz / far_h.w - ray_origin;
    } else {
        view_pos.z = view_pos.z + atom_markers.view_bias;
        clip = frame.proj * vec4<f32>(view_pos, 1.0);
        let pixel_to_clip = vec2<f32>(frame.viewport.z * 2.0, frame.viewport.w * 2.0);
        let offset = corner * atom_markers.radius_px * pixel_to_clip * clip.w;
        clip = vec4<f32>(clip.x + offset.x, clip.y + offset.y, clip.z, clip.w);
    }

    var out: VsOut;
    out.clip_position = clip;
    out.uv = corner;
    out.enabled = 1u;
    out.center_view = view_pos;
    out.ray_origin = ray_origin;
    out.ray_dir = ray_dir;
    out.radius = radius;
    out.kind = marker_instance.kind;
    return out;
}

struct MarkerOut {
    @builtin(frag_depth) depth: f32,
    @location(0) color: vec4<f32>,
};

@fragment
fn fs_main(input: VsOut) -> MarkerOut {
    if input.enabled == 0u {
        discard;
    }
    var out: MarkerOut;
    let d2 = dot(input.uv, input.uv);
    if d2 > 1.0 {
        discard;
    }
    if input.kind == MARKER_KIND_RECENT {
        let ray = normalize(input.ray_dir);
        let oc = input.ray_origin - input.center_view;
        let b = 2.0 * dot(oc, ray);
        let c = dot(oc, oc) - input.radius * input.radius;
        let discriminant = b * b - 4.0 * c;
        if discriminant < 0.0 {
            discard;
        }
        let root = sqrt(discriminant);
        var t = (-b - root) * 0.5;
        if t < 0.0 {
            t = (-b + root) * 0.5;
        }
        if t < 0.0 {
            discard;
        }
        let hit = input.ray_origin + ray * t;
        let normal = normalize(hit - input.center_view);
        let clip = frame.proj * vec4<f32>(hit, 1.0);
        out.depth = clip.z / clip.w;

        let light = 0.62 + 0.38 * max(dot(normal, normalize(vec3<f32>(-0.35, 0.45, 1.0))), 0.0);
        let rim = pow(1.0 - max(normal.z, 0.0), 2.2);
        let alpha = mix(0.14, 0.54, rim);
        out.color = vec4<f32>(RECENT_COLOR * light, alpha);
        return out;
    }

    let rim = smoothstep(0.58, 0.88, d2);
    out.depth = input.clip_position.z;
    out.color = mix(SELECTED_FILL, SELECTED_RIM, rim);
    return out;
}
