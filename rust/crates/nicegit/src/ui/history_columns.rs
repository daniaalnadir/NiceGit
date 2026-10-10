//! The commit history's columns: which are shown and how wide each is, and the header row that
//! resizes them by dragging and shows or hides them from its context menu.

use std::collections::{BTreeMap, BTreeSet};

use egui::{Rangef, Sense, Ui};
use serde::{Deserialize, Serialize};

use crate::theme;
use crate::tools::widgets;

/// The history's columns, in the order they appear.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Column {
    Refs,
    Graph,
    Message,
    Author,
    Date,
    Id,
}

/// The narrowest the commit message is allowed to become before columns on its right are left
/// out to make room.
pub const MIN_MESSAGE: f32 = 160.0;
/// The widest a column can be dragged.
pub const MAX_WIDTH: f32 = 600.0;
/// How far either side of a column edge the pointer can grab it.
const GRAB: f32 = 4.0;

impl Column {
    pub const ALL: [Column; 6] = [Column::Refs, Column::Graph, Column::Message, Column::Author, Column::Date, Column::Id];

    pub fn title(self) -> &'static str {
        match self {
            Column::Refs => "Branch / tag",
            Column::Graph => "Graph",
            Column::Message => "Commit message",
            Column::Author => "Author",
            Column::Date => "Date",
            Column::Id => "Commit",
        }
    }

    /// Columns before the message are resized from their right edge, the rest from their left,
    /// so the edge that moves is always the one beside the message.
    fn before_message(self) -> bool {
        matches!(self, Column::Refs | Column::Graph)
    }

    fn default_width(self) -> f32 {
        match self {
            Column::Refs => 184.0,
            Column::Author => 140.0,
            Column::Date => 110.0,
            Column::Id => 76.0,
            // The graph is as wide as its lanes, and the message takes what is left.
            Column::Graph | Column::Message => 0.0,
        }
    }

    fn min_width(self) -> f32 {
        match self {
            Column::Graph => 28.0,
            Column::Id => 56.0,
            _ => 60.0,
        }
    }
}

/// Which columns the history shows and the widths the user has dragged them to.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct HistoryColumns {
    pub hidden: BTreeSet<Column>,
    /// Widths set by dragging; other columns keep their default width.
    pub widths: BTreeMap<Column, f32>,
}

impl HistoryColumns {
    /// Whether `column` is shown. The commit message always is.
    pub fn shown(&self, column: Column) -> bool {
        column == Column::Message || !self.hidden.contains(&column)
    }

    pub fn set_shown(&mut self, column: Column, shown: bool) {
        if column == Column::Message {
            return;
        }
        if shown {
            self.hidden.remove(&column);
        } else {
            self.hidden.insert(column);
        }
    }

    /// How wide `column` is; `graph` is the graph's width for its lanes.
    pub fn width(&self, column: Column, graph: f32) -> f32 {
        let default = if column == Column::Graph { graph } else { column.default_width() };
        self.widths.get(&column).copied().unwrap_or(default)
    }

    pub fn resize(&mut self, column: Column, width: f32) {
        if column != Column::Message {
            self.widths.insert(column, width.clamp(column.min_width(), MAX_WIDTH));
        }
    }

    /// How wide `column` can grow toward `wanted` from `width` across `span`: as wide as keeps it
    /// shown with the message at least `MIN_MESSAGE` wide. Growing may leave out the columns a
    /// narrow history leaves out, which return when there is room; it never leaves out the
    /// column being dragged, which would end the drag.
    pub fn widest_within(&self, column: Column, width: f32, wanted: f32, span: Rangef, graph: f32) -> f32 {
        let fits = |width: f32| {
            let mut trial = self.clone();
            trial.resize(column, width);
            let layout = trial.layout(span.min, span.max, graph);
            layout.get(column).is_some() && layout.message().span() >= MIN_MESSAGE - 0.5
        };
        if wanted <= width || fits(wanted) {
            return wanted;
        }
        if !fits(width) {
            return width;
        }
        let (mut low, mut high) = (width, wanted);
        for _ in 0..24 {
            let middle = (low + high) / 2.0;
            if fits(middle) {
                low = middle;
            } else {
                high = middle;
            }
        }
        low
    }

    /// Lays the shown columns out from `left` to `right`. When the message would be narrower
    /// than `MIN_MESSAGE`, columns on its right are left out, Author first, then Date, then the
    /// ID, without changing which are chosen.
    pub fn layout(&self, left: f32, right: f32, graph: f32) -> Layout {
        let mut spans = Vec::new();
        let mut x = left;
        for column in [Column::Refs, Column::Graph] {
            if self.shown(column) {
                let width = self.width(column, graph);
                spans.push((column, Rangef::new(x, x + width)));
                x += width;
            }
        }
        let mut trailing: Vec<Column> = [Column::Author, Column::Date, Column::Id].into_iter().filter(|&c| self.shown(c)).collect();
        let room = |trailing: &[Column]| right - x - trailing.iter().map(|&c| self.width(c, graph)).sum::<f32>();
        while !trailing.is_empty() && room(&trailing) < MIN_MESSAGE {
            trailing.remove(0);
        }
        let mut end = right;
        let mut after = Vec::new();
        for &column in trailing.iter().rev() {
            let width = self.width(column, graph);
            after.push((column, Rangef::new(end - width, end)));
            end -= width;
        }
        spans.push((Column::Message, Rangef::new(x, end.max(x))));
        spans.extend(after.into_iter().rev());
        Layout { spans }
    }
}

/// Where each shown column lies across a row.
#[derive(Clone, Debug, PartialEq)]
pub struct Layout {
    spans: Vec<(Column, Rangef)>,
}

impl Layout {
    pub fn get(&self, column: Column) -> Option<Rangef> {
        self.spans.iter().find(|(c, _)| *c == column).map(|(_, span)| *span)
    }

    pub fn message(&self) -> Rangef {
        self.get(Column::Message).expect("the message is always laid out")
    }

    pub fn columns(&self) -> impl Iterator<Item = (Column, Rangef)> + '_ {
        self.spans.iter().copied()
    }

    /// The columns after the message, from its right edge to the row's, if any are shown.
    pub fn trailing(&self) -> Option<Rangef> {
        let message = self.message();
        let ends: Vec<Rangef> = self.spans.iter().filter(|(_, span)| span.min >= message.max).map(|(_, span)| *span).collect();
        Some(Rangef::new(ends.first()?.min, ends.last()?.max))
    }
}

/// Draws the header row: each shown column's title, an edge to drag beside each column but the
/// message (double-click restores its width), and a menu on right-click to choose the columns.
pub fn header(ui: &mut Ui, columns: &mut HistoryColumns, graph: f32, row_height: f32) {
    let c = theme::of(ui);
    let (rect, response) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 22.0), Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Other, true, "History columns"));
    let response = response.on_hover_text("Drag a column's edge to resize it. Right-click to choose columns.");
    let layout = columns.layout(rect.left(), rect.right(), graph);
    let font = egui::TextStyle::Small.resolve(ui.style());
    let y = rect.center().y;
    for (column, span) in layout.columns() {
        let cell = egui::Rect::from_x_y_ranges(span, rect.y_range());
        let painter = ui.painter().with_clip_rect(cell);
        if column == Column::Graph {
            // Over the first lane, where the graph's line starts.
            let graph_rect = egui::Rect::from_min_size(cell.min, egui::vec2(span.span(), row_height));
            let x = crate::graph_view::lane_x(graph_rect, 0).max(cell.left() + 14.0);
            painter.text(egui::pos2(x, y), egui::Align2::CENTER_CENTER, column.title(), font.clone(), c.muted);
        } else {
            let inset = if column.before_message() || column == Column::Message { 10.0 } else { 8.0 };
            painter.text(egui::pos2(span.min + inset, y), egui::Align2::LEFT_CENTER, column.title(), font.clone(), c.muted);
        }
    }

    for (column, span) in layout.columns() {
        if column == Column::Message {
            continue;
        }
        let edge = if column.before_message() { span.max } else { span.min };
        let grab = egui::Rect::from_x_y_ranges(Rangef::new(edge - GRAB, edge + GRAB), rect.y_range());
        let handle = ui.interact(grab, ui.id().with(("history column edge", column)), Sense::click_and_drag());
        handle.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Other, true, format!("Resize the {} column", column.title())));
        let active = handle.hovered() || handle.dragged();
        if active {
            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
        }
        // The width follows the pointer from where it was pressed, so the edge stays under it,
        // including the few points moved before the press counts as a drag.
        let start = handle.id.with("width at press");
        if handle.drag_started() {
            ui.data_mut(|d| d.insert_temp(start, columns.width(column, graph)));
        }
        if handle.dragged() {
            let at_press = ui.data(|d| d.get_temp::<f32>(start));
            let pointer = ui.input(|i| i.pointer.press_origin().zip(i.pointer.interact_pos()));
            if let (Some(at_press), Some((origin, now))) = (at_press, pointer) {
                let moved = now.x - origin.x;
                let wanted = if column.before_message() { at_press + moved } else { at_press - moved };
                let width = columns.width(column, graph);
                columns.resize(column, columns.widest_within(column, width, wanted, rect.x_range(), graph));
            }
        }
        if handle.double_clicked() {
            columns.widths.remove(&column);
        }
        let (color, inset) = if active { (c.accent, 2.0) } else { (c.border, 5.0) };
        ui.painter().vline(edge, rect.shrink2(egui::vec2(0.0, inset)).y_range(), egui::Stroke::new(1.0, color));
    }

    response.context_menu(|ui| {
        ui.label(egui::RichText::new("Columns").small().color(c.muted));
        for column in Column::ALL {
            let mut shown = columns.shown(column);
            let toggle = widgets::checkbox(ui, column != Column::Message, &mut shown, column.title());
            if toggle.changed() {
                columns.set_shown(column, shown);
            }
        }
        ui.separator();
        if ui.add_enabled(!columns.widths.is_empty(), egui::Button::new("Restore column widths")).clicked() {
            columns.widths.clear();
            ui.close();
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn widths(layout: &Layout) -> Vec<(Column, f32, f32)> {
        layout.columns().map(|(c, span)| (c, span.min, span.max)).collect()
    }

    #[test]
    fn columns_sit_in_order_with_the_message_taking_what_is_left() {
        let columns = HistoryColumns::default();
        let layout = columns.layout(0.0, 1000.0, 60.0);
        assert_eq!(
            widths(&layout),
            vec![
                (Column::Refs, 0.0, 184.0),
                (Column::Graph, 184.0, 244.0),
                (Column::Message, 244.0, 674.0),
                (Column::Author, 674.0, 814.0),
                (Column::Date, 814.0, 924.0),
                (Column::Id, 924.0, 1000.0),
            ]
        );
        assert_eq!(layout.trailing(), Some(Rangef::new(674.0, 1000.0)));
    }

    #[test]
    fn a_narrow_history_leaves_out_author_then_date_then_the_id() {
        let columns = HistoryColumns::default();
        // 244 before the message, 160 for it: room for the ID only.
        let layout = columns.layout(0.0, 500.0, 60.0);
        assert_eq!(layout.columns().map(|(c, _)| c).collect::<Vec<_>>(), vec![Column::Refs, Column::Graph, Column::Message, Column::Id]);
        // With a little more room, Date comes back before Author.
        let layout = columns.layout(0.0, 600.0, 60.0);
        assert_eq!(
            layout.columns().map(|(c, _)| c).collect::<Vec<_>>(),
            vec![Column::Refs, Column::Graph, Column::Message, Column::Date, Column::Id]
        );
        let layout = columns.layout(0.0, 300.0, 60.0);
        assert_eq!(layout.trailing(), None, "nothing fits beside the message");
        assert!(columns.hidden.is_empty(), "leaving columns out for room does not change the choice");
    }

    #[test]
    fn hidden_columns_give_their_room_to_the_message_but_the_message_stays() {
        let mut columns = HistoryColumns::default();
        columns.set_shown(Column::Refs, false);
        columns.set_shown(Column::Date, false);
        columns.set_shown(Column::Message, false);
        let layout = columns.layout(0.0, 1000.0, 60.0);
        assert_eq!(
            widths(&layout),
            vec![(Column::Graph, 0.0, 60.0), (Column::Message, 60.0, 784.0), (Column::Author, 784.0, 924.0), (Column::Id, 924.0, 1000.0)]
        );
        columns.set_shown(Column::Refs, true);
        assert!(columns.shown(Column::Refs));
    }

    #[test]
    fn a_growing_column_stops_before_it_squeezes_the_message_or_itself_out() {
        let columns = HistoryColumns::default();
        let span = Rangef::new(0.0, 780.0);
        // The ID grows past the room Author took; Author is left out, then the ID stops where
        // the message would fall under its minimum.
        let id = columns.widest_within(Column::Id, 76.0, 400.0, span, 44.0);
        let mut grown = columns.clone();
        grown.resize(Column::Id, id);
        let layout = grown.layout(0.0, 780.0, 44.0);
        assert!(layout.get(Column::Id).is_some() && layout.get(Column::Author).is_none());
        assert!((layout.message().span() - MIN_MESSAGE).abs() < 0.5, "{:?}", layout.message());
        // Shrinking is never limited, and a width that fits is kept as asked.
        assert_eq!(columns.widest_within(Column::Id, 76.0, 60.0, span, 44.0), 60.0);
        assert_eq!(columns.widest_within(Column::Refs, 184.0, 200.0, span, 44.0), 200.0);
    }

    #[test]
    fn resized_widths_are_kept_within_limits() {
        let mut columns = HistoryColumns::default();
        columns.resize(Column::Refs, 10.0);
        assert_eq!(columns.width(Column::Refs, 60.0), Column::Refs.min_width());
        columns.resize(Column::Author, 5000.0);
        assert_eq!(columns.width(Column::Author, 60.0), MAX_WIDTH);
        assert_eq!(columns.width(Column::Graph, 72.0), 72.0, "the graph follows its lanes until resized");
        columns.resize(Column::Graph, 100.0);
        assert_eq!(columns.width(Column::Graph, 72.0), 100.0);
        columns.resize(Column::Message, 300.0);
        assert!(!columns.widths.contains_key(&Column::Message), "the message takes what is left");
    }
}
