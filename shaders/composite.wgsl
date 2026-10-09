// Composite: show the background through keyed levels, add bloom, then CRT treatment
// (chromatic bleed, scanlines, noise), letterboxed into the output. A transparent
// composite (for alpha capture) leaves the background out and writes straight alpha.

struct Composite {
    glow: vec4<f32>, // bloom intensity, scanline strength, scanline count, chroma offset (uv)
    misc: vec4<f32>, // noise, time, internal width, internal height
    fit: vec4<f32>,  // image size as a fraction of the output (x, y), encode sRGB (z: 0 or 1), transparent (w: 0 or 1)
    background: vec4<f32>, // background uv scale for a "cover" fit (x, y), unused, unused
};

@group(0) @binding(0) var<uniform> u: Composite;
@group(0) @binding(1) var samp: sampler;
@group(0) @binding(2) var image: texture_2d<f32>;
@group(0) @binding(3) var bloom: texture_2d<f32>;
@group(0) @binding(4) var background: texture_2d<f32>;

// Exact sRGB encoding, for outputs that are not sRGB formats.
fn linear_to_srgb(c: vec3<f32>) -> vec3<f32> {
    return select(
        1.055 * pow(c, vec3<f32>(1.0 / 2.4)) - 0.055,
        12.92 * c,
        c <= vec3<f32>(0.0031308),
    );
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let transparent = u.fit.w > 0.5;
    let iuv = (in.uv - 0.5) / u.fit.xy + 0.5;
    if !inside01(iuv) {
        return vec4<f32>(0.0, 0.0, 0.0, select(1.0, 0.0, transparent));
    }
    let off = vec2<f32>(u.glow.w, 0.0);
    var c = vec3<f32>(
        textureSampleLevel(image, samp, iuv + off, 0.0).r,
        textureSampleLevel(image, samp, iuv, 0.0).g,
        textureSampleLevel(image, samp, iuv - off, 0.0).b,
    );
    // The image is premultiplied: the background shows where it is see-through.
    var alpha = textureSampleLevel(image, samp, iuv, 0.0).a;
    if !transparent {
        let buv = (iuv - 0.5) * u.background.xy + 0.5;
        c += textureSampleLevel(background, samp, buv, 0.0).rgb * (1.0 - alpha);
        alpha = 1.0;
    }
    // The upsample chain sums all bloom levels, so scale it back down.
    let glow = textureSampleLevel(bloom, samp, iuv, 0.0).rgb * u.glow.x * 0.2;
    c += glow;
    // Glow over a see-through part is light on its own: as opaque as it is bright.
    alpha = min(alpha + max(glow.r, max(glow.g, glow.b)), 1.0);
    let scan = 0.5 + 0.5 * cos(iuv.y * u.glow.z * TAU);
    c *= mix(1.0, scan, u.glow.y);
    let grain = hash12(floor(iuv * u.misc.zw) + fract(u.misc.y) * 113.0) - 0.5;
    // Premultiplied, so grain stays off the see-through parts.
    c += grain * u.misc.x * alpha;
    c = max(c, vec3<f32>(0.0));
    if transparent {
        // Files keep straight alpha: undo the premultiplication.
        c = select(c / alpha, vec3<f32>(0.0), alpha <= 0.0);
    }
    if u.fit.z > 0.5 {
        c = linear_to_srgb(c);
    }
    return vec4<f32>(c, alpha);
}
