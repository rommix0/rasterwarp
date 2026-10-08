// Deflection shared by the warp and raster passes: summed oscillators, the global
// transform with its wandering rotation axis, and line timing jitter. Positions are in
// frame heights, centered: y spans [-0.5, 0.5], x spans [-aspect/2, aspect/2].

struct Osc {
    mode: vec4<u32>, // waveform, target (0 = X, 1 = Y), input (0 = U, 1 = V, 2 = radius, 3 = time), oscillator index
    wave: vec4<f32>, // frequency, amplitude (weighted), phase (cycles), unused
    lfo: vec4<f32>,  // phase (cycles), depth, unused, unused
};

struct Deflection {
    frame: vec4<f32>,       // time, frame aspect, drift, slot count
    transform: vec4<f32>,   // zoom, rotation, offset.x, offset.y
    source_size: vec4<f32>, // source width, height in frame units, unused, unused
    look: vec4<f32>,        // line jitter (frame heights), axis wander, unused, unused
    seed: vec4<u32>,        // line jitter seed (animation frame count), unused, unused, unused
    osc: array<Osc, 8>,
};

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

// The summed oscillator displacement at frame position p.
fn displacement(w: Deflection, p: vec2<f32>) -> vec2<f32> {
    let time = w.frame.x;
    let drift = w.frame.z;
    var d = vec2<f32>(0.0);
    let slots = u32(w.frame.w);
    for (var i = 0u; i < slots; i++) {
        let o = w.osc[i];
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
    return d;
}

// Where the rotation axis has wandered to: up to 2% of the frame height times the axis
// wander, scaled by how far the frame is rotated, drifting slowly with time.
fn pivot(w: Deflection) -> vec2<f32> {
    let time = w.frame.x;
    let amount = w.look.y * 0.02 * abs(sin(w.transform.y));
    return amount * vec2<f32>(value_noise(time * 0.15 + 31.0), value_noise(time * 0.15 + 57.0));
}

// The global transform, inverse direction: deflected frame position to source frame
// position (the warp pass's sampling map).
fn to_source(w: Deflection, s: vec2<f32>) -> vec2<f32> {
    let c = pivot(w);
    return (rotate2(s - c, w.transform.y) + c) / w.transform.x - w.transform.zw;
}

// The global transform, forward direction: source frame position to frame position (the
// raster pass's beam map). Exactly undoes `to_source`.
fn from_source(w: Deflection, q: vec2<f32>) -> vec2<f32> {
    let c = pivot(w);
    return rotate2((q + w.transform.zw) * w.transform.x - c, -w.transform.y) + c;
}

// PCG integer hash.
fn pcg(v: u32) -> u32 {
    let state = v * 747796405u + 2891336453u;
    let word = ((state >> ((state >> 28u) + 4u)) ^ state) * 277803737u;
    return (word >> 22u) ^ word;
}

// Line `line`'s sideways time-base error this animation frame, in frame heights: a
// uniform random amount within ± the line jitter.
fn line_shift(w: Deflection, line: u32) -> f32 {
    let r = f32(pcg(line ^ pcg(w.seed.x))) / 4294967295.0;
    return w.look.x * (2.0 * r - 1.0);
}
