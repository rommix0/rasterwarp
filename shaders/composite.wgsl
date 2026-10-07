// Composite: add bloom, then CRT treatment (chromatic bleed, scanlines, noise),
// letterboxed into the output.

struct Composite {
    glow: vec4<f32>, // bloom intensity, scanline strength, scanline count, chroma offset (uv)
    misc: vec4<f32>, // noise, time, internal width, internal height
    fit: vec4<f32>,  // image size as a fraction of the output (x, y), unused, unused
};

@group(0) @binding(0) var<uniform> u: Composite;
@group(0) @binding(1) var samp: sampler;
@group(0) @binding(2) var image: texture_2d<f32>;
@group(0) @binding(3) var bloom: texture_2d<f32>;

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let iuv = (in.uv - 0.5) / u.fit.xy + 0.5;
    if !inside01(iuv) {
        return vec4<f32>(0.0, 0.0, 0.0, 1.0);
    }
    let off = vec2<f32>(u.glow.w, 0.0);
    var c = vec3<f32>(
        textureSampleLevel(image, samp, iuv + off, 0.0).r,
        textureSampleLevel(image, samp, iuv, 0.0).g,
        textureSampleLevel(image, samp, iuv - off, 0.0).b,
    );
    // The upsample chain sums all bloom levels, so scale it back down.
    c += textureSampleLevel(bloom, samp, iuv, 0.0).rgb * u.glow.x * 0.2;
    let scan = 0.5 + 0.5 * cos(iuv.y * u.glow.z * TAU);
    c *= mix(1.0, scan, u.glow.y);
    let grain = hash12(floor(iuv * u.misc.zw) + fract(u.misc.y) * 113.0) - 0.5;
    c += grain * u.misc.x;
    return vec4<f32>(max(c, vec3<f32>(0.0)), 1.0);
}
