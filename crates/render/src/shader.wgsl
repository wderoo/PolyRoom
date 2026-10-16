struct Uniforms {
    view_proj: mat4x4<f32>,
    eye: vec4<f32>,
    right: vec4<f32>,
    up: vec4<f32>,
    base_color: vec4<f32>,
    line_color: vec4<f32>,
};

@group(0) @binding(0) var<uniform> u: Uniforms;

struct VOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world: vec3<f32>,
};

@vertex
fn vs_main(@location(0) pos: vec3<f32>) -> VOut {
    var o: VOut;
    o.clip = u.view_proj * vec4<f32>(pos, 1.0);
    o.world = pos;
    return o;
}

// Flat shading from screen-space derivatives: no per-vertex normals needed, so meshes stay
// compact and faceted STL surfaces look the way slicers show them.
@fragment
fn fs_mesh(in: VOut) -> @location(0) vec4<f32> {
    let v = normalize(u.eye.xyz - in.world);
    let c = cross(dpdx(in.world), dpdy(in.world));
    var n = v;
    if (dot(c, c) > 1e-20) {
        n = normalize(c);
    }
    // Face the viewer, so inverted or open meshes still shade sensibly.
    if (dot(n, v) < 0.0) {
        n = -n;
    }

    // Camera-relative lights so the model stays lit while orbiting.
    let key = normalize(v * 0.5 + u.up.xyz * 0.7 + u.right.xyz * 0.5);
    let fill = normalize(v * 0.6 - u.right.xyz * 0.8 - u.up.xyz * 0.2);
    let diffuse = max(dot(n, key), 0.0) * 0.75 + max(dot(n, fill), 0.0) * 0.22;
    let sky = mix(vec3<f32>(0.20, 0.19, 0.18), vec3<f32>(0.42, 0.44, 0.48), n.z * 0.5 + 0.5);
    let h = normalize(key + v);
    let spec = pow(max(dot(n, h), 0.0), 48.0) * 0.22;
    let rim = pow(1.0 - max(dot(n, v), 0.0), 3.0) * 0.10;

    let color = u.base_color.rgb * (diffuse + sky * 0.55) + vec3<f32>(spec + rim);
    return vec4<f32>(gamma_from_linear(color), 1.0);
}

// Lighting happens in linear space; the target stores sRGB-encoded bytes.
fn gamma_from_linear(rgb: vec3<f32>) -> vec3<f32> {
    let c = clamp(rgb, vec3<f32>(0.0), vec3<f32>(1.0));
    let lower = c * 12.92;
    let higher = 1.055 * pow(c, vec3<f32>(1.0 / 2.4)) - 0.055;
    return select(higher, lower, c < vec3<f32>(0.0031308));
}

// The line colour is already sRGB-encoded.
@fragment
fn fs_line(in: VOut) -> @location(0) vec4<f32> {
    return u.line_color;
}
