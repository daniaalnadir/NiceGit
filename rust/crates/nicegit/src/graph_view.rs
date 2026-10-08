use egui::epaint::{CubicBezierShape, PathStroke};
use egui::{Color32, FontId, Painter, Pos2, Rect, Stroke};
use nicegit_core::graph::GraphRow;

use crate::theme::graph_color as color;

pub const LANE_WIDTH: f32 = 18.0;
const LEFT_PADDING: f32 = 8.0;
pub const NODE_RADIUS: f32 = 10.0;

/// How a row's node is drawn.
pub enum Node<'a> {
    /// A commit, with its author's initials.
    Commit { initials: &'a str, merge: bool },
    /// Uncommitted work: a dashed ring.
    WorkingTree,
}

pub fn width_for(lane_count: usize) -> f32 {
    LEFT_PADDING * 2.0 + LANE_WIDTH * lane_count.saturating_sub(1) as f32 + NODE_RADIUS * 2.0
}

pub fn lane_x(rect: Rect, lane: usize) -> f32 {
    rect.left() + LEFT_PADDING + NODE_RADIUS + lane as f32 * LANE_WIDTH
}

/// Initials for an author name, such as "AP" for "Ada Park".
pub fn initials(name: &str) -> String {
    let mut letters = name.split_whitespace().filter_map(|word| word.chars().find(|c| c.is_alphanumeric()));
    let first = letters.next();
    let last = letters.next_back();
    [first, last].into_iter().flatten().flat_map(char::to_uppercase).collect()
}

/// Paints one row: lines entering from the row above, lines leaving towards the row below,
/// and the node.
pub fn paint_row(painter: &Painter, rect: Rect, row: &GraphRow, node: Node, background: Color32) {
    let (top, middle, bottom) = (rect.top(), rect.center().y, rect.bottom());
    for segment in &row.segments {
        let start = Pos2::new(lane_x(rect, segment.from_lane), if segment.starts_at_node { middle } else { top });
        let end = Pos2::new(lane_x(rect, segment.to_lane), if segment.ends_at_node { middle } else { bottom });
        let stroke = Stroke::new(2.0, color(segment.color));
        if segment.from_lane == segment.to_lane {
            painter.line_segment([start, end], stroke);
        } else {
            // Bend with vertical tangents, so lines leave and enter lanes smoothly.
            let bend = (end.y - start.y) / 2.0;
            let points = [start, Pos2::new(start.x, start.y + bend), Pos2::new(end.x, end.y - bend), end];
            painter.add(CubicBezierShape::from_points_stroke(points, false, Color32::TRANSPARENT, PathStroke::from(stroke)));
        }
    }
    let center = Pos2::new(lane_x(rect, row.lane), middle);
    let node_color = color(row.color);
    match node {
        Node::WorkingTree => {
            painter.circle_filled(center, NODE_RADIUS, background);
            // A dashed ring: short arcs around the circle.
            let dashes = 12;
            for i in 0..dashes {
                let a0 = i as f32 / dashes as f32 * std::f32::consts::TAU;
                let a1 = a0 + std::f32::consts::TAU / dashes as f32 * 0.55;
                let p0 = center + egui::vec2(a0.cos(), a0.sin()) * (NODE_RADIUS - 1.0);
                let p1 = center + egui::vec2(a1.cos(), a1.sin()) * (NODE_RADIUS - 1.0);
                painter.line_segment([p0, p1], Stroke::new(2.0, node_color));
            }
        }
        Node::Commit { merge: true, .. } => {
            painter.circle(center, 4.5, node_color, Stroke::new(2.0, background));
        }
        Node::Commit { initials, .. } => {
            painter.circle(center, NODE_RADIUS, node_color, Stroke::new(2.0, background));
            painter.text(center, egui::Align2::CENTER_CENTER, initials, FontId::proportional(8.5), Color32::WHITE);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::initials;

    #[test]
    fn initials_use_first_and_last_words() {
        assert_eq!(initials("Ada Park"), "AP");
        assert_eq!(initials("lee van chen"), "LC");
        assert_eq!(initials("Prince"), "P");
        assert_eq!(initials(""), "");
    }
}
