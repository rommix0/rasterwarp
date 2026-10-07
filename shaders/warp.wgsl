// Deflection: displace the sampling position with summed oscillators.

struct Osc {
    mode: vec4<u32>, // waveform, target (0 = X, 1 = Y), input (0 = U, 1 = V, 2 = radius, 3 = time), oscillator index
    wave: vec4<f32>, // frequency, amplitude (weighted), phase (cycles), unused
    lfo: vec4<f32>,  // phase (cycles), depth, unused, unused
};

struct Warp {
    frame: vec4<f32>,       // time, frame aspect, drift, slot count
    transform: vec4<f32>,   // zoom, rotation, offset.x, offset.y
    source_size: vec4<f32>, // source width, height in frame units, unused, unused
    osc: array<Osc, 8>,
};

@group(0) @binding(0) var<uniform> u: Warp;
@group(0) @binding(1) var samp: sampler;
@group(0) @binding(2) var source: texture_2d<f32>;

// One cycle per unit of x, output in [-1, 1].
fn wave(shape: u32, x: f32) -> f32 {
    let f = fract(x);
    switch shape {
        case 0u: { return sin(x * TAU); }
        case 1u: { return 1.0 - 4.0 * abs(f - 0.5); }
        case 2u: { return 2.0 * f - 1.0; }
        case 3u: { return select(1.0, -1.0, f >= 0.5); }
        default: { return value_noise(x); }
    }
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let time = u.frame.x;
    let aspect = u.frame.y;
    let drift = u.frame.z;
    // Centered frame coordinates: y spans [-0.5, 0.5], x spans [-aspect/2, aspect/2].
    let p = (in.uv - 0.5) * vec2<f32>(aspect, 1.0);

    var d = vec2<f32>(0.0);
    let slots = u32(u.frame.w);
    for (var i = 0u; i < slots; i++) {
        let o = u.osc[i];
        var input = 0.0;
        switch o.mode.z {
            case 0u: { input = p.x; }
            case 1u: { input = p.y; }
            case 2u: { input = length(p); }
            default: { input = 0.0; }
        }
        // Seed drift by oscillator, not slot, so it doesn't jump when slots appear.
        let seed = f32(o.mode.w) * 17.0;
        let freq = o.wave.x * (1.0 + drift * 0.05 * value_noise(time * 0.3 + seed));
        let phase = o.wave.z + drift * 0.1 * value_noise(time * 0.2 + seed + 5.0);
        let lfo = 1.0 - o.lfo.y * (0.5 - 0.5 * cos(TAU * o.lfo.x));
        let v = wave(o.mode.x, input * freq + phase) * o.wave.y * lfo;
        if o.mode.y == 0u {
            d.x += v;
        } else {
            d.y += v;
        }
    }

    let q = rotate2(p + d, u.transform.y) / u.transform.x - u.transform.zw;
    let suv = q / u.source_size.xy + 0.5;
    if !inside01(suv) {
        return vec4<f32>(0.0, 0.0, 0.0, 1.0);
    }
    let g = textureSampleLevel(source, samp, suv, 0.0).r;
    return vec4<f32>(g, g, g, 1.0);
}
