//! The command log: every Git command NiceGit has run, newest first, with its exit status, how long
//! it took, and its error output. Commands that only read the repository are hidden unless asked
//! for; a failure always shows. Clicking a row shows its error output and configuration.

use std::collections::HashSet;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use egui::{Rect, RichText, Sense, TextStyle, Ui, UiBuilder};
use egui_phosphor::regular as icon;
use nicegit_core::command_log::{self, Entry};

use crate::i18n::{tr, tr_fmt};
use crate::theme;
use crate::tools::{widgets, Ctx, ToolWindow};

/// Space above and below a row's first line.
const ROW_PADDING: f32 = 6.0;
/// Space above and below the error output shown under an expanded row.
const DETAIL_PADDING: f32 = 6.0;
/// Space between two rows.
const ROW_GAP: f32 = 2.0;
/// How wide the repository folder's name may be before it is shortened.
const FOLDER_WIDTH: f32 = 130.0;
/// How wide the duration column is.
const DURATION_WIDTH: f32 = 64.0;

#[derive(Default)]
pub struct CommandLogWindow {
    /// The entries read so far, oldest first. At most `command_log::CAPACITY` of them.
    entries: Vec<Entry>,
    /// The id of the newest entry read, so the next read asks only for later ones.
    last_id: u64,
    filter: String,
    show_reads: bool,
    only_this_repository: bool,
    /// The rows that show their error output and configuration, by entry id.
    expanded: HashSet<u64>,
}

impl CommandLogWindow {
    pub fn new() -> Self {
        Self { only_this_repository: true, ..Self::default() }
    }

    /// Reads the entries recorded since the last read. Returns whether there were any.
    fn read_new_entries(&mut self) -> bool {
        let new = command_log::entries_since(self.last_id);
        let Some(newest) = new.last() else { return false };
        self.last_id = newest.id;
        self.entries.extend(new);
        let excess = self.entries.len().saturating_sub(command_log::CAPACITY);
        self.entries.drain(..excess);
        true
    }

    /// The indices of the entries to list, newest first, after the filters.
    fn visible(&self, repo: &Path) -> Vec<usize> {
        let filter = self.filter.trim().to_lowercase();
        (0..self.entries.len())
            .rev()
            .filter(|&index| {
                let entry = &self.entries[index];
                (self.show_reads || !entry.background || entry.exit != Some(0))
                    && (!self.only_this_repository || entry.directory.starts_with(repo))
                    && (filter.is_empty() || matches_filter(entry, &filter))
            })
            .collect()
    }

    fn clear(&mut self) {
        command_log::clear();
        self.entries.clear();
        self.expanded.clear();
    }

    fn copy_all(&self, visible: &[usize]) -> String {
        visible.iter().map(|&index| copy_text(&self.entries[index])).collect::<Vec<_>>().join("\n\n")
    }

    fn toolbar(&mut self, ui: &mut Ui, visible: &[usize]) {
        ui.horizontal_wrapped(|ui| {
            widgets::search_input(ui, &mut self.filter, tr("Filter commands"), 220.0);
            widgets::checkbox(ui, true, &mut self.show_reads, tr("Show reads"));
            widgets::checkbox(ui, true, &mut self.only_this_repository, tr("Only this repository"));
            if ui.add_enabled(!self.entries.is_empty(), egui::Button::new(tr("Clear"))).clicked() {
                self.clear();
            }
            if ui.add_enabled(!visible.is_empty(), egui::Button::new(tr("Copy all"))).clicked() {
                ui.ctx().copy_text(self.copy_all(visible));
            }
        });
    }

    /// The rows, laid out at their own heights and drawn only where the list is visible.
    fn list(&mut self, ui: &mut Ui, visible: &[usize]) {
        let heights: Vec<f32> = visible.iter().map(|&index| self.row_height(ui, &self.entries[index])).collect();
        let mut tops = Vec::with_capacity(visible.len() + 1);
        tops.push(0.0);
        for height in &heights {
            tops.push(tops.last().copied().unwrap_or_default() + height + ROW_GAP);
        }
        let total = tops.last().copied().unwrap_or_default();
        egui::ScrollArea::vertical().id_salt("command_log_rows").auto_shrink([false, false]).show_viewport(ui, |ui, viewport| {
            let origin = ui.max_rect().min;
            let width = ui.max_rect().width();
            ui.allocate_rect(Rect::from_min_size(origin, egui::vec2(width, total)), Sense::hover());
            // The rows that overlap the visible part of the list.
            let first = tops.partition_point(|top| *top <= viewport.min.y).saturating_sub(1);
            let last = tops.partition_point(|top| *top < viewport.max.y).min(visible.len());
            for row in first..last {
                let entry = &self.entries[visible[row]];
                let rect = Rect::from_min_size(origin + egui::vec2(0.0, tops[row]), egui::vec2(width, heights[row]));
                let expanded = self.expanded.contains(&entry.id);
                if draw_row(ui, entry, expanded, rect) {
                    if expanded {
                        self.expanded.remove(&entry.id);
                    } else {
                        self.expanded.insert(entry.id);
                    }
                }
            }
        });
    }

    fn row_height(&self, ui: &Ui, entry: &Entry) -> f32 {
        let header = header_height(ui);
        if self.expanded.contains(&entry.id) {
            header + 2.0 * DETAIL_PADDING + detail_lines(entry) as f32 * monospace_height(ui)
        } else {
            header
        }
    }
}

impl ToolWindow for CommandLogWindow {
    fn id(&self) -> String {
        "command_log".to_string()
    }

    fn title(&self) -> String {
        tr("Command Log").to_string()
    }

    fn default_size(&self) -> egui::Vec2 {
        egui::vec2(900.0, 560.0)
    }

    fn ui(&mut self, ui: &mut Ui, cx: &mut Ctx) {
        if self.read_new_entries() {
            ui.ctx().request_repaint();
        }
        let visible = self.visible(cx.repo);
        self.toolbar(ui, &visible);
        ui.add_space(6.0);
        if self.entries.is_empty() {
            widgets::empty_state(ui, icon::TERMINAL, tr("No Git commands have run yet."));
        } else if visible.is_empty() {
            widgets::empty_state(ui, icon::TERMINAL, tr("No commands match these filters."));
        } else {
            self.list(ui, &visible);
        }
    }
}

/// Whether the command or its folder contains `filter`, which is already lower case.
fn matches_filter(entry: &Entry, filter: &str) -> bool {
    command_log::command_line(entry).to_lowercase().contains(filter)
        || entry.directory.display().to_string().to_lowercase().contains(filter)
}

/// The row's first line: status, time, duration, folder, and the command.
fn header_height(ui: &Ui) -> f32 {
    ui.text_style_height(&TextStyle::Body) + 2.0 * ROW_PADDING
}

fn monospace_height(ui: &Ui) -> f32 {
    ui.text_style_height(&TextStyle::Monospace)
}

/// The lines under an expanded row: its error output, then its configuration.
fn detail_lines(entry: &Entry) -> usize {
    entry.stderr.lines().count().max(1) + entry.config.len()
}

/// Draws one row, and returns whether its first line was clicked, to expand or collapse it. The
/// click area covers only the first line, and is registered after its contents, so the contents
/// do not take the pointer from it and the output under an expanded row stays selectable.
fn draw_row(ui: &mut Ui, entry: &Entry, expanded: bool, rect: Rect) -> bool {
    let c = theme::of(ui);
    let header_rect = Rect::from_min_size(rect.min, egui::vec2(rect.width(), header_height(ui)));
    let hovered = ui.input(|input| input.pointer.hover_pos()).is_some_and(|pointer| header_rect.contains(pointer));
    if hovered {
        ui.painter().rect_filled(header_rect, 6.0, ui.visuals().widgets.hovered.weak_bg_fill);
    }
    let command = command_log::command_line(entry);
    let folder = folder_name(&entry.directory);
    ui.scope_builder(UiBuilder::new().max_rect(header_rect.shrink2(egui::vec2(8.0, 0.0))), |ui| {
        ui.horizontal(|ui| {
            let (glyph, color) = if entry.exit == Some(0) { (icon::CHECK_CIRCLE, c.added) } else { (icon::WARNING_CIRCLE, c.danger) };
            ui.label(RichText::new(glyph).color(color)).on_hover_text(status_text(entry));
            ui.label(RichText::new(clock(entry.started)).monospace().small().color(c.muted));
            ui.add_sized(
                [DURATION_WIDTH, ui.spacing().interact_size.y],
                egui::Label::new(
                    RichText::new(tr_fmt!("{milliseconds} ms", milliseconds = entry.duration.as_millis())).small().color(c.muted),
                ),
            );
            ui.add_sized(
                [FOLDER_WIDTH, ui.spacing().interact_size.y],
                egui::Label::new(RichText::new(&folder).small().color(c.muted)).truncate(),
            )
            .on_hover_text(entry.directory.display().to_string());
            ui.add(egui::Label::new(RichText::new(&command).monospace()).truncate()).on_hover_text(command.as_str());
        });
    });
    if expanded {
        let detail = Rect::from_min_max(
            egui::pos2(rect.left() + 8.0, rect.top() + header_rect.height() + DETAIL_PADDING),
            egui::pos2(rect.right() - 8.0, rect.bottom() - DETAIL_PADDING),
        );
        draw_detail(ui, entry, detail);
    }
    let response = ui.interact(header_rect, ui.scope_id().with(("command_log_row", entry.id)), Sense::click());
    response.context_menu(|ui| {
        if ui.button(tr("Copy command")).clicked() {
            ui.ctx().copy_text(command.clone());
            ui.close();
        }
        if ui.add_enabled(!entry.stderr.is_empty(), egui::Button::new(tr("Copy output"))).clicked() {
            ui.ctx().copy_text(entry.stderr.clone());
            ui.close();
        }
    });
    response.clicked()
}

/// The error output and configuration of an expanded row, one line each, in monospace.
fn draw_detail(ui: &mut Ui, entry: &Entry, rect: Rect) {
    let c = theme::of(ui);
    let font = TextStyle::Monospace.resolve(ui.style());
    ui.scope_builder(UiBuilder::new().max_rect(rect).layout(egui::Layout::top_down(egui::Align::Min)), |ui| {
        // The rows are sized to their lines, so no space may sit between them.
        ui.spacing_mut().item_spacing.y = 0.0;
        if entry.stderr.is_empty() {
            ui.label(RichText::new(tr("No error output.")).font(font.clone()).color(c.muted));
        }
        for line in entry.stderr.lines() {
            ui.add(egui::Label::new(RichText::new(line).font(font.clone())).truncate().selectable(true)).on_hover_text(line);
        }
        for (key, value) in &entry.config {
            let text = format!("-c {key}={value}");
            ui.add(egui::Label::new(RichText::new(&text).font(font.clone()).color(c.muted)).truncate().selectable(true))
                .on_hover_text(text.as_str());
        }
    });
}

fn status_text(entry: &Entry) -> String {
    match entry.exit {
        Some(0) => tr("Succeeded").to_string(),
        Some(code) => tr_fmt!("Exited with status {code}.", code = code),
        None => tr("Stopped or did not start.").to_string(),
    }
}

/// The repository folder's name, or its whole path when it has none.
fn folder_name(directory: &Path) -> String {
    directory.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_else(|| directory.display().to_string())
}

/// The text copied for one entry: its time and command, then its error output.
fn copy_text(entry: &Entry) -> String {
    let mut text = format!("{}  {}", clock(entry.started), command_log::command_line(entry));
    if !entry.stderr.is_empty() {
        text.push('\n');
        text.push_str(entry.stderr.trim_end());
    }
    text
}

/// The local time of day, as HH:MM:SS.
fn clock(time: SystemTime) -> String {
    let seconds = time.duration_since(UNIX_EPOCH).map(|elapsed| elapsed.as_secs() as i64).unwrap_or(0);
    let (hour, minute, second) = local_time_of_day(seconds);
    format!("{hour:02}:{minute:02}:{second:02}")
}

/// The hour, minute, and second at `seconds` since the Unix epoch in the local time zone. Windows
/// uses UTC, as this build reads no local time zone there.
#[cfg(unix)]
fn local_time_of_day(seconds: i64) -> (i64, i64, i64) {
    let time = seconds as libc::time_t;
    // SAFETY: `localtime_r` writes only into the `tm` it is given, which lives on this frame, and
    // keeps no pointer to either argument.
    let local = unsafe {
        let mut parts: libc::tm = std::mem::zeroed();
        if libc::localtime_r(&time, &mut parts).is_null() {
            None
        } else {
            Some(parts)
        }
    };
    match local {
        Some(parts) => (i64::from(parts.tm_hour), i64::from(parts.tm_min), i64::from(parts.tm_sec)),
        None => utc_time_of_day(seconds),
    }
}

#[cfg(not(unix))]
fn local_time_of_day(seconds: i64) -> (i64, i64, i64) {
    utc_time_of_day(seconds)
}

fn utc_time_of_day(seconds: i64) -> (i64, i64, i64) {
    let day = seconds.rem_euclid(86_400);
    (day / 3_600, day % 3_600 / 60, day % 60)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::time::Duration;

    fn entry(arguments: &[&str], directory: &str) -> Entry {
        Entry {
            id: 1,
            started: UNIX_EPOCH,
            duration: Duration::from_millis(4),
            directory: PathBuf::from(directory),
            arguments: arguments.iter().map(|argument| argument.to_string()).collect(),
            config: Vec::new(),
            exit: Some(0),
            stdout_bytes: 0,
            stderr: String::new(),
            background: false,
        }
    }

    #[test]
    fn the_filter_matches_the_command_or_the_folder() {
        let stage = entry(&["add", "--all"], "/work/notes");
        assert!(matches_filter(&stage, "add --all"));
        assert!(matches_filter(&stage, "notes"));
        assert!(!matches_filter(&stage, "status"));
    }

    #[test]
    fn utc_times_of_day_are_split_into_hours_minutes_and_seconds() {
        assert_eq!(utc_time_of_day(0), (0, 0, 0));
        assert_eq!(utc_time_of_day(3_723), (1, 2, 3));
        assert_eq!(utc_time_of_day(86_399 + 86_400 * 4), (23, 59, 59));
    }

    #[test]
    fn copied_text_has_the_time_and_command_then_the_error_output() {
        let mut failed = entry(&["push", "origin", "main"], "/work/notes");
        failed.exit = Some(1);
        failed.stderr = "rejected\n".to_string();
        let text = copy_text(&failed);
        assert!(text.ends_with("git push origin main\nrejected"), "{text}");
        assert!(!copy_text(&entry(&["log"], "/work")).contains('\n'));
    }

    #[test]
    fn detail_lines_count_error_output_and_configuration() {
        let mut detail = entry(&["fetch"], "/work");
        assert_eq!(detail_lines(&detail), 1, "an empty error output still takes a line");
        detail.stderr = "one\ntwo\nthree".to_string();
        detail.config = vec![("core.autocrlf".to_string(), "false".to_string())];
        assert_eq!(detail_lines(&detail), 4);
    }
}
