// Bloom: threshold + downsample chain, then upsample back up adding each level.

struct Bloom {
    settings: vec4<f32>, // threshold, unused, unused, unused
};

@group(0) @binding(0) var<uniform> u: Bloom;
@group(0) @binding(1) var samp: sampler;
@group(0) @binding(2) var src: texture_2d<f32>;

fn tap(uv: vec2<f32>) -> vec3<f32> {
    return textureSampleLevel(src, samp, uv, 0.0).rgb;
}

// 4 bilinear taps one source texel off each diagonal: a smooth 2x downsample.
fn box4(uv: vec2<f32>) -> vec3<f32> {
    let t = 1.0 / vec2<f32>(textureDimensions(src));
    return 0.25 * (tap(uv + t * vec2<f32>(-1.0, -1.0)) + tap(uv + t * vec2<f32>(1.0, -1.0))
        + tap(uv + t * vec2<f32>(-1.0, 1.0)) + tap(uv + t * vec2<f32>(1.0, 1.0)));
}

@fragment
fn fs_prefilter(in: VsOut) -> @location(0) vec4<f32> {
    let c = box4(in.uv);
    let brightness = max(c.r, max(c.g, c.b));
    let keep = max(brightness - u.settings.x, 0.0) / max(brightness, 1e-4);
    return vec4<f32>(c * keep, 1.0);
}

@fragment
fn fs_down(in: VsOut) -> @location(0) vec4<f32> {
    return vec4<f32>(box4(in.uv), 1.0);
}

// 3x3 tent filter over the smaller level; additively blended onto the larger one.
@fragment
fn fs_up(in: VsOut) -> @location(0) vec4<f32> {
    let t = 1.0 / vec2<f32>(textureDimensions(src));
    var c = tap(in.uv) * 4.0;
    c += (tap(in.uv + vec2<f32>(t.x, 0.0)) + tap(in.uv - vec2<f32>(t.x, 0.0))
        + tap(in.uv + vec2<f32>(0.0, t.y)) + tap(in.uv - vec2<f32>(0.0, t.y))) * 2.0;
    c += tap(in.uv + t) + tap(in.uv - t) + tap(in.uv + vec2<f32>(t.x, -t.y))
        + tap(in.uv + vec2<f32>(-t.x, t.y));
    return vec4<f32>(c / 16.0, 1.0);
}
