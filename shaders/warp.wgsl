// Deflection: displace the sampling position with summed oscillators.
// deflection.wgsl is prepended.

@group(0) @binding(0) var<uniform> u: Deflection;
@group(0) @binding(1) var samp: sampler;
@group(0) @binding(2) var source: texture_2d<f32>;

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let aspect = u.frame.y;
    var p = (in.uv - 0.5) * vec2<f32>(aspect, 1.0);
    // Each output row is a scan line; its time-base error shifts it sideways.
    p.x += line_shift(u, u32(in.pos.y));
    let q = to_source(u, p + displacement(u, p));
    let suv = q / u.source_size.xy + 0.5;
    if !inside01(suv) {
        return vec4<f32>(0.0, 0.0, 0.0, 1.0);
    }
    let g = textureSampleLevel(source, samp, suv, 0.0).r;
    return vec4<f32>(g, g, g, 1.0);
}
