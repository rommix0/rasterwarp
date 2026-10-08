//! Transition curves: built-in Linear and S-curve, plus user-drawn curves.

use std::ops::RangeInclusive;

use serde::{Deserialize, Serialize};

/// Allowed y range for user curve points (overshoot and anticipation).
pub const Y_RANGE: RangeInclusive<f32> = -0.5..=1.5;
/// Minimum x gap kept between neighbouring points.
pub const MIN_GAP: f32 = 0.01;
/// Maximum number of user curves in the library.
pub const MAX_CUSTOM: usize = 8;

/// Which curve a transition or cue uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CurveRef {
    Linear,
    SCurve,
    /// A user curve, by its stable id in the library.
    Custom(u32),
}

impl Serialize for CurveRef {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            CurveRef::Linear => s.serialize_str("linear"),
            CurveRef::SCurve => s.serialize_str("s-curve"),
            CurveRef::Custom(id) => s.serialize_str(&format!("custom:{id}")),
        }
    }
}

/// An unknown curve name reads as Linear.
impl<'de> Deserialize<'de> for CurveRef {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let value = serde_json::Value::deserialize(d)?;
        let name = value.as_str().unwrap_or("");
        Ok(match name {
            "s-curve" => CurveRef::SCurve,
            _ => name
                .strip_prefix("custom:")
                .and_then(|id| id.parse().ok())
                .map_or(CurveRef::Linear, CurveRef::Custom),
        })
    }
}

/// A user-drawn curve through (0,0), its interior points, and (1,1).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CustomCurve {
    pub id: u32,
    pub name: String,
    /// Interior points, sorted by x, each x strictly inside (0, 1).
    points: Vec<[f32; 2]>,
}

impl CustomCurve {
    pub fn new(id: u32, name: String) -> Self {
        Self {
            id,
            name,
            points: vec![[0.5, 0.5]],
        }
    }

    /// A copy of a curve read from a file, keeping only the points `insert` would accept.
    pub fn checked(&self) -> Self {
        let mut curve = Self {
            id: self.id,
            name: self.name.clone(),
            points: Vec::new(),
        };
        if curve.name.trim().is_empty() {
            curve.name = format!("Curve {}", self.id.saturating_add(1));
        }
        for &[x, y] in &self.points {
            curve.insert(x, y);
        }
        curve
    }

    pub fn points(&self) -> &[[f32; 2]] {
        &self.points
    }

    /// All points including the fixed endpoints.
    pub fn all_points(&self) -> Vec<[f32; 2]> {
        let mut all = Vec::with_capacity(self.points.len() + 2);
        all.push([0.0, 0.0]);
        all.extend_from_slice(&self.points);
        all.push([1.0, 1.0]);
        all
    }

    /// Inserts a point, keeping x order. Returns its index, or `None` if it would
    /// sit too close to an existing point.
    pub fn insert(&mut self, x: f32, y: f32) -> Option<usize> {
        if x < MIN_GAP || x > 1.0 - MIN_GAP {
            return None;
        }
        if self.points.iter().any(|p| (p[0] - x).abs() < MIN_GAP) {
            return None;
        }
        let i = self.points.partition_point(|p| p[0] < x);
        self.points
            .insert(i, [x, y.clamp(*Y_RANGE.start(), *Y_RANGE.end())]);
        Some(i)
    }

    pub fn remove(&mut self, i: usize) {
        if i < self.points.len() {
            self.points.remove(i);
        }
    }

    /// Moves point `i`, keeping it between its neighbours and inside the y range.
    pub fn move_point(&mut self, i: usize, x: f32, y: f32) {
        if i >= self.points.len() {
            return;
        }
        let lo = if i == 0 { 0.0 } else { self.points[i - 1][0] } + MIN_GAP;
        let hi = if i + 1 == self.points.len() {
            1.0
        } else {
            self.points[i + 1][0]
        } - MIN_GAP;
        self.points[i] = [
            x.clamp(lo, hi.max(lo)),
            y.clamp(*Y_RANGE.start(), *Y_RANGE.end()),
        ];
    }

    pub fn eval(&self, x: f32) -> f32 {
        monotone_cubic(&self.all_points(), x)
    }
}

/// Built-in curves plus the user's curves.
#[derive(Clone, Debug, Default)]
pub struct CurveLibrary {
    pub custom: Vec<CustomCurve>,
    next_id: u32,
}

impl CurveLibrary {
    /// A library holding `curves` (from a file): at most [`MAX_CUSTOM`], each id once,
    /// each curve checked.
    pub fn from_curves(curves: &[CustomCurve]) -> Self {
        let mut library = Self::default();
        for curve in curves {
            if library.custom.len() < MAX_CUSTOM && library.get(curve.id).is_none() {
                library.custom.push(curve.checked());
            }
        }
        library.next_id = library
            .custom
            .iter()
            .map(|c| c.id.saturating_add(1))
            .max()
            .unwrap_or(0);
        library
    }

    /// `curve`, or Linear if it names a user curve that isn't in the library.
    pub fn resolve(&self, curve: CurveRef) -> CurveRef {
        match curve {
            CurveRef::Custom(id) if self.get(id).is_none() => CurveRef::Linear,
            _ => curve,
        }
    }

    /// Adds a new user curve. Returns its id, or `None` when the library is full.
    pub fn add(&mut self) -> Option<u32> {
        if self.custom.len() >= MAX_CUSTOM || self.next_id == u32::MAX {
            return None;
        }
        let id = self.next_id;
        self.next_id += 1;
        self.custom.push(CustomCurve::new(
            id,
            format!("Curve {}", id.saturating_add(1)),
        ));
        Some(id)
    }

    pub fn remove(&mut self, id: u32) {
        self.custom.retain(|c| c.id != id);
    }

    pub fn get(&self, id: u32) -> Option<&CustomCurve> {
        self.custom.iter().find(|c| c.id == id)
    }

    pub fn get_mut(&mut self, id: u32) -> Option<&mut CustomCurve> {
        self.custom.iter_mut().find(|c| c.id == id)
    }

    /// Curve value at progress `x` in [0, 1]. A deleted user curve falls back to Linear.
    pub fn eval(&self, curve: CurveRef, x: f32) -> f32 {
        let x = x.clamp(0.0, 1.0);
        match curve {
            CurveRef::Linear => x,
            CurveRef::SCurve => 0.5 - 0.5 * (std::f32::consts::PI * x).cos(),
            CurveRef::Custom(id) => self.get(id).map_or(x, |c| c.eval(x)),
        }
    }

    pub fn name(&self, curve: CurveRef) -> String {
        match curve {
            CurveRef::Linear => "Linear".into(),
            CurveRef::SCurve => "S-curve".into(),
            CurveRef::Custom(id) => self
                .get(id)
                .map_or_else(|| "Linear (deleted curve)".into(), |c| c.name.clone()),
        }
    }

    /// Every selectable curve, built-ins first.
    pub fn choices(&self) -> Vec<CurveRef> {
        let mut all = vec![CurveRef::Linear, CurveRef::SCurve];
        all.extend(self.custom.iter().map(|c| CurveRef::Custom(c.id)));
        all
    }
}

/// Monotone cubic Hermite interpolation (Fritsch–Carlson) through `pts`, sorted by x.
/// It never overshoots between neighbouring points.
pub fn monotone_cubic(pts: &[[f32; 2]], x: f32) -> f32 {
    let n = pts.len();
    if n < 2 {
        return x;
    }
    let d: Vec<f32> = pts
        .windows(2)
        .map(|w| (w[1][1] - w[0][1]) / (w[1][0] - w[0][0]))
        .collect();
    let mut m = vec![0.0f32; n];
    m[0] = d[0];
    m[n - 1] = d[n - 2];
    for k in 1..n - 1 {
        m[k] = if d[k - 1] * d[k] <= 0.0 {
            0.0
        } else {
            (d[k - 1] + d[k]) / 2.0
        };
    }
    for k in 0..n - 1 {
        if d[k] == 0.0 {
            m[k] = 0.0;
            m[k + 1] = 0.0;
        } else {
            let a = m[k] / d[k];
            let b = m[k + 1] / d[k];
            let s = a * a + b * b;
            if s > 9.0 {
                let t = 3.0 / s.sqrt();
                m[k] = t * a * d[k];
                m[k + 1] = t * b * d[k];
            }
        }
    }
    let x = x.clamp(pts[0][0], pts[n - 1][0]);
    let k = pts.partition_point(|p| p[0] <= x).clamp(1, n - 1) - 1;
    let (x0, y0) = (pts[k][0], pts[k][1]);
    let (x1, y1) = (pts[k + 1][0], pts[k + 1][1]);
    let h = x1 - x0;
    let t = (x - x0) / h;
    let (t2, t3) = (t * t, t * t * t);
    (2.0 * t3 - 3.0 * t2 + 1.0) * y0
        + (t3 - 2.0 * t2 + t) * h * m[k]
        + (-2.0 * t3 + 3.0 * t2) * y1
        + (t3 - t2) * h * m[k + 1]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_curves() {
        let lib = CurveLibrary::default();
        assert_eq!(lib.eval(CurveRef::Linear, 0.25), 0.25);
        assert!((lib.eval(CurveRef::SCurve, 0.5) - 0.5).abs() < 1e-6);
        assert!(lib.eval(CurveRef::SCurve, 0.1) < 0.1);
        assert!(lib.eval(CurveRef::SCurve, 0.9) > 0.9);
    }

    #[test]
    fn endpoints_are_exact() {
        let mut lib = CurveLibrary::default();
        let id = lib.add().unwrap();
        lib.get_mut(id).unwrap().move_point(0, 0.3, 0.9);
        for curve in lib.choices() {
            assert_eq!(lib.eval(curve, 0.0), 0.0, "{curve:?}");
            assert!((lib.eval(curve, 1.0) - 1.0).abs() < 1e-6, "{curve:?}");
        }
    }

    #[test]
    fn progress_is_clamped() {
        let lib = CurveLibrary::default();
        assert_eq!(lib.eval(CurveRef::Linear, -1.0), 0.0);
        assert_eq!(lib.eval(CurveRef::Linear, 2.0), 1.0);
    }

    #[test]
    fn monotone_points_never_overshoot() {
        let pts = [[0.0, 0.0], [0.2, 0.8], [0.5, 0.9], [1.0, 1.0]];
        let mut prev = 0.0;
        for i in 0..=100 {
            let y = monotone_cubic(&pts, i as f32 / 100.0);
            assert!((0.0..=1.0 + 1e-6).contains(&y), "y = {y}");
            assert!(y >= prev - 1e-6, "not monotone at {i}");
            prev = y;
        }
    }

    #[test]
    fn points_outside_unit_range_overshoot() {
        let mut c = CustomCurve::new(0, "bounce".into());
        c.move_point(0, 0.7, 1.3);
        assert!((c.eval(0.7) - 1.3).abs() < 1e-5);
        assert!(c.eval(0.8) > 1.0);
    }

    #[test]
    fn points_stay_sorted_and_clamped() {
        let mut c = CustomCurve::new(0, "c".into());
        assert_eq!(c.insert(0.2, 0.1), Some(0));
        assert_eq!(c.insert(0.8, 3.0), Some(2));
        assert_eq!(c.points()[2], [0.8, 1.5]);
        assert_eq!(c.insert(0.505, 0.0), None, "too close to 0.5");
        assert_eq!(c.insert(0.0, 0.0), None, "endpoint");
        // Dragging a point past its neighbour stops short of it.
        c.move_point(1, 0.95, 0.5);
        assert!((c.points()[1][0] - (0.8 - MIN_GAP)).abs() < 1e-6);
        c.remove(0);
        assert_eq!(c.points().len(), 2);
    }

    #[test]
    fn moving_an_out_of_range_point_is_a_no_op() {
        let mut c = CustomCurve::new(0, "c".into());
        c.insert(0.5, 0.5);
        c.move_point(99, 0.5, 0.5);
        assert_eq!(c.points(), &[[0.5, 0.5]]);
    }

    #[test]
    fn deleted_curve_falls_back_to_linear() {
        let mut lib = CurveLibrary::default();
        let id = lib.add().unwrap();
        lib.remove(id);
        assert_eq!(lib.eval(CurveRef::Custom(id), 0.3), 0.3);
        assert!(lib.choices().iter().all(|c| *c != CurveRef::Custom(id)));
    }

    #[test]
    fn library_is_capped() {
        let mut lib = CurveLibrary::default();
        for _ in 0..MAX_CUSTOM {
            assert!(lib.add().is_some());
        }
        assert!(lib.add().is_none());
    }

    #[test]
    fn huge_curve_ids_do_not_overflow() {
        let huge_curve = CustomCurve::new(u32::MAX, "Huge".into());
        let checked = huge_curve.checked();
        assert_eq!(checked.id, u32::MAX);
        // Name should not panic on saturating_add
        assert!(!checked.name.is_empty());

        let lib = CurveLibrary::from_curves(std::slice::from_ref(&huge_curve));
        assert_eq!(lib.custom.len(), 1);
        assert_eq!(lib.custom[0].id, u32::MAX);

        // Cannot add more curves when next_id would be u32::MAX
        let mut lib = CurveLibrary::from_curves(&[huge_curve]);
        assert!(lib.add().is_none(), "cannot add when next_id == u32::MAX");
        assert_eq!(lib.custom.len(), 1, "library still holds the huge curve");
    }
}
