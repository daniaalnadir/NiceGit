//! The diff view: unified or side by side, with word highlights, a find bar, and line selection
//! for staging individual lines. Rows are virtualised, so large diffs stay responsive. Long
//! lines wrap within each side of a split diff, as in the Mac app, and scroll horizontally in
//! the unified view.

#![allow(dead_code)]

use std::collections::BTreeSet;
use std::ops::Range;

use egui::text::{LayoutJob, TextFormat};
use egui::{pos2, vec2, Align2, Color32, CursorIcon, FontId, Id, Key, Rect, RichText, ScrollArea, Sense, Stroke, TextEdit, TextStyle, Ui};
use egui_phosphor::regular as icon;
use nicegit_core::diff::{side_by_side, DiffLine, DiffLineKind, SplitRow};
use nicegit_core::inline::{self, InlineChange};

use crate::theme;
use crate::tools::widgets;

/// Digits reserved for each line number.
const NUMBER_COLUMNS: f32 = 5.0;
/// Space between the two sides of a split diff.
const SPLIT_GAP: f32 = 2.0;
/// Space around line numbers and text.
const PADDING: f32 = 6.0;
/// Width of the bar that marks a selected line.
const SELECTION_BAR: f32 = 3.0;
/// Room kept free for the vertical scroll bar, so the horizontal extent does not overflow.
const SCROLLBAR_ALLOWANCE: f32 = 14.0;
/// Columns a tab is drawn as.
const TAB_COLUMNS: usize = 4;
/// The fewest columns of text a side of a split diff keeps before the view scrolls instead.
const MIN_WRAP_COLUMNS: usize = 20;

/// A diff prepared for display: its lines, their side-by-side arrangement, and the parts of
/// paired lines that changed.
pub struct DiffContent {
    pub title: String,
    pub lines: Vec<DiffLine>,
    pub split: Vec<SplitRow>,
    /// Changed words within each paired deletion and addition, keyed by line index.
    highlights: std::collections::BTreeMap<usize, InlineChange>,
    /// The row each line is drawn in, when shown side by side.
    split_row_of_line: Vec<usize>,
    /// The widest line, in columns.
    max_columns: usize,
    /// Each line's width in columns, for working out how many rows it wraps onto.
    line_columns: Vec<usize>,
}

impl DiffContent {
    pub fn new(title: String, lines: Vec<DiffLine>) -> Self {
        let split = side_by_side(&lines);
        let mut split_row_of_line = vec![0; lines.len()];
        for (row, split_row) in split.iter().enumerate() {
            match *split_row {
                SplitRow::Banner(index) => split_row_of_line[index] = row,
                SplitRow::Pair { left, right } => {
                    for index in [left, right].into_iter().flatten() {
                        split_row_of_line[index] = row;
                    }
                }
            }
        }
        let line_columns: Vec<usize> = lines.iter().map(|line| columns(&line.text)).collect();
        let max_columns = line_columns.iter().copied().max().unwrap_or(0);
        let highlights = inline::highlights(&lines);
        Self { title, lines, split, highlights, split_row_of_line, max_columns, line_columns }
    }

    pub fn is_empty(&self) -> bool {
        !self.lines.iter().any(|line| matches!(line.kind, DiffLineKind::Addition | DiffLineKind::Deletion | DiffLineKind::Context))
    }

    fn row_of(&self, line: usize, split: bool) -> usize {
        if split {
            self.split_row_of_line.get(line).copied().unwrap_or(0)
        } else {
            line
        }
    }
}

fn columns(text: &str) -> usize {
    text.chars().map(|c| if c == '\t' { TAB_COLUMNS } else { 1 }).sum()
}

/// The find bar's state. The caller keeps it between frames.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DiffFind {
    pub query: String,
    /// Which match is current, counted from the first match in the diff.
    pub current: usize,
}

/// How the diff is shown, and what the user has selected. The caller keeps it between frames.
#[derive(Clone, Debug, Default)]
pub struct DiffOptions {
    pub split: bool,
    pub find: DiffFind,
    /// Whether added and removed lines can be selected, for staging individual lines.
    pub selectable: bool,
    /// Selected added and removed lines, as indices into `DiffContent::lines`. Clicking a line
    /// toggles it. Dragging sets every line the pointer crosses to the state of the line where
    /// the drag began.
    pub selected: BTreeSet<usize>,
}

/// What happened in the diff view this frame.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DiffResponse {
    /// The added or removed line the pointer pressed on, if any.
    pub clicked: Option<usize>,
    /// Whether `DiffOptions::selected` changed this frame.
    pub selection_changed: bool,
    /// How many places the find query matches.
    pub match_count: usize,
    /// The line holding the current match, if there is one.
    pub current_match_line: Option<usize>,
}

/// Shows the diff without a find bar, keeping no state. Use [`show_with`] for the full view.
pub fn show(ui: &mut Ui, content: &DiffContent, split: bool) {
    let mut options = DiffOptions { split, ..Default::default() };
    show_inner(ui, content, &mut options, false);
}

/// Shows the diff with its find bar, and lets the user select lines when `options.selectable`
/// is set. Returns what the user did, so the caller can act on it.
pub fn show_with(ui: &mut Ui, content: &DiffContent, options: &mut DiffOptions) -> DiffResponse {
    show_inner(ui, content, options, true)
}

fn show_inner(ui: &mut Ui, content: &DiffContent, options: &mut DiffOptions, find_bar: bool) -> DiffResponse {
    let mut response = DiffResponse::default();
    if content.lines.is_empty() || content.is_empty() {
        ui.centered_and_justified(|ui| ui.label(RichText::new("No changes to show").weak()));
        return response;
    }
    let drag_id = Id::new(("diff-selection-drag", content.title.as_str()));
    if !ui.input(|input| input.pointer.primary_down()) {
        ui.ctx().data_mut(|data| data.remove::<Option<bool>>(drag_id));
    }

    let (occurrences, scroll_to) = if find_bar { find_bar_row(ui, content, options) } else { (Vec::new(), None) };
    response.match_count = occurrences.len();
    response.current_match_line = occurrences.get(options.find.current).map(|occurrence| occurrence.line);

    // A new or deleted file has only one side; showing it beside an empty column would halve
    // the room its lines have, so it is shown in one column.
    let one_sided = !content.lines.iter().any(|line| line.kind == DiffLineKind::Deletion)
        || !content.lines.iter().any(|line| line.kind == DiffLineKind::Addition);
    let split = options.split && !one_sided;
    let style = Style::new(ui, content, split);
    let rows = if split { content.split.len() } else { content.lines.len() };
    // Where each row starts. Unified rows are one line high; split rows as high as their
    // longer side wraps.
    let tops: Vec<f32> = if split {
        let mut tops = Vec::with_capacity(rows + 1);
        let mut top = 0.0;
        for row in &content.split {
            tops.push(top);
            top += style.split_row_height(content, row);
        }
        tops.push(top);
        tops
    } else {
        Vec::new()
    };
    let mut area = ScrollArea::both().id_salt(("diff", content.title.as_str(), split)).auto_shrink([false, false]);
    if let Some(line) = scroll_to {
        // Scroll so the match sits a little above the middle of the viewport.
        let row = content.row_of(line, split);
        let top = if split { tops[row] } else { row as f32 * style.row_height };
        area = area.vertical_scroll_offset((top - ui.available_height() * 0.4).max(0.0));
    }
    let view = View { content, occurrences: &occurrences, current: options.find.current, split: split, style, drag_id };
    ui.scope(|ui| {
        // Rows are placed at computed heights, so no spacing may sit between them.
        ui.spacing_mut().item_spacing.y = 0.0;
        if split {
            area.show_viewport(ui, |ui, viewport| {
                let origin = ui.max_rect().min;
                let height = tops[rows];
                ui.allocate_rect(Rect::from_min_size(origin, vec2(view.style.total, height)), Sense::hover());
                // The rows that overlap the visible part of the diff.
                let first = tops.partition_point(|top| *top <= viewport.min.y).saturating_sub(1);
                let last = tops.partition_point(|top| *top < viewport.max.y).min(rows);
                for row in first..last {
                    let rect = Rect::from_min_size(origin + vec2(0.0, tops[row]), vec2(view.style.total, tops[row + 1] - tops[row]));
                    draw_split_row(ui, row, rect, &view, options, &mut response);
                }
            });
        } else {
            area.show_rows(ui, view.style.row_height, rows, |ui, range| {
                draw_rows(ui, range, &view, options, &mut response);
            });
        }
    });
    response
}

/// Sizes and colours that stay the same for every row of one frame.
struct Style {
    font: FontId,
    colors: theme::Colors,
    text: Color32,
    muted: Color32,
    /// Width of one line-number column.
    gutter: f32,
    row_height: f32,
    /// Width of the whole diff, at least the width of its widest line.
    total: f32,
    /// Width of one side of a split diff.
    half: f32,
    /// In a split diff, the columns of text that fit on one side before a line wraps.
    wrap_columns: Option<usize>,
    /// Width of the text on one side of a split diff, where its lines wrap.
    wrap_width: f32,
}

impl Style {
    fn new(ui: &Ui, content: &DiffContent, split: bool) -> Self {
        let font = TextStyle::Monospace.resolve(ui.style());
        let char_width = ui.ctx().fonts_mut(|fonts| fonts.glyph_width(&font, '0'));
        let gutter = NUMBER_COLUMNS * char_width + PADDING;
        let text_width = content.max_columns as f32 * char_width + 2.0 * PADDING;
        let available = (ui.available_width() - SCROLLBAR_ALLOWANCE).max(0.0);
        let (total, half) = if split {
            // Each side takes half the width and its long lines wrap, down to a narrow minimum.
            let half = ((available - SPLIT_GAP) / 2.0).max(gutter + MIN_WRAP_COLUMNS as f32 * char_width + 2.0 * PADDING);
            (2.0 * half + SPLIT_GAP, half)
        } else {
            let total = available.max(2.0 * gutter + text_width);
            (total, total)
        };
        let wrap_width = (half - gutter - 2.0 * PADDING).max(char_width);
        let wrap_columns = split.then(|| ((wrap_width / char_width).floor() as usize).max(1));
        Self {
            font,
            colors: theme::of(ui),
            text: ui.visuals().text_color(),
            muted: ui.visuals().weak_text_color(),
            gutter,
            row_height: ui.text_style_height(&TextStyle::Monospace) + 2.0,
            total,
            half,
            wrap_columns,
            wrap_width,
        }
    }

    /// How many lines of text `line` takes on one side of a split diff.
    fn wrapped_lines(&self, content: &DiffContent, line: usize) -> usize {
        match self.wrap_columns {
            Some(columns) => content.line_columns[line].div_ceil(columns).max(1),
            None => 1,
        }
    }

    /// The height of a split row: its longer side, wrapped.
    fn split_row_height(&self, content: &DiffContent, row: &SplitRow) -> f32 {
        let lines = match *row {
            SplitRow::Banner(_) => 1,
            SplitRow::Pair { left, right } => {
                [left, right].into_iter().flatten().map(|line| self.wrapped_lines(content, line)).max().unwrap_or(1)
            }
        };
        lines as f32 * (self.row_height - 2.0) + 2.0
    }
}

/// Everything a row needs to be drawn.
struct View<'a> {
    content: &'a DiffContent,
    occurrences: &'a [Occurrence],
    current: usize,
    split: bool,
    style: Style,
    drag_id: Id,
}

impl View<'_> {
    /// Colours and ranges for the parts of a line that stand out: changed words, then matches.
    /// Later ranges paint over earlier ones.
    fn spans(&self, line: usize) -> Vec<(Range<usize>, Color32)> {
        let colors = self.style.colors;
        let mut spans = Vec::new();
        if let Some(change) = self.content.highlights.get(&line) {
            if !change.changed.is_empty() {
                // The sign before the content is one byte, so the changed text starts after it.
                let start = 1 + change.prefix.len();
                let word =
                    if self.content.lines[line].kind == DiffLineKind::Deletion { colors.removed_word_bg } else { colors.added_word_bg };
                spans.push((start..start + change.changed.len(), word));
            }
        }
        let first = self.occurrences.partition_point(|occurrence| occurrence.line < line);
        for (offset, occurrence) in self.occurrences[first..].iter().take_while(|occurrence| occurrence.line == line).enumerate() {
            let strong = first + offset == self.current;
            let color = tint(colors.warning, if strong { 170 } else { 70 });
            spans.push((occurrence.range.clone(), color));
        }
        spans
    }

    /// Paints one line into `rect`, with a number column for each entry in `numbers`.
    fn paint_line(&self, ui: &Ui, rect: Rect, line: usize, numbers: &[Option<usize>], selected: bool, wrap: bool) {
        let painter = ui.painter().with_clip_rect(rect);
        let colors = self.style.colors;
        let diff_line = &self.content.lines[line];
        let background = match diff_line.kind {
            DiffLineKind::Addition => Some(colors.added_bg),
            DiffLineKind::Deletion => Some(colors.removed_bg),
            DiffLineKind::Hunk => Some(tint(colors.modified, 26)),
            _ => None,
        };
        if let Some(background) = background {
            painter.rect_filled(rect, 0.0, background);
        }
        if selected {
            painter.rect_filled(rect, 0.0, tint(colors.accent, 48));
            painter.rect_filled(Rect::from_min_size(rect.min, vec2(SELECTION_BAR, rect.height())), 0.0, colors.accent);
        }
        // Numbers sit beside the first line of text, which is centred in a single-line row.
        let first_line_center = rect.top() + self.style.row_height / 2.0;
        for (column, number) in numbers.iter().enumerate() {
            if let Some(number) = number {
                let x = rect.left() + (column + 1) as f32 * self.style.gutter - PADDING / 2.0;
                painter.text(
                    pos2(x, first_line_center),
                    Align2::RIGHT_CENTER,
                    number.to_string(),
                    self.style.font.clone(),
                    self.style.muted,
                );
            }
        }
        let text_color = match diff_line.kind {
            DiffLineKind::Hunk => colors.modified,
            DiffLineKind::Metadata => self.style.muted,
            _ => self.style.text,
        };
        let mut job = layout(&diff_line.text, &self.style.font, text_color, &self.spans(line));
        if wrap {
            // Code wraps at any character, so the rows taken match the column count.
            job.wrap.max_width = self.style.wrap_width;
            job.wrap.break_anywhere = true;
        }
        let galley = painter.layout_job(job);
        let x = rect.left() + numbers.len() as f32 * self.style.gutter + PADDING;
        let y = first_line_center - (self.style.row_height - 2.0) / 2.0;
        painter.galley(pos2(x, y), galley, text_color);
    }

    /// Paints an empty half of a split row, where the other side has no counterpart.
    fn paint_empty(&self, ui: &Ui, rect: Rect) {
        ui.painter().rect_filled(rect, 0.0, self.style.colors.subtle_bg);
    }
}

/// Draws the unified rows in `range`, handling pointer selection on changed lines.
fn draw_rows(ui: &mut Ui, range: Range<usize>, view: &View, options: &mut DiffOptions, response: &mut DiffResponse) {
    let height = view.style.row_height;
    for line in range {
        let row_rect = allocate_row(ui, view.style.total, height);
        let selected = select_by_pointer(ui, view, row_rect, line, options, response);
        let diff_line = &view.content.lines[line];
        let numbers = [diff_line.old_number, diff_line.new_number];
        view.paint_line(ui, row_rect, line, &numbers, selected, false);
    }
}

/// Draws one row of a split diff into `row_rect`, each side wrapping its long lines.
fn draw_split_row(ui: &mut Ui, row: usize, row_rect: Rect, view: &View, options: &mut DiffOptions, response: &mut DiffResponse) {
    match view.content.split[row] {
        SplitRow::Banner(line) => view.paint_line(ui, row_rect, line, &[], false, false),
        SplitRow::Pair { left, right } => {
            let height = row_rect.height();
            let left_rect = Rect::from_min_size(row_rect.min, vec2(view.style.half, height));
            let right_rect =
                Rect::from_min_size(pos2(row_rect.left() + view.style.half + SPLIT_GAP, row_rect.top()), vec2(view.style.half, height));
            match left {
                Some(line) => {
                    let selected = select_by_pointer(ui, view, left_rect, line, options, response);
                    let number = view.content.lines[line].old_number;
                    view.paint_line(ui, left_rect, line, &[number], selected, true);
                }
                None => view.paint_empty(ui, left_rect),
            }
            match right {
                Some(line) => {
                    let selected = select_by_pointer(ui, view, right_rect, line, options, response);
                    let number = view.content.lines[line].new_number;
                    view.paint_line(ui, right_rect, line, &[number], selected, true);
                }
                None => view.paint_empty(ui, right_rect),
            }
            let divider = row_rect.left() + view.style.half + SPLIT_GAP / 2.0;
            ui.painter().vline(divider, row_rect.y_range(), Stroke::new(1.0, view.style.colors.border));
        }
    }
}

fn allocate_row(ui: &mut Ui, width: f32, height: f32) -> Rect {
    let (rect, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
    rect
}

/// Applies a press or drag on one line to the selection, and reports whether the line is now
/// selected. Only added and removed lines can be selected.
fn select_by_pointer(ui: &Ui, view: &View, rect: Rect, line: usize, options: &mut DiffOptions, response: &mut DiffResponse) -> bool {
    let is_change = matches!(view.content.lines[line].kind, DiffLineKind::Addition | DiffLineKind::Deletion);
    if !options.selectable || !is_change {
        return false;
    }
    let (pressed, down, pointer) =
        ui.input(|input| (input.pointer.primary_pressed(), input.pointer.primary_down(), input.pointer.interact_pos()));
    let inside = pointer.is_some_and(|position| rect.contains(position));
    if inside {
        ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
    }
    let drag: Option<bool> = ui.ctx().data(|data| data.get_temp::<Option<bool>>(view.drag_id)).flatten();
    if let Some(mode) = drag {
        if down && inside {
            response.selection_changed |= set_selected(options, line, mode);
        }
    } else if pressed && inside {
        // A press decides what the whole drag will do: select if the line starts unselected.
        let mode = !options.selected.contains(&line);
        response.selection_changed |= set_selected(options, line, mode);
        response.clicked = Some(line);
        ui.ctx().data_mut(|data| data.insert_temp(view.drag_id, Some(mode)));
    }
    options.selected.contains(&line)
}

fn set_selected(options: &mut DiffOptions, line: usize, selected: bool) -> bool {
    if selected {
        options.selected.insert(line)
    } else {
        options.selected.remove(&line)
    }
}

/// Draws the find bar and handles its controls. Returns the matches for this frame and the line
/// to scroll to, if navigation moved.
fn find_bar_row(ui: &mut Ui, content: &DiffContent, options: &mut DiffOptions) -> (Vec<Occurrence>, Option<usize>) {
    let mut occurrences = find_occurrences(&content.lines, &options.find.query);
    let mut scroll_to = None;
    let muted = theme::of(ui).muted;
    ui.horizontal(|ui| {
        ui.label(RichText::new(icon::MAGNIFYING_GLASS).color(muted));
        let edit = ui.add(TextEdit::singleline(&mut options.find.query).hint_text("Find in diff").desired_width(180.0));
        edit.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, "Find in diff"));
        // Command-F (Ctrl-F elsewhere) jumps to the search field, as in the Mac app.
        if ui.input_mut(|i| i.consume_shortcut(&egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::F))) {
            edit.request_focus();
        }
        if edit.changed() {
            occurrences = find_occurrences(&content.lines, &options.find.query);
            options.find.current = 0;
            scroll_to = occurrences.first().map(|occurrence| occurrence.line);
        }
        let count = occurrences.len();
        if !options.find.query.is_empty() {
            let label =
                if count == 0 { "No matches".to_string() } else { format!("{} of {count}", options.find.current.min(count - 1) + 1) };
            ui.label(RichText::new(label).color(muted));
        }
        let previous = widgets::icon_button(ui, icon::CARET_UP, "Previous match (Shift+Enter)", count > 0).clicked();
        let next = widgets::icon_button(ui, icon::CARET_DOWN, "Next match (Enter)", count > 0).clicked();
        let enter = (edit.has_focus() || edit.lost_focus()) && ui.input(|input| input.key_pressed(Key::Enter));
        let backwards = previous || (enter && ui.input(|input| input.modifiers.shift));
        if (next || previous || enter) && count > 0 {
            let current = options.find.current.min(count - 1) as isize;
            let step = if backwards { -1 } else { 1 };
            let target = (current + step).rem_euclid(count as isize) as usize;
            options.find.current = target;
            scroll_to = Some(occurrences[target].line);
        }
    });
    (occurrences, scroll_to)
}

/// A case-insensitive match of the find query within one line.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Occurrence {
    line: usize,
    range: Range<usize>,
}

fn find_occurrences(lines: &[DiffLine], query: &str) -> Vec<Occurrence> {
    if query.is_empty() {
        return Vec::new();
    }
    let mut found = Vec::new();
    for (line, diff_line) in lines.iter().enumerate() {
        for range in matches_in(&diff_line.text, query) {
            found.push(Occurrence { line, range });
        }
    }
    found
}

/// Byte ranges of the non-overlapping, case-insensitive matches of `query` in `text`.
fn matches_in(text: &str, query: &str) -> Vec<Range<usize>> {
    let needle: Vec<char> = query.chars().flat_map(char::to_lowercase).collect();
    if needle.is_empty() {
        return Vec::new();
    }
    // Each lowercased character, with the bytes of the original character it came from.
    let mut folded: Vec<(usize, usize, char)> = Vec::new();
    for (start, ch) in text.char_indices() {
        let end = start + ch.len_utf8();
        folded.extend(ch.to_lowercase().map(|lower| (start, end, lower)));
    }
    let mut found = Vec::new();
    let mut index = 0;
    while index + needle.len() <= folded.len() {
        let matches = folded[index..index + needle.len()].iter().map(|(_, _, c)| *c).eq(needle.iter().copied());
        if matches {
            found.push(folded[index].0..folded[index + needle.len() - 1].1);
            index += needle.len();
        } else {
            index += 1;
        }
    }
    found
}

/// A text layout for one line: tabs drawn as spaces, and a background behind each span.
fn layout(text: &str, font: &FontId, color: Color32, spans: &[(Range<usize>, Color32)]) -> LayoutJob {
    let mut job = LayoutJob::default();
    job.wrap.max_width = f32::INFINITY;
    let mut run = String::new();
    let mut run_background: Option<Color32> = None;
    for (offset, ch) in text.char_indices() {
        let background = spans.iter().rev().find(|(range, _)| range.contains(&offset)).map(|(_, color)| *color);
        if background != run_background {
            append_run(&mut job, &mut run, run_background, font, color);
            run_background = background;
        }
        if ch == '\t' {
            run.push_str(&" ".repeat(TAB_COLUMNS));
        } else {
            run.push(ch);
        }
    }
    append_run(&mut job, &mut run, run_background, font, color);
    job
}

fn append_run(job: &mut LayoutJob, run: &mut String, background: Option<Color32>, font: &FontId, color: Color32) {
    if run.is_empty() {
        return;
    }
    let format = TextFormat { font_id: font.clone(), color, background: background.unwrap_or(Color32::TRANSPARENT), ..Default::default() };
    job.append(run, 0.0, format);
    run.clear();
}

/// The colour with its opacity replaced by `alpha`.
fn tint(color: Color32, alpha: u8) -> Color32 {
    let [red, green, blue, _] = color.to_array();
    Color32::from_rgba_unmultiplied(red, green, blue, alpha)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_every_case_insensitive_match_without_overlap() {
        assert_eq!(matches_in("Foo foo FOO", "foo"), vec![0..3, 4..7, 8..11]);
        assert_eq!(matches_in("aaaa", "aa"), vec![0..2, 2..4]);
        assert!(matches_in("text", "").is_empty());
    }

    #[test]
    fn match_ranges_are_byte_offsets_into_multibyte_text() {
        let text = "café Café";
        let found = matches_in(text, "café");
        assert_eq!(found.len(), 2);
        assert_eq!(&text[found[1].clone()], "Café");
    }

    #[test]
    fn split_rows_are_as_tall_as_the_wrapped_text() {
        let long = format!("-{}", "wrap ".repeat(80));
        let lines = vec![
            DiffLine { text: long.clone(), kind: DiffLineKind::Deletion, old_number: Some(1), new_number: None },
            DiffLine { text: "+short".to_string(), kind: DiffLineKind::Addition, old_number: None, new_number: Some(1) },
        ];
        let content = DiffContent::new("long.txt".to_string(), lines);
        let ctx = egui::Context::default();
        let input = egui::RawInput { screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(900.0, 600.0))), ..Default::default() };
        let mut output = ctx.run_ui(input, |ui| {
            let style = Style::new(ui, &content, true);
            let row = content.split[content.row_of(0, true)];
            let wrapped = style.wrapped_lines(&content, 0);
            assert!(wrapped > 1, "a 400-column line wraps on half of a 900-point view");
            assert_eq!(style.split_row_height(&content, &row), wrapped as f32 * (style.row_height - 2.0) + 2.0);
            // egui wraps the text onto exactly the rows the height allows for.
            let mut job = layout(&long, &style.font, Color32::WHITE, &[]);
            job.wrap.max_width = style.wrap_width;
            job.wrap.break_anywhere = true;
            let galley = ui.ctx().fonts_mut(|fonts| fonts.layout_job(job));
            assert_eq!(galley.rows.len(), wrapped);
        });
        // No renderer takes the font texture here.
        output.textures_delta.clear();
    }

    #[test]
    fn tabs_count_as_several_columns() {
        assert_eq!(columns("a\tb"), 1 + TAB_COLUMNS + 1);
    }
}
