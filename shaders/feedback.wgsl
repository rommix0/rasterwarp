// Feedback: keep a decaying, transformed copy of the previous frame under the new one.
// Color and alpha (keying) both take the brighter of the new frame and the decayed trail.

struct Feedback {
    settings: vec4<f32>, // amount, zoom, rotation, frame aspect
    offset: vec4<f32>,   // offset.x, offset.y, unused, unused
};

@group(0) @binding(0) var<uniform> u: Feedback;
@group(0) @binding(1) var samp: sampler;
@group(0) @binding(2) var current: texture_2d<f32>;
@group(0) @binding(3) var previous: texture_2d<f32>;

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let cur = textureSampleLevel(current, samp, in.uv, 0.0);
    let aspect = u.settings.w;
    let p = (in.uv - 0.5) * vec2<f32>(aspect, 1.0);
    let q = rotate2(p, -u.settings.z) / u.settings.y - u.offset.xy;
    let puv = q / vec2<f32>(aspect, 1.0) + 0.5;
    var prev = vec4<f32>(0.0);
    if inside01(puv) {
        prev = textureSampleLevel(previous, samp, puv, 0.0);
    }
    return max(cur, prev * u.settings.x);
}
