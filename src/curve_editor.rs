//! Interactive editor for a user curve: drag points, double-click to add,
//! right-click to remove.

use egui::{Color32, Pos2, Rect, Response, Sense, Stroke, StrokeKind, Ui, Vec2, pos2};

use crate::curve::{CustomCurve, Y_RANGE};

const SIZE: Vec2 = Vec2::new(260.0, 160.0);
/// How close (in points) the pointer must be to grab a curve point.
const GRAB_RADIUS: f32 = 8.0;

/// Curve space (x in 0..1, y in the curve's y range, y up) to screen space.
pub fn to_screen(rect: Rect, p: [f32; 2]) -> Pos2 {
    let (y0, y1) = (*Y_RANGE.start(), *Y_RANGE.end());
    pos2(
        rect.left() + p[0] * rect.width(),
        rect.bottom() - (p[1] - y0) / (y1 - y0) * rect.height(),
    )
}

/// Screen space back to curve space.
pub fn from_screen(rect: Rect, pos: Pos2) -> [f32; 2] {
    let (y0, y1) = (*Y_RANGE.start(), *Y_RANGE.end());
    [
        (pos.x - rect.left()) / rect.width(),
        y0 + (rect.bottom() - pos.y) / rect.height() * (y1 - y0),
    ]
}

/// Index of the interior point nearest `pos`, if within the grab radius.
pub fn nearest_point(rect: Rect, points: &[[f32; 2]], pos: Pos2) -> Option<usize> {
    points
        .iter()
        .enumerate()
        .map(|(i, p)| (i, to_screen(rect, *p).distance(pos)))
        .filter(|(_, d)| *d <= GRAB_RADIUS)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(i, _)| i)
}

pub fn curve_editor(ui: &mut Ui, curve: &mut CustomCurve) -> Response {
    let (response, painter) = ui.allocate_painter(SIZE, Sense::click_and_drag());
    let rect = response.rect;
    let drag_id = response.id.with("dragging");

    if response.drag_started()
        && let Some(pos) = response.interact_pointer_pos()
    {
        let grabbed = nearest_point(rect, curve.points(), pos);
        ui.memory_mut(|m| m.data.insert_temp(drag_id, grabbed));
    }
    let dragging: Option<usize> = ui.memory(|m| m.data.get_temp(drag_id)).flatten();
    if let (true, Some(i), Some(pos)) = (
        response.dragged(),
        dragging,
        response.interact_pointer_pos(),
    ) {
        let [x, y] = from_screen(rect, pos);
        curve.move_point(i, x, y);
    }
    if response.drag_stopped() {
        ui.memory_mut(|m| m.data.remove_temp::<Option<usize>>(drag_id));
    }
    if response.double_clicked()
        && let Some(pos) = response.interact_pointer_pos()
    {
        let [x, y] = from_screen(rect, pos);
        curve.insert(x, y);
    }
    if response.secondary_clicked()
        && let Some(pos) = response.interact_pointer_pos()
        && let Some(i) = nearest_point(rect, curve.points(), pos)
    {
        curve.remove(i);
    }

    let visuals = ui.visuals();
    painter.rect_filled(rect, 2.0, visuals.extreme_bg_color);
    painter.rect_stroke(
        rect,
        2.0,
        visuals.widgets.noninteractive.bg_stroke,
        StrokeKind::Inside,
    );
    let guide = Stroke::new(1.0, visuals.weak_text_color());
    for y in [0.0, 1.0] {
        let a = to_screen(rect, [0.0, y]);
        let b = to_screen(rect, [1.0, y]);
        painter.line_segment([a, b], guide);
    }
    let line: Vec<Pos2> = (0..=128)
        .map(|i| {
            let x = i as f32 / 128.0;
            to_screen(rect, [x, curve.eval(x)])
        })
        .collect();
    painter.line(line, Stroke::new(2.0, Color32::from_rgb(255, 170, 60)));
    for p in curve.all_points() {
        painter.circle_filled(to_screen(rect, p), 4.0, visuals.strong_text_color());
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect() -> Rect {
        Rect::from_min_size(pos2(10.0, 20.0), SIZE)
    }

    #[test]
    fn screen_mapping_round_trips() {
        for p in [[0.0, 0.0], [1.0, 1.0], [0.3, -0.4], [0.9, 1.4]] {
            let back = from_screen(rect(), to_screen(rect(), p));
            assert!((back[0] - p[0]).abs() < 1e-4 && (back[1] - p[1]).abs() < 1e-4);
        }
    }

    #[test]
    fn y_increases_upward() {
        let low = to_screen(rect(), [0.5, 0.0]);
        let high = to_screen(rect(), [0.5, 1.0]);
        assert!(high.y < low.y);
    }

    #[test]
    fn grabs_only_nearby_points() {
        let points = [[0.25, 0.5], [0.75, 0.5]];
        let near = to_screen(rect(), points[1]) + Vec2::new(3.0, 3.0);
        let far = to_screen(rect(), [0.5, 1.4]);
        assert_eq!(nearest_point(rect(), &points, near), Some(1));
        assert_eq!(nearest_point(rect(), &points, far), None);
    }
}
