// Focale viewport shader: working-space (linear Rec.2020) → display.
//
// Mirrors the CPU export pathway (focale-core color module) so the preview
// is perceptually faithful to the export (HARD-DET; docs/subsystems/color.md):
//   1. extended Reinhard tone map on max-RGB (white point uniform)
//   2. gamut map into the active rendering gamut: fast path if in gamut,
//      else hue-preserving Oklab chroma compression, 20 fixed bisections
//   3. active gamut → display (sRGB assumed in v1) linear matrix, clamp
//   4. sRGB encode unless the surface format is sRGB (hardware encodes)
//
// Every matrix is uploaded from focale-core's constants — the shader holds
// no colour constants of its own.

struct Mat {
    r0: vec4<f32>,
    r1: vec4<f32>,
    r2: vec4<f32>,
};

struct Uniforms {
    rec2020_to_xyz: Mat,
    oklab_m1: Mat,
    oklab_m2: Mat,
    oklab_m1_inv: Mat,
    oklab_m2_inv: Mat,
    xyz_to_target: Mat,
    rec2020_to_target: Mat,
    target_to_display: Mat,
    // xy = uv scale, zw = uv offset (pan/zoom).
    uv_transform: vec4<f32>,
    // x = reinhard white point, y = flags (1.0 = encode sRGB in shader),
    // z = background luminance for out-of-image area, w unused.
    params: vec4<f32>,
};

@group(0) @binding(0) var t_image: texture_2d<f32>;
@group(0) @binding(1) var s_image: sampler;
@group(0) @binding(2) var<uniform> u: Uniforms;

fn mul3(m: Mat, v: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(dot(m.r0.xyz, v), dot(m.r1.xyz, v), dot(m.r2.xyz, v));
}

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

// Fullscreen triangle over the callback viewport.
@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> VsOut {
    var out: VsOut;
    let x = f32(i32(vi & 1u) * 4 - 1);
    let y = f32(i32(vi >> 1u) * 4 - 1);
    out.pos = vec4<f32>(x, -y, 0.0, 1.0);
    out.uv = vec2<f32>((x + 1.0) * 0.5, (y + 1.0) * 0.5);
    return out;
}

fn cbrt(x: f32) -> f32 {
    // Oklab LMS values are non-negative in practice; guard anyway.
    return sign(x) * pow(abs(x), 1.0 / 3.0);
}

fn xyz_to_oklab(xyz: vec3<f32>) -> vec3<f32> {
    let lms = mul3(u.oklab_m1, xyz);
    return mul3(u.oklab_m2, vec3<f32>(cbrt(lms.x), cbrt(lms.y), cbrt(lms.z)));
}

fn oklab_to_xyz(lab: vec3<f32>) -> vec3<f32> {
    let p = mul3(u.oklab_m2_inv, lab);
    return mul3(u.oklab_m1_inv, p * p * p);
}

const GAMUT_EPS: f32 = 1e-4;

fn in_gamut(rgb: vec3<f32>) -> bool {
    return all(rgb >= vec3<f32>(-GAMUT_EPS)) && all(rgb <= vec3<f32>(1.0 + GAMUT_EPS));
}

// Mirrors focale_core::color::gamut_map::map_to_gamut (v1, frozen).
fn map_to_gamut(rec2020: vec3<f32>) -> vec3<f32> {
    let direct = mul3(u.rec2020_to_target, rec2020);
    if in_gamut(direct) {
        return clamp(direct, vec3<f32>(0.0), vec3<f32>(1.0));
    }
    let lab = xyz_to_oklab(mul3(u.rec2020_to_xyz, rec2020));
    var lo = 0.0;
    var hi = 1.0;
    for (var i = 0u; i < 20u; i += 1u) {
        let mid = 0.5 * (lo + hi);
        let cand = mul3(u.xyz_to_target, oklab_to_xyz(vec3<f32>(lab.x, lab.y * mid, lab.z * mid)));
        if in_gamut(cand) {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    let mapped = mul3(u.xyz_to_target, oklab_to_xyz(vec3<f32>(lab.x, lab.y * lo, lab.z * lo)));
    return clamp(mapped, vec3<f32>(0.0), vec3<f32>(1.0));
}

// Mirrors focale_core::color::tonemap::tonemap_reinhard_extended.
fn tonemap(rgb: vec3<f32>, white: f32) -> vec3<f32> {
    let m = max(rgb.r, max(rgb.g, rgb.b));
    if m <= 0.0 {
        return rgb;
    }
    let w2 = white * white;
    let scale = (1.0 + m / w2) / (1.0 + m);
    return rgb * scale;
}

fn srgb_encode(c: f32) -> f32 {
    if c <= 0.0031308 {
        return 12.92 * c;
    }
    return 1.055 * pow(c, 1.0 / 2.4) - 0.055;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let uv = in.uv * u.uv_transform.xy + u.uv_transform.zw;
    let bg = u.params.z;
    if uv.x < 0.0 || uv.x > 1.0 || uv.y < 0.0 || uv.y > 1.0 {
        var b = bg;
        if u.params.y >= 0.5 {
            b = srgb_encode(bg);
        }
        return vec4<f32>(b, b, b, 1.0);
    }
    let working = textureSample(t_image, s_image, uv).rgb;
    let toned = tonemap(working, u.params.x);
    let gamut_mapped = map_to_gamut(toned);
    var display = clamp(mul3(u.target_to_display, gamut_mapped), vec3<f32>(0.0), vec3<f32>(1.0));
    if u.params.y >= 0.5 {
        display = vec3<f32>(
            srgb_encode(display.r),
            srgb_encode(display.g),
            srgb_encode(display.b),
        );
    }
    return vec4<f32>(display, 1.0);
}
