// Frames: draws an input's picture from its ring of buffered frames. Each pixel finds
// the moment it shows (the playhead, or further back for slit-scan) as a fractional
// frame index, then shows the nearest frame or blends the two either side.

struct Frames {
    // x: the playhead, as frames after the window's first frame
    // y: frames behind the playhead at the far end of the slit-scan map (signed)
    // z: the window's last frame, after its first
    // w: unused
    playhead: vec4<f32>,
    // x: the ring layer holding the window's first frame
    // y: layers in the ring
    // z: slit-scan: 0 off, 1 rows, 2 columns, 3 map
    // w: bit 0 flips the map, bit 1 blends between frames
    ring: vec4<u32>,
};

@group(0) @binding(0) var<uniform> u: Frames;
@group(0) @binding(1) var samp: sampler;
@group(0) @binding(2) var frames: texture_2d_array<f32>;
@group(0) @binding(3) var slit_map: texture_2d<f32>;

// How far back this pixel looks, 0 (now) to 1 (the full depth).
fn slit_amount(uv: vec2<f32>) -> f32 {
    var m = 0.0;
    switch u.ring.z {
        case 1u: { m = uv.y; }
        case 2u: { m = uv.x; }
        case 3u: { m = textureSampleLevel(slit_map, samp, uv, 0.0).r; }
        default: { return 0.0; }
    }
    if (u.ring.w & 1u) != 0u {
        m = 1.0 - m;
    }
    return m;
}

// Frame `i` of the window (0 = its first frame).
fn frame(uv: vec2<f32>, i: f32) -> vec4<f32> {
    let layer = (u.ring.x + u32(i)) % u.ring.y;
    return textureSampleLevel(frames, samp, uv, i32(layer), 0.0);
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let last = u.playhead.z;
    let v = clamp(u.playhead.x - u.playhead.y * slit_amount(in.uv), 0.0, last);
    if (u.ring.w & 2u) == 0u {
        return frame(in.uv, min(round(v), last));
    }
    let lo = floor(v);
    let hi = min(lo + 1.0, last);
    return mix(frame(in.uv, lo), frame(in.uv, hi), v - lo);
}
