//! The diff view: unified or side by side, with word highlights, a find bar, and line selection
//! for staging individual lines. Rows are virtualised, so large diffs stay responsive. Long
//! lines wrap, within each side of a split diff as in the Mac app, and across the unified view,
//! so no line is cut off at the panel edge.

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

/// Rows per line, with the bits of the font size, wrap width and character width they were
/// measured for.
type WrappedRows = ((u32, u32, u32), std::sync::Arc<Vec<usize>>);

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
    /// Each line's width in columns.
    line_columns: Vec<usize>,
    /// How many rows each line wraps onto, measured with the layout the view draws with, and
    /// the font size and width they were measured for.
    wrapped_rows: std::sync::Mutex<Option<WrappedRows>>,
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
        Self { title, lines, split, highlights, split_row_of_line, max_columns, line_columns, wrapped_rows: Default::default() }
    }

    /// How many rows each line takes when wrapped at `width` for code. Lines wrap between words
    /// where they can, so this is measured from the laid-out text, once for each width.
    fn rows_at(&self, ctx: &egui::Context, font: &FontId, width: f32, char_width: f32) -> std::sync::Arc<Vec<usize>> {
        let key = (font.size.to_bits(), width.to_bits(), char_width.to_bits());
        let mut cache = self.wrapped_rows.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some((measured, rows)) = cache.as_ref() {
            if *measured == key {
                return rows.clone();
            }
        }
        let rows: Vec<usize> = ctx.fonts_mut(|fonts| {
            self.lines
                .iter()
                .map(|line| {
                    let mut job = layout(&line.text, font, Color32::WHITE, &[]);
                    job.wrap.max_width = wrap_width_of(line.kind, width, char_width);
                    fonts.layout_job(job).rows.len().max(1)
                })
                .collect()
        });
        let rows = std::sync::Arc::new(rows);
        *cache = Some((key, rows.clone()));
        rows
    }

    /// Whether only one side has lines, as for a new or deleted file; such a diff is always
    /// shown in one column, since the other would be blank.
    pub fn is_one_sided(&self) -> bool {
        // A changed file keeps unchanged lines on both sides; only an added or removed file has
        // none, and lines of one kind.
        let has = |kind: DiffLineKind| self.lines.iter().any(|line| line.kind == kind);
        !has(DiffLineKind::Context) && (!has(DiffLineKind::Deletion) || !has(DiffLineKind::Addition))
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

/// Whether a line is a header, drawn from the column where code content starts rather than from
/// the marker column, so it lines up with the code beside it.
fn is_header(kind: DiffLineKind) -> bool {
    matches!(kind, DiffLineKind::Metadata | DiffLineKind::Hunk)
}

/// How far right of the text edge a line's text starts. Code lines begin with a one-character
/// marker (' ', '+' or '-'); headers skip that marker so they start where code content starts.
fn text_offset(kind: DiffLineKind, char_width: f32) -> f32 {
    if is_header(kind) {
        char_width
    } else {
        0.0
    }
}

/// The width a line's text wraps at, given the width for code. A header starts one character
/// further right, so it has one character less room.
fn wrap_width_of(kind: DiffLineKind, width: f32, char_width: f32) -> f32 {
    (width - text_offset(kind, char_width)).max(char_width)
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

    // Beside an empty column, a new or deleted file's lines would have half the room.
    let split = options.split && !content.is_one_sided();
    let style = Style::new(ui, content, split);
    let rows = if split { content.split.len() } else { content.lines.len() };
    // Where each row starts: as high as its line wraps, or a split row's longer side.
    let mut tops = Vec::with_capacity(rows + 1);
    let mut top = 0.0;
    for row in 0..rows {
        tops.push(top);
        top += if split { style.split_row_height(&content.split[row]) } else { style.unified_row_height(row) };
    }
    tops.push(top);
    // The scroll bar stays visible, so a long diff never looks cut off at the panel's edge.
    let mut area = ScrollArea::both()
        .id_salt(("diff", content.title.as_str(), split))
        .auto_shrink([false, false])
        .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysVisible);
    if let Some(line) = scroll_to {
        // Scroll so the match sits a little above the middle of the viewport.
        let row = content.row_of(line, split);
        area = area.vertical_scroll_offset((tops[row] - ui.available_height() * 0.4).max(0.0));
    }
    let view = View { content, occurrences: &occurrences, current: options.find.current, split, style, drag_id };
    ui.scope(|ui| {
        // Rows are placed at computed heights, so no spacing may sit between them.
        ui.spacing_mut().item_spacing.y = 0.0;
        area.show_viewport(ui, |ui, viewport| {
            let origin = ui.max_rect().min;
            ui.allocate_rect(Rect::from_min_size(origin, vec2(view.style.total, tops[rows])), Sense::hover());
            // The rows that overlap the visible part of the diff.
            let first = tops.partition_point(|top| *top <= viewport.min.y).saturating_sub(1);
            let last = tops.partition_point(|top| *top < viewport.max.y).min(rows);
            for row in first..last {
                let rect = Rect::from_min_size(origin + vec2(0.0, tops[row]), vec2(view.style.total, tops[row + 1] - tops[row]));
                if split {
                    draw_split_row(ui, row, rect, &view, options, &mut response);
                } else {
                    draw_unified_row(ui, row, rect, &view, options, &mut response);
                }
            }
        });
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
    /// The columns of text that fit before a line wraps: on one side of a split diff, or
    /// across the unified view.
    wrap_columns: Option<usize>,
    /// Width of the text, where its lines wrap.
    wrap_width: f32,
    /// Width of one monospace column.
    char_width: f32,
    /// How many rows each line of the diff wraps onto at that width.
    rows: std::sync::Arc<Vec<usize>>,
}

impl Style {
    fn new(ui: &Ui, content: &DiffContent, split: bool) -> Self {
        let font = TextStyle::Monospace.resolve(ui.style());
        let char_width = ui.ctx().fonts_mut(|fonts| fonts.glyph_width(&font, '0'));
        let gutter = NUMBER_COLUMNS * char_width + PADDING;
        let available = (ui.available_width() - SCROLLBAR_ALLOWANCE).max(0.0);
        // Text wraps to the room it has, down to a narrow minimum below which the view scrolls.
        let minimum_text = MIN_WRAP_COLUMNS as f32 * char_width + 2.0 * PADDING;
        let (total, half, gutters) = if split {
            let half = ((available - SPLIT_GAP) / 2.0).max(gutter + minimum_text);
            (2.0 * half + SPLIT_GAP, half, 1.0)
        } else {
            let total = available.max(2.0 * gutter + minimum_text);
            (total, total, 2.0)
        };
        let wrap_width = (half - gutters * gutter - 2.0 * PADDING).max(char_width);
        let wrap_columns = Some(((wrap_width / char_width).floor() as usize).max(1));
        let rows = content.rows_at(ui.ctx(), &font, wrap_width, char_width);
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
            char_width,
            rows,
        }
    }

    /// How many rows of text `line` wraps onto.
    fn wrapped_lines(&self, line: usize) -> usize {
        self.rows.get(line).copied().unwrap_or(1)
    }

    /// The height of a unified row: its line, wrapped.
    fn unified_row_height(&self, line: usize) -> f32 {
        self.wrapped_lines(line) as f32 * (self.row_height - 2.0) + 2.0
    }

    /// The height of a split row: its longer side, wrapped.
    fn split_row_height(&self, row: &SplitRow) -> f32 {
        let lines = match *row {
            SplitRow::Banner(_) => 1,
            SplitRow::Pair { left, right } => [left, right].into_iter().flatten().map(|line| self.wrapped_lines(line)).max().unwrap_or(1),
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
        // A side whose line is shorter than its partner's tints only its own rows; the rest of
        // the row is filler, so a one-line deletion does not look like a longer one.
        let own = if wrap { self.style.unified_row_height(line).min(rect.height()) } else { rect.height() };
        let rect_own = Rect::from_min_size(rect.min, vec2(rect.width(), own));
        if own < rect.height() {
            painter.rect_filled(Rect::from_min_max(pos2(rect.left(), rect.top() + own), rect.max), 0.0, colors.subtle_bg);
        }
        if let Some(background) = background {
            painter.rect_filled(rect_own, 0.0, background);
        }
        if selected {
            painter.rect_filled(rect_own, 0.0, tint(colors.accent, 48));
            painter.rect_filled(Rect::from_min_size(rect.min, vec2(SELECTION_BAR, own)), 0.0, colors.accent);
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
            // Lines wrap between words where they can, as measured for the row heights.
            job.wrap.max_width = wrap_width_of(diff_line.kind, self.style.wrap_width, self.style.char_width);
        }
        let galley = painter.layout_job(job);
        let x = rect.left() + numbers.len() as f32 * self.style.gutter + PADDING + text_offset(diff_line.kind, self.style.char_width);
        let y = first_line_center - (self.style.row_height - 2.0) / 2.0;
        painter.galley(pos2(x, y), galley, text_color);
    }

    /// Paints an empty half of a split row, where the other side has no counterpart.
    fn paint_empty(&self, ui: &Ui, rect: Rect) {
        ui.painter().rect_filled(rect, 0.0, self.style.colors.subtle_bg);
    }
}

/// Draws one unified line into `row_rect`, handling pointer selection on changed lines.
fn draw_unified_row(ui: &mut Ui, line: usize, row_rect: Rect, view: &View, options: &mut DiffOptions, response: &mut DiffResponse) {
    let selected = select_by_pointer(ui, view, row_rect, line, options, response);
    let diff_line = &view.content.lines[line];
    let numbers = [diff_line.old_number, diff_line.new_number];
    view.paint_line(ui, row_rect, line, &numbers, selected, true);
}

/// Draws one row of a split diff into `row_rect`, each side wrapping its long lines.
fn draw_split_row(ui: &mut Ui, row: usize, row_rect: Rect, view: &View, options: &mut DiffOptions, response: &mut DiffResponse) {
    match view.content.split[row] {
        // Banners line up with the text beside the line numbers rather than the pane's edge.
        SplitRow::Banner(line) => view.paint_line(ui, row_rect, line, &[None], false, false),
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
    // Backgrounds stop just short of the row, so highlights on wrapped rows stay apart.
    let format = TextFormat {
        font_id: font.clone(),
        color,
        background: background.unwrap_or(Color32::TRANSPARENT),
        expand_bg: -1.0,
        ..Default::default()
    };
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
            let wrapped = style.wrapped_lines(0);
            assert!(wrapped > 1, "a 400-column line wraps on half of a 900-point view");
            assert_eq!(style.split_row_height(&row), wrapped as f32 * (style.row_height - 2.0) + 2.0);
            // The drawn text, with its highlight colours, wraps onto exactly the rows allowed for.
            let mut job = layout(&long, &style.font, Color32::RED, &[(3..9, Color32::BLUE)]);
            job.wrap.max_width = style.wrap_width;
            let galley = ui.ctx().fonts_mut(|fonts| fonts.layout_job(job));
            assert_eq!(galley.rows.len(), wrapped);
            // Rows break between words: every row but the last ends with the space before a word.
            for row in &galley.rows[..galley.rows.len() - 1] {
                assert!(row.text().ends_with(' '), "{:?}", row.text());
            }
        });
        // No renderer takes the font texture here.
        output.textures_delta.clear();
    }

    #[test]
    fn only_new_and_deleted_files_are_one_sided() {
        let line = |text: &str, kind| DiffLine { text: text.to_string(), kind, old_number: None, new_number: None };
        let new_file = DiffContent::new("new".into(), vec![line("+a", DiffLineKind::Addition), line("+b", DiffLineKind::Addition)]);
        let deleted = DiffContent::new("gone".into(), vec![line("-a", DiffLineKind::Deletion)]);
        let grown = DiffContent::new("grown".into(), vec![line(" a", DiffLineKind::Context), line("+b", DiffLineKind::Addition)]);
        let edited = DiffContent::new("edited".into(), vec![line("-a", DiffLineKind::Deletion), line("+b", DiffLineKind::Addition)]);
        assert!(new_file.is_one_sided() && deleted.is_one_sided());
        assert!(!grown.is_one_sided(), "a file that only gained lines still has its old lines on the left");
        assert!(!edited.is_one_sided());
    }

    #[test]
    fn unified_rows_wrap_long_lines_too() {
        let long = format!("+{}", "word ".repeat(120));
        let lines = vec![DiffLine { text: long, kind: DiffLineKind::Addition, old_number: None, new_number: Some(1) }];
        let content = DiffContent::new("new.txt".to_string(), lines);
        let ctx = egui::Context::default();
        let input = egui::RawInput { screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(700.0, 500.0))), ..Default::default() };
        let mut output = ctx.run_ui(input, |ui| {
            let style = Style::new(ui, &content, false);
            assert!(style.wrapped_lines(0) > 1, "a 600-column line wraps in a 700-point view");
            assert!(style.total <= 700.0, "the view fits its width instead of scrolling sideways");
        });
        output.textures_delta.clear();
    }

    #[test]
    fn tabs_count_as_several_columns() {
        assert_eq!(columns("a\tb"), 1 + TAB_COLUMNS + 1);
    }

    #[test]
    fn headers_start_one_column_in_where_code_content_starts() {
        assert_eq!(text_offset(DiffLineKind::Hunk, 8.0), 8.0);
        assert_eq!(text_offset(DiffLineKind::Metadata, 8.0), 8.0);
        for kind in [DiffLineKind::Context, DiffLineKind::Addition, DiffLineKind::Deletion] {
            assert_eq!(text_offset(kind, 8.0), 0.0, "{kind:?} text begins with its marker, so it is not shifted");
        }
    }

    #[test]
    fn wrapped_header_rows_match_their_measured_height() {
        let header = format!("@@ -1,2 +1,2 @@ {}", "fn long_name_part ".repeat(60));
        let lines = vec![
            DiffLine { text: header.clone(), kind: DiffLineKind::Hunk, old_number: None, new_number: None },
            DiffLine { text: format!("+{}", "word ".repeat(60)), kind: DiffLineKind::Addition, old_number: None, new_number: Some(1) },
        ];
        let content = DiffContent::new("header.rs".to_string(), lines);
        let ctx = egui::Context::default();
        let input = egui::RawInput { screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(700.0, 500.0))), ..Default::default() };
        let mut output = ctx.run_ui(input, |ui| {
            let style = Style::new(ui, &content, false);
            assert_eq!(text_offset(DiffLineKind::Hunk, style.char_width), style.char_width);
            let wrapped = style.wrapped_lines(0);
            assert!(wrapped > 1, "a long hunk header wraps in a 700-point view");
            assert_eq!(style.unified_row_height(0), wrapped as f32 * (style.row_height - 2.0) + 2.0);
            // The drawn header starts one column in, so it wraps onto exactly the rows allowed for.
            let mut job = layout(&header, &style.font, Color32::RED, &[]);
            job.wrap.max_width = wrap_width_of(DiffLineKind::Hunk, style.wrap_width, style.char_width);
            let galley = ui.ctx().fonts_mut(|fonts| fonts.layout_job(job));
            assert_eq!(galley.rows.len(), wrapped);
            // The code beside it keeps the full width.
            assert_eq!(style.wrapped_lines(1), {
                let mut job = layout(&content.lines[1].text, &style.font, Color32::RED, &[]);
                job.wrap.max_width = style.wrap_width;
                ui.ctx().fonts_mut(|fonts| fonts.layout_job(job)).rows.len()
            });
        });
        output.textures_delta.clear();
    }
}
