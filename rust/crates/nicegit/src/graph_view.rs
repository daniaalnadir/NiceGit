use egui::epaint::{CubicBezierShape, PathStroke};
use egui::{Color32, Painter, Pos2, Rect, Stroke};
use nicegit_core::graph::GraphRow;

pub const LANE_WIDTH: f32 = 16.0;
const LEFT_PADDING: f32 = 10.0;
const NODE_RADIUS: f32 = 4.5;

/// Lane colours, chosen to stay distinct on light and dark backgrounds. Colour 0 is the
/// checkout's own line.
pub const PALETTE: [Color32; 8] = [
    Color32::from_rgb(0x4c, 0xb8, 0x7c),
    Color32::from_rgb(0x4a, 0x90, 0xe2),
    Color32::from_rgb(0xe8, 0x8b, 0x2e),
    Color32::from_rgb(0xb0, 0x6a, 0xd9),
    Color32::from_rgb(0xe0, 0x55, 0x6b),
    Color32::from_rgb(0x2a, 0xb3, 0xc0),
    Color32::from_rgb(0xc9, 0xa2, 0x27),
    Color32::from_rgb(0x8a, 0x94, 0xa6),
];

pub fn color(index: usize) -> Color32 {
    PALETTE[index % PALETTE.len()]
}

pub fn width_for(lane_count: usize) -> f32 {
    LEFT_PADDING * 2.0 + LANE_WIDTH * lane_count.saturating_sub(1) as f32 + NODE_RADIUS * 2.0
}

fn lane_x(rect: Rect, lane: usize) -> f32 {
    rect.left() + LEFT_PADDING + NODE_RADIUS + lane as f32 * LANE_WIDTH
}

/// Paints one row: lines entering from the row above, lines leaving towards the row below,
/// and the commit's node. `hollow` marks the uncommitted-work row.
pub fn paint_row(painter: &Painter, rect: Rect, row: &GraphRow, hollow: bool, background: Color32) {
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
    if hollow {
        painter.circle(center, NODE_RADIUS, background, Stroke::new(2.0, node_color));
    } else {
        painter.circle(center, NODE_RADIUS, node_color, Stroke::new(1.5, background));
    }
}
