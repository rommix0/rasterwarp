//! The source plane's pseudo-3D tilt: the flat picture turned about the screen's
//! horizontal and vertical axes through the frame centre, then seen in perspective. A
//! flat plane seen this way is a 3×3 projective map (a homography), built here and
//! applied by deflection.wgsl after the 2D zoom, offset and rotation.
//!
//! Positions are in frame heights, centred, y down (as in deflection.wgsl). Depth is
//! away from the viewer; the eye sits `1 / perspective` frame heights in front of the
//! frame, and no perspective is a flat (orthographic) squash.

/// Row-major 3×3 matrix.
pub type Matrix = [[f32; 3]; 3];

/// Below this, the plane is edge-on to the eye and no screen point sees it.
const EDGE_ON: f32 = 1e-4;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tilt {
    /// Flat position to screen position, as homogeneous coordinates.
    pub forward: Matrix,
    /// Screen position back to flat position: all zeros when the plane is edge-on.
    pub back: Matrix,
}

impl Tilt {
    /// Tilt X (radians) tips the top away; tilt Y (radians) turns the right side away.
    /// Past a quarter turn the plane shows its back, mirrored.
    pub fn new(tilt_x: f32, tilt_y: f32, perspective: f32) -> Tilt {
        let (sa, ca) = tilt_x.sin_cos();
        let (sb, cb) = tilt_y.sin_cos();
        let p = perspective;
        // The point (x, y, 0) turned about the vertical axis by tilt y, then about the
        // horizontal axis by tilt x, lands at (x', y', z) = (x cb, y ca + x sb sa,
        // x sb ca - y sa); the eye scales it by 1 / (1 + p z).
        let forward = [
            [cb, 0.0, 0.0],
            [sa * sb, ca, 0.0],
            [p * sb * ca, -p * sa, 1.0],
        ];
        Tilt {
            forward,
            back: inverse(&forward).unwrap_or([[0.0; 3]; 3]),
        }
    }

    /// Where flat position `r` shows on screen, or `None` behind the eye.
    pub fn forward(&self, r: [f32; 2]) -> Option<[f32; 2]> {
        project(&self.forward, r)
    }

    /// The flat position screen position `s` shows, or `None` where it sees no plane.
    pub fn back(&self, s: [f32; 2]) -> Option<[f32; 2]> {
        project(&self.back, s)
    }
}

/// `m` applied to (v, 1), divided through, if the result is in front of the eye.
fn project(m: &Matrix, v: [f32; 2]) -> Option<[f32; 2]> {
    let h = m.map(|row| row[0] * v[0] + row[1] * v[1] + row[2]);
    (h[2] > 0.0).then(|| [h[0] / h[2], h[1] / h[2]])
}

fn inverse(m: &Matrix) -> Option<Matrix> {
    let cof =
        |r0: usize, r1: usize, c0: usize, c1: usize| m[r0][c0] * m[r1][c1] - m[r0][c1] * m[r1][c0];
    let adj = [
        [cof(1, 2, 1, 2), -cof(0, 2, 1, 2), cof(0, 1, 1, 2)],
        [-cof(1, 2, 0, 2), cof(0, 2, 0, 2), -cof(0, 1, 0, 2)],
        [cof(1, 2, 0, 1), -cof(0, 2, 0, 1), cof(0, 1, 0, 1)],
    ];
    let det = m[0][0] * adj[0][0] + m[0][1] * adj[1][0] + m[0][2] * adj[2][0];
    (det.abs() >= EDGE_ON).then(|| adj.map(|row| row.map(|v| v / det)))
}

/// `m` as the three padded columns of a WGSL `mat3x3<f32>`.
pub fn columns(m: &Matrix) -> [[f32; 4]; 3] {
    std::array::from_fn(|c| [m[0][c], m[1][c], m[2][c], 0.0])
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::{FRAC_PI_2, PI};

    const IDENTITY: Matrix = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];

    fn close(a: [f32; 2], b: [f32; 2]) -> bool {
        (a[0] - b[0]).abs() < 1e-4 && (a[1] - b[1]).abs() < 1e-4
    }

    #[test]
    fn no_tilt_is_the_identity_at_any_perspective() {
        for p in [0.0, 0.5, 1.0] {
            let t = Tilt::new(0.0, 0.0, p);
            assert_eq!((t.forward, t.back), (IDENTITY, IDENTITY));
        }
    }

    #[test]
    fn back_undoes_forward() {
        let t = Tilt::new(0.7, -0.4, 0.8);
        for r in [[0.3, -0.2], [-0.8, 0.45], [0.0, 0.0], [0.6, 0.5]] {
            let s = t.forward(r).unwrap();
            assert!(close(t.back(s).unwrap(), r), "{r:?}");
        }
    }

    #[test]
    fn tilting_x_tips_the_top_away() {
        let t = Tilt::new(0.6, 0.0, 1.0);
        let top = t.forward([0.5, -0.5]).unwrap();
        let bottom = t.forward([0.5, 0.5]).unwrap();
        assert!(top[0] < bottom[0], "the far top edge draws narrower");
        assert!(-top[1] < bottom[1], "and closer to the centre");
        // Flat, both edges just squash.
        let flat = Tilt::new(0.6, 0.0, 0.0);
        assert!(close(
            flat.forward([0.5, -0.5]).unwrap(),
            [0.5, -0.5 * 0.6f32.cos()]
        ));
    }

    #[test]
    fn tilting_y_turns_the_right_side_away() {
        let t = Tilt::new(0.0, 0.6, 1.0);
        let right = t.forward([0.5, 0.5]).unwrap();
        let left = t.forward([-0.5, 0.5]).unwrap();
        assert!(right[1] < left[1], "the far right edge draws shorter");
    }

    #[test]
    fn a_half_turn_shows_the_back_mirrored() {
        let t = Tilt::new(PI, 0.0, 0.5);
        assert!(close(t.forward([0.3, 0.2]).unwrap(), [0.3, -0.2]));
        let t = Tilt::new(0.0, PI, 0.5);
        assert!(close(t.forward([0.3, 0.2]).unwrap(), [-0.3, 0.2]));
    }

    #[test]
    fn an_edge_on_plane_shows_nothing() {
        for (x, y) in [(FRAC_PI_2, 0.0), (0.0, -FRAC_PI_2), (FRAC_PI_2, 1.0)] {
            let t = Tilt::new(x, y, 0.5);
            assert_eq!(t.back, [[0.0; 3]; 3]);
            assert_eq!(t.back([0.1, 0.1]), None);
        }
    }

    #[test]
    fn points_behind_the_eye_have_no_place() {
        // The eye is one frame height away; tipped toward it by a quarter turn less a
        // little, the top of a zoomed-up picture swings past it.
        let t = Tilt::new(-1.4, 0.0, 1.0);
        assert!(t.forward([0.0, -0.2]).is_some());
        assert_eq!(t.forward([0.0, -2.0]), None);
        // The rest of the plane runs off to a horizon a little below the centre, and
        // screen points past it see no plane.
        assert!(t.back([0.0, 0.1]).is_some());
        assert_eq!(t.back([0.0, 0.5]), None);
    }

    #[test]
    fn columns_pad_a_wgsl_mat3() {
        let m = [[1.0, 2.0, 3.0], [4.0, 5.0, 6.0], [7.0, 8.0, 9.0]];
        assert_eq!(
            columns(&m),
            [
                [1.0, 4.0, 7.0, 0.0],
                [2.0, 5.0, 8.0, 0.0],
                [3.0, 6.0, 9.0, 0.0]
            ]
        );
    }
}
