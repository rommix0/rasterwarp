// Colorizing: posterize brightness into levels at adjustable thresholds, map each level
// to a palette color.

struct Colorize {
    settings: vec4<f32>, // levels, softness, cycle offset (levels), bypass (0 or 1)
    palette: array<vec4<f32>, 8>, // linear RGB
    thresholds: array<vec4<f32>, 2>, // 7 thresholds; only the first levels - 1 are used
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

// Fractional level index of brightness g: the number of thresholds at or below g, each
// step softened over `width` centered on its threshold. Order doesn't matter.
fn level_of(g: f32, levels: u32, width: f32) -> f32 {
    var x = 0.0;
    for (var k = 0u; k + 1u < levels; k++) {
        let th = u.thresholds[k / 4u][k % 4u];
        if width > 0.0 {
            x += smoothstep(th - 0.5 * width, th + 0.5 * width, g);
        } else {
            x += select(0.0, 1.0, g >= th);
        }
    }
    return x;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let g = textureSampleLevel(source, samp, in.uv, 0.0).r;
    if u.settings.w > 0.5 {
        let l = pow(g, 2.2);
        return vec4<f32>(l, l, l, 1.0);
    }
    let levels = u.settings.x;
    let n = u32(levels);
    // Softness is in level bands: an even band is 1 / levels of brightness.
    let x = level_of(g, n, u.settings.y / levels);
    let i0 = min(floor(x), levels - 1.0);
    let i1 = min(i0 + 1.0, levels - 1.0);
    let cycle = u.settings.z;
    let color = mix(palette_at(i0 + cycle, n), palette_at(i1 + cycle, n), x - i0);
    return vec4<f32>(color, 1.0);
}
