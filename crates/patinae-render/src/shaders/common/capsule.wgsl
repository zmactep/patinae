// Shared by stick color, depth, shadow, and picking passes.
// Requires INCLUDE_FRAME. Ray origins and normals stay near the capsule;
// no quadratic discriminant contains the squared camera distance.

fn capsule_perspective() -> bool {
    return frame.proj[3][3] == 0.0;
}

fn capsule_quad_uv(vid: u32) -> vec2<f32> {
    let id = vid % 6u;
    return vec2<f32>(select(-1.0, 1.0, id == 1u || id == 4u || id == 5u),
                     select(-1.0, 1.0, id == 2u || id == 3u || id == 5u));
}

// Conservative NDC bounds for an endpoint sphere. In perspective, projecting
// its view-aligned enclosing box is safe as long as it does not cross the eye.
fn capsule_endpoint_bounds(center: vec3<f32>, radius: f32) -> vec4<f32> {
    if !capsule_perspective() {
        let clip = frame.proj * vec4<f32>(center, 1.0);
        let pad = radius * abs(vec2<f32>(frame.proj[0][0], frame.proj[1][1]));
        return vec4<f32>(clip.xy - pad, clip.xy + pad);
    }
    var lo = vec2<f32>(1e30);
    var hi = vec2<f32>(-1e30);
    for (var corner = 0u; corner < 8u; corner++) {
        let offset = vec3<f32>(select(-radius, radius, (corner & 1u) != 0u),
                               select(-radius, radius, (corner & 2u) != 0u),
                               select(-radius, radius, (corner & 4u) != 0u));
        let clip = frame.proj * vec4<f32>(center + offset, 1.0);
        let ndc = clip.xy / clip.w;
        lo = min(lo, ndc);
        hi = max(hi, ndc);
    }
    return vec4<f32>(lo, hi);
}

fn capsule_billboard(uv: vec2<f32>, pa: vec3<f32>, pb: vec3<f32>, radius: f32) -> vec4<f32> {
    if capsule_perspective() && max(pa.z, pb.z) + radius >= 0.0 {
        // Crossing the eye plane has an unbounded projection. Fragment hits
        // still enforce the actual near/far planes and forward ray interval.
        return vec4<f32>(uv, 0.0, 1.0);
    }
    let ca = frame.proj * vec4<f32>(pa, 1.0);
    let cb = frame.proj * vec4<f32>(pb, 1.0);
    let delta = cb.xy / cb.w - ca.xy / ca.w;
    var axis = vec2<f32>(1.0, 0.0);
    if dot(delta, delta) > 1e-12 { axis = normalize(delta); }
    let side = vec2<f32>(-axis.y, axis.x);
    var lo = vec2<f32>(1e30);
    var hi = vec2<f32>(-1e30);
    for (var end = 0u; end < 2u; end++) {
        let bounds = capsule_endpoint_bounds(select(pa, pb, end == 1u), radius);
        let center = 0.5 * (bounds.xy + bounds.zw);
        let half_size = 0.5 * (bounds.zw - bounds.xy);
        let along = vec2<f32>(dot(center, axis), dot(center, side));
        let pad = vec2<f32>(dot(half_size, abs(axis)), dot(half_size, abs(side)));
        lo = min(lo, along - pad);
        hi = max(hi, along + pad);
    }
    // The oriented rectangle encloses both projected endpoint bounds and their
    // convex hull. Keeping it aligned to the bond avoids quadratic overdraw
    // for long diagonal bonds while retaining conservative perspective bounds.
    let point = mix(lo, hi, uv * 0.5 + 0.5);
    let xy = axis * point.x + side * point.y;
    // Rasterize at the near plane; clipping the proxy at mid.z would drop
    // valid surface intersections when only part of the capsule is in view.
    return vec4<f32>(xy, 0.0, 1.0);
}

struct CapsuleRay {
    origin: vec3<f32>,
    direction: vec3<f32>,
    min_t: f32,
};

fn capsule_ray(ndc: vec2<f32>, center: vec3<f32>) -> CapsuleRay {
    let near_h = frame.proj_inv * vec4<f32>(ndc, 0.0, 1.0);
    let near = near_h.xyz / near_h.w;
    var ray: CapsuleRay;
    ray.direction = vec3<f32>(0.0, 0.0, -1.0);
    var plane_xy = near.xy;
    if capsule_perspective() {
        ray.direction = normalize(near);
        plane_xy = near.xy * (center.z / near.z);
    }
    // Intersect the original ray with the capsule's center-z plane directly.
    // In particular, do not compute ro + rd * camera_distance and then
    // subtract the center: that reintroduces cancellation in the z component.
    ray.origin = vec3<f32>(plane_xy - center.xy, 0.0);
    // Retain the nearest root in the view frustum, including an exit root
    // when the near plane cuts through the capsule.
    ray.min_t = (near.z - center.z) / ray.direction.z;
    return ray;
}

struct CapsuleHit {
    t: f32,
    point: vec3<f32>,
    normal: vec3<f32>,
    ok: bool,
};

fn capsule_intersection(ray: CapsuleRay, pa: vec3<f32>, pb: vec3<f32>, radius: f32) -> CapsuleHit {
    var hit: CapsuleHit;
    hit.t = 1e30;
    hit.ok = false;
    let ba = pb - pa;
    let axis_length = length(ba);
    var axis = vec3<f32>(0.0, 0.0, 1.0);
    // A sub-microangstrom bond is treated as a sphere, avoiding division by 0.
    let has_axis = axis_length > 1e-6;
    if has_axis { axis = ba / axis_length; }
    let oa = ray.origin - pa;
    let axial_origin = dot(oa, axis);
    let axial_direction = dot(ray.direction, axis);
    let op = oa - axis * axial_origin;
    let dp = ray.direction - axis * axial_direction;
    let a = dot(dp, dp);
    // The exactly parallel case has no cylinder-wall crossing; caps handle it.
    if has_axis && a > 1e-12 {
        let closest_t = -dot(op, dp) / a;
        let closest = op + dp * closest_t;
        let h = radius * radius - dot(closest, closest);
        if h >= 0.0 {
            let dt = sqrt(h / a);
            for (var side = 0u; side < 2u; side++) {
                let t = closest_t + select(-dt, dt, side == 1u);
                let y = axial_origin + t * axial_direction;
                if t >= ray.min_t && t < hit.t && y >= 0.0 && y <= axis_length {
                    hit.t = t;
                    hit.point = ray.origin + ray.direction * t;
                    hit.normal = normalize(op + dp * t);
                    hit.ok = true;
                }
            }
        }
    }
    for (var end = 0u; end < 2u; end++) {
        let center = select(pa, pb, end == 1u);
        let oc = ray.origin - center;
        let closest_t = -dot(ray.direction, oc);
        let closest = oc + ray.direction * closest_t;
        let h = radius * radius - dot(closest, closest);
        if h < 0.0 { continue; }
        let dt = sqrt(h);
        for (var side = 0u; side < 2u; side++) {
            let t = closest_t + select(-dt, dt, side == 1u);
            let point = ray.origin + ray.direction * t;
            let y = dot(point - pa, axis);
            let on_cap = !has_axis || select(y <= 0.0, y >= axis_length, end == 1u);
            if on_cap && t >= ray.min_t && t < hit.t {
                hit.t = t;
                hit.point = point;
                hit.normal = normalize(point - center);
                hit.ok = true;
            }
        }
    }
    return hit;
}

fn capsule_hit(ndc: vec2<f32>, pa: vec3<f32>, pb: vec3<f32>, radius: f32) -> CapsuleHit {
    let center = 0.5 * (pa + pb);
    let half_axis = 0.5 * (pb - pa);
    let ray = capsule_ray(ndc, center);
    var hit = capsule_intersection(ray, -half_axis, half_axis, radius);
    hit.point += center;
    return hit;
}

fn capsule_view_direction(point: vec3<f32>) -> vec3<f32> {
    return select(vec3<f32>(0.0, 0.0, 1.0), -point, capsule_perspective());
}
