// Colorizing: posterize brightness into levels, map each level to a palette color.

struct Colorize {
    settings: vec4<f32>, // levels, softness, cycle offset (levels), bypass (0 or 1)
    palette: array<vec4<f32>, 8>, // linear RGB
};

@group(0) @binding(0) var<uniform> u: Colorize;
@group(0) @binding(1) var samp: sampler;
@group(0) @binding(2) var source: texture_2d<f32>;

// Palette color for a (possibly fractional) level index, wrapping and blending
// between neighbors so palette cycling is smooth.
fn palette_at(c: f32, levels: u32) -> vec3<f32> {
    let i0 = floor(c);
    let a = u32(i0) % levels;
    let b = (a + 1u) % levels;
    return mix(u.palette[a].rgb, u.palette[b].rgb, c - i0);
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let g = textureSampleLevel(source, samp, in.uv, 0.0).r;
    if u.settings.w > 0.5 {
        let l = pow(g, 2.2);
        return vec4<f32>(l, l, l, 1.0);
    }
    let levels = u.settings.x;
    let softness = u.settings.y;
    let cycle = u.settings.z;
    let x = g * levels;
    let i0 = min(floor(x), levels - 1.0);
    let i1 = min(i0 + 1.0, levels - 1.0);
    let t = select(0.0, smoothstep(1.0 - softness, 1.0, fract(x)), softness > 0.0);
    let n = u32(levels);
    let color = mix(palette_at(i0 + cycle, n), palette_at(i1 + cycle, n), t);
    return vec4<f32>(color, 1.0);
}
