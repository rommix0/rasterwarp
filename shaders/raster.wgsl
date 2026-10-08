// True raster mode: draws the source as scan lines, each mapped forward through the
// global transform and the deflection, with a Gaussian beam profile and additive
// blending. fullscreen.wgsl and deflection.wgsl are prepended.

struct Raster {
    now: Deflection,
    before: Deflection, // the previous canvas frame's deflection, for the beam speed
    beam: vec4<f32>,    // lines, beam width (frame heights), compensation, speed compensation
    misc: vec4<f32>,    // samples per line, unused, unused, unused
};

@group(0) @binding(0) var<uniform> r: Raster;
@group(0) @binding(1) var samp: sampler;
@group(0) @binding(2) var source: texture_2d<f32>;

struct BeamOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) suv: vec2<f32>,  // source uv under the beam centre
    @location(1) across: f32,     // -1 to 1 across the drawn strip
    @location(2) gain: f32,       // area and speed compensation
};

// Where the beam for source frame position q lands, before line jitter.
fn deflected(w: Deflection, q: vec2<f32>) -> vec2<f32> {
    let s = from_source(w, q);
    return s + displacement(w, s);
}

@vertex
fn vs_beam(@builtin(vertex_index) vi: u32, @builtin(instance_index) line: u32) -> BeamOut {
    let lines = r.beam.x;
    let samples = r.misc.x;
    let size = r.now.source_size.xy;
    // Each line is a triangle strip: two vertices (one per side) per sample.
    let u = f32(vi / 2u) / samples;
    let side = select(-1.0, 1.0, (vi & 1u) == 1u);
    let v = (f32(line) + 0.5) / lines;
    let q = (vec2<f32>(u, v) - 0.5) * size;
    let centre = deflected(r.now, q);

    // The line's direction, from its neighbouring samples.
    let du = vec2<f32>(size.x / samples, 0.0);
    let along = deflected(r.now, q + du) - deflected(r.now, q - du);
    let len = length(along);
    let normal = select(vec2<f32>(0.0, 1.0), vec2<f32>(-along.y, along.x) / len, len > 1e-8);

    // Area compensation: lines spread apart brighten, packed lines dim, measured as the
    // distance to the next line across this one against the undeflected spacing.
    let rest = size.y / lines;
    let next = deflected(r.now, q + vec2<f32>(0.0, rest));
    let spacing = abs(dot(next - centre, normal)) / rest;
    let area = mix(1.0, spacing, r.beam.z);

    // Speed compensation: fast-moving beams brighten so fast sweeps don't fade. Speed is
    // in frame heights per second since the previous canvas frame (none while paused).
    let dt = r.now.frame.x - r.before.frame.x;
    let moved = length(centre - deflected(r.before, q));
    let speed = select(0.0, moved / dt, dt > 0.0);
    let boost = 1.0 + r.beam.w * min(speed / 0.5, 4.0);

    // Each line's time-base error shifts it sideways.
    var p = centre;
    p.x += line_shift(r.now, line);
    // The strip reaches one beam width either side of the centre: the beam's Gaussian is
    // half that wide, so the edges are already dark.
    p += normal * side * r.beam.y;

    var out: BeamOut;
    let aspect = r.now.frame.y;
    out.pos = vec4<f32>(p.x / (0.5 * aspect), -2.0 * p.y, 0.0, 1.0);
    out.suv = vec2<f32>(u, v);
    out.across = side;
    out.gain = area * boost;
    return out;
}

@fragment
fn fs_beam(in: BeamOut) -> @location(0) vec4<f32> {
    let g = textureSampleLevel(source, samp, in.suv, 0.0).r;
    // Gaussian with sigma = half the beam width.
    let profile = exp(-2.0 * in.across * in.across);
    let b = g * profile * in.gain;
    return vec4<f32>(b, b, b, 1.0);
}
