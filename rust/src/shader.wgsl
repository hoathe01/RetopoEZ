struct U {
    vp: mat4x4<f32>,
    view: mat4x4<f32>,
    low: vec4<f32>,  // low-poly fill rgba
    eye: vec4<f32>,  // xyz camera, w = distance the low-poly floats toward the camera
    res: vec4<f32>,  // viewport size in pixels
};
@group(0) @binding(0) var<uniform> u: U;

struct VIn { @location(0) pos: vec3<f32>, @location(1) nrm: vec3<f32> };
struct VOut { @builtin(position) clip: vec4<f32>, @location(0) n: vec3<f32> };

// Low-poly floats toward the camera along the view ray: same pixels, nearer depth, so flat
// faces that sag below the curved high mesh are not swallowed by it.
fn lift(p: vec3<f32>) -> vec3<f32> {
    return p + normalize(u.eye.xyz - p) * u.eye.w;
}

@vertex fn vs(v: VIn) -> VOut {
    var o: VOut;
    o.clip = u.vp * vec4<f32>(v.pos, 1.0);
    o.n = (u.view * vec4<f32>(v.nrm, 0.0)).xyz;
    return o;
}
@vertex fn vs_low(v: VIn) -> VOut {
    var o: VOut;
    o.clip = u.vp * vec4<f32>(lift(v.pos), 1.0);
    o.n = (u.view * vec4<f32>(v.nrm, 0.0)).xyz;
    return o;
}

// Lines: one instance per segment, expanded to a screen-space quad of constant pixel width.
struct SIn {
    @builtin(vertex_index) vi: u32,
    @location(0) a: vec3<f32>,
    @location(1) b: vec3<f32>,
    @location(2) col: vec3<f32>,
    @location(3) w: f32,
};
fn seg_clip(i: SIn, lifted: bool) -> VOut {
    var o: VOut;
    o.n = i.col;
    var pa = i.a;
    var pb = i.b;
    if (lifted) {
        pa = lift(pa);
        pb = lift(pb);
    }
    let ca = u.vp * vec4<f32>(pa, 1.0);
    let cb = u.vp * vec4<f32>(pb, 1.0);
    if (ca.w < 1e-5 || cb.w < 1e-5) {
        o.clip = vec4<f32>(2.0, 2.0, 2.0, 1.0); // behind the camera: outside the clip volume
        return o;
    }
    var tc = array<vec2<f32>, 6>(vec2<f32>(0.0, -1.0), vec2<f32>(0.0, 1.0), vec2<f32>(1.0, -1.0), vec2<f32>(1.0, -1.0), vec2<f32>(0.0, 1.0), vec2<f32>(1.0, 1.0));
    let c = tc[i.vi];
    let hs = u.res.xy * 0.5;
    let sa = ca.xy / ca.w * hs;
    let sb = cb.xy / cb.w * hs;
    var d = sb - sa;
    let l = length(d);
    if (l < 1e-4) {
        d = vec2<f32>(1.0, 0.0);
    } else {
        d = d / l;
    }
    let nrm = vec2<f32>(-d.y, d.x);
    let off = (nrm * c.y + d * (c.x * 2.0 - 1.0)) * i.w * 0.5; // sideways + a hs-width cap so joints close
    let m = mix(ca, cb, c.x);
    o.clip = vec4<f32>(m.xy + off / hs * m.w, m.z, m.w);
    return o;
}
@vertex fn vs_seg(i: SIn) -> VOut {
    return seg_clip(i, false);
}
@vertex fn vs_seg_low(i: SIn) -> VOut {
    return seg_clip(i, true);
}

@fragment fn fs_mesh(i: VOut) -> @location(0) vec4<f32> {
    let l = abs(normalize(i.n).z); // headlight, two-sided
    return vec4<f32>(vec3<f32>(0.25) + vec3<f32>(0.55, 0.6, 0.65) * l, 1.0);
}
@fragment fn fs_low(i: VOut) -> @location(0) vec4<f32> {
    let l = abs(normalize(i.n).z);
    return vec4<f32>(u.low.rgb * (0.6 + 0.4 * l), u.low.a);
}
@fragment fn fs_line(i: VOut) -> @location(0) vec4<f32> {
    return vec4<f32>(i.n, 1.0);
}
