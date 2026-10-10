#![allow(dead_code)]
//! Blame: each line of a file beside the commit that last changed it. Newer changes are tinted
//! more strongly, and selecting a line shows that commit's change to the file.

use std::collections::HashMap;
use std::path::Path;

use egui::{pos2, vec2, Align, Align2, Color32, CursorIcon, FontId, Layout, Rect, RichText, Sense, Stroke, TextStyle, Ui};
use egui_phosphor::regular as icon;
use nicegit_core::blame::BlameLine;
use nicegit_core::diff::parse_diff;
use nicegit_core::Snapshot;

use crate::diff_view::{self, DiffContent};
use crate::theme;
use crate::tools::widgets;
use crate::tools::{query, Ctx, Task, ToolWindow};

/// Width of the commit column, which holds the hash, author, and age of each group.
const GUTTER: f32 = 250.0;
/// Width of the right-aligned line number after the gutter.
const NUMBER_WIDTH: f32 = 52.0;
const ACCENT_WIDTH: f32 = 4.0;

/// The commit whose change to the file is shown beside the list.
struct Selected {
    hash: String,
    summary: String,
    /// The file's path in that commit.
    file: String,
    /// Whether the diff leaves out whitespace-only changes.
    ignores_whitespace: bool,
    diff: Task<nicegit_core::Result<DiffContent>>,
}

/// Blame for a file in the working copy, or at a revision.
pub struct BlameWindow {
    path: String,
    revision: Option<String>,
    ignore_whitespace: bool,
    /// A one-based line to highlight and scroll to, such as a content search match.
    focus_line: Option<usize>,
    /// Scrolls to the focus line on the first frame that shows the lines.
    focus_pending: bool,
    lines: Option<Task<nicegit_core::Result<Vec<BlameLine>>>>,
    /// Each commit's age tint, from 0 (oldest) to 1 (newest), computed once the lines arrive.
    ages: Option<HashMap<String, f32>>,
    selected: Option<Selected>,
    split: bool,
}

impl BlameWindow {
    /// Blame for `path` at `revision`, or in the working file when `revision` is `None`.
    pub fn new(path: String, revision: Option<String>) -> Self {
        Self {
            path,
            revision,
            ignore_whitespace: false,
            focus_line: None,
            focus_pending: false,
            lines: None,
            ages: None,
            selected: None,
            split: false,
        }
    }

    /// Highlights a one-based line and scrolls to it when the window first shows the lines.
    pub fn focused_on(mut self, line: usize) -> Self {
        self.focus_line = Some(line);
        self.focus_pending = true;
        self
    }

    fn reload(&mut self, ctx: &egui::Context, repo: &Path) {
        let path = self.path.clone();
        let revision = self.revision.clone();
        let ignore_whitespace = self.ignore_whitespace;
        self.lines = Some(query(ctx, repo, move |git, directory| git.blame(&path, revision.as_deref(), ignore_whitespace, directory)));
        self.ages = None;
    }

    fn header(&mut self, ui: &mut Ui, reload: &mut bool) {
        let c = theme::of(ui);
        ui.horizontal(|ui| {
            let scope = match &self.revision {
                Some(revision) => format!("at {}", short_revision(revision)),
                None => "working file".to_string(),
            };
            ui.vertical(|ui| {
                ui.label(RichText::new(&self.path).monospace());
                ui.label(RichText::new(scope).small().color(c.muted));
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let toggle = crate::tools::widgets::checkbox(ui, true, &mut self.ignore_whitespace, "Ignore whitespace")
                    .on_hover_text("Attribute lines past whitespace-only changes to the commit that changed their content");
                if toggle.changed() {
                    *reload = true;
                }
            });
        });
    }
}

impl ToolWindow for BlameWindow {
    fn id(&self) -> String {
        format!("blame\0{}\0{}", self.path, self.revision.as_deref().unwrap_or(""))
    }

    fn title(&self) -> String {
        format!("Blame: {}", file_name(&self.path))
    }

    fn default_size(&self) -> egui::Vec2 {
        egui::vec2(980.0, 620.0)
    }

    fn ui(&mut self, ui: &mut Ui, cx: &mut Ctx) {
        if self.lines.is_none() {
            self.reload(ui.ctx(), cx.repo);
        }
        let mut reload = false;
        self.header(ui, &mut reload);
        if reload {
            self.reload(ui.ctx(), cx.repo);
        }
        ui.add_space(6.0);
        ui.separator();

        let Self { path, lines, ages, selected, split, focus_line, focus_pending, .. } = self;
        // A change to the whitespace setting reloads the commit's diff.
        if let Some(current) = selected.as_mut().filter(|current| current.ignores_whitespace != cx.ignore_whitespace) {
            current.ignores_whitespace = cx.ignore_whitespace;
            current.diff = commit_diff(ui.ctx(), cx.repo, &current.hash, &current.file, cx.ignore_whitespace);
        }
        let Some(task) = lines.as_mut() else { return };
        let Some(result) = task.get() else {
            widgets::loading(ui, "Reading blame");
            return;
        };
        let lines = match result {
            Ok(lines) => lines,
            Err(error) => {
                widgets::error(ui, &error.to_string());
                return;
            }
        };
        if lines.is_empty() {
            widgets::empty_state(ui, icon::FILE_TEXT, "This file is empty.");
            return;
        }
        let ages = ages.get_or_insert_with(|| age_ranks(lines));

        let highlight = *focus_line;
        let scroll_row = if *focus_pending { highlight.and_then(|line| lines.iter().position(|item| item.number == line)) } else { None };
        let selected_hash = selected.as_ref().map(|selection| selection.hash.clone());
        let mut clicked = None;
        let mut close = false;
        if let Some(selection) = selected.as_mut() {
            ui.columns(2, |columns| {
                clicked = blame_rows(&mut columns[0], lines, ages, selected_hash.as_deref(), highlight, scroll_row);
                close = diff_pane(&mut columns[1], selection, split);
            });
        } else {
            clicked = blame_rows(ui, lines, ages, None, highlight, scroll_row);
        }
        *focus_pending = false;
        if close {
            *selected = None;
        }

        if let Some(index) = clicked {
            let commit = &lines[index].commit;
            if !commit.is_uncommitted() {
                cx.select_commit(commit.hash.clone());
                if selected.as_ref().is_none_or(|current| current.hash != commit.hash) {
                    let file = if commit.path.is_empty() { path.as_str() } else { commit.path.as_str() };
                    *selected = Some(Selected {
                        hash: commit.hash.clone(),
                        summary: commit.summary.clone(),
                        file: file.to_string(),
                        ignores_whitespace: cx.ignore_whitespace,
                        diff: commit_diff(ui.ctx(), cx.repo, &commit.hash, file, cx.ignore_whitespace),
                    });
                }
            }
        }
    }

    fn repository_changed(&mut self, _snapshot: &Snapshot) {
        // The working file may have changed; a revision never does.
        if self.revision.is_none() {
            self.lines = None;
        }
    }
}

/// The change a commit made to one file, against its first parent, as a diff.
pub fn commit_diff(
    ctx: &egui::Context,
    repo: &Path,
    hash: &str,
    path: &str,
    ignore_whitespace: bool,
) -> Task<nicegit_core::Result<DiffContent>> {
    let (hash, path) = (hash.to_string(), path.to_string());
    query(ctx, repo, move |git, directory| {
        let patch = git.commit_file_diff(&hash, &path, ignore_whitespace, directory)?;
        Ok(DiffContent::new(hash, parse_diff(&patch)))
    })
}

/// Newest commits rank 1, oldest 0, so the tint shows recency rather than absolute age.
fn age_ranks(lines: &[BlameLine]) -> HashMap<String, f32> {
    let mut times: HashMap<&str, i64> = HashMap::new();
    for line in lines {
        times.entry(line.commit.hash.as_str()).or_insert(line.commit.author_time.unwrap_or(i64::MIN));
    }
    let mut ordered: Vec<(&str, i64)> = times.into_iter().collect();
    ordered.sort_by_key(|&(hash, time)| (time, hash));
    let last = ordered.len().saturating_sub(1);
    ordered
        .iter()
        .enumerate()
        .map(|(rank, (hash, _))| {
            let strength = if last == 0 { 1.0 } else { rank as f32 / last as f32 };
            (hash.to_string(), strength)
        })
        .collect()
}

/// Paints the blame list, one row per line, and returns the index of a row clicked this frame.
fn blame_rows(
    ui: &mut Ui,
    lines: &[BlameLine],
    ages: &HashMap<String, f32>,
    selected: Option<&str>,
    highlight: Option<usize>,
    scroll_row: Option<usize>,
) -> Option<usize> {
    let row_height = ui.text_style_height(&TextStyle::Monospace) + 6.0;
    let mut area = egui::ScrollArea::vertical().auto_shrink([false, false]);
    if let Some(row) = scroll_row {
        // Keep the target a third of the way down, so the lines around it stay in view.
        let offset = (row as f32 * row_height - ui.available_height() / 3.0).max(0.0);
        area = area.vertical_scroll_offset(offset);
    }
    let mut clicked = None;
    area.show_rows(ui, row_height, lines.len(), |ui, range| {
        for index in range {
            let focus = highlight == Some(lines[index].number);
            if paint_row(ui, lines, index, ages, selected, focus, row_height) {
                clicked = Some(index);
            }
        }
    });
    clicked
}

/// Paints one line with its gutter, and reports whether it was clicked.
fn paint_row(
    ui: &mut Ui,
    lines: &[BlameLine],
    index: usize,
    ages: &HashMap<String, f32>,
    selected: Option<&str>,
    focus: bool,
    row_height: f32,
) -> bool {
    let c = theme::of(ui);
    let line = &lines[index];
    let commit = &line.commit;
    let uncommitted = commit.is_uncommitted();
    let accent = if uncommitted { c.conflict } else { c.accent };
    let strength = if uncommitted { 0.9 } else { 0.2 + 0.7 * ages.get(&commit.hash).copied().unwrap_or(0.5) };

    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), row_height), Sense::click());
    let painter = ui.painter_at(rect);
    if selected == Some(commit.hash.as_str()) {
        painter.rect_filled(rect, 0.0, tint(accent, 0.16));
    } else if response.hovered() {
        painter.rect_filled(rect, 0.0, tint(accent, 0.07));
    }
    if focus {
        painter.rect_filled(rect, 0.0, c.banner_bg);
    }
    painter.rect_filled(Rect::from_min_size(rect.min, vec2(ACCENT_WIDTH, rect.height())), 0.0, tint(accent, strength));

    let middle = rect.center().y;
    let text_color = ui.visuals().text_color();
    let starts_group = index == 0 || lines[index - 1].commit.hash != commit.hash;
    if starts_group {
        if index > 0 {
            painter.hline(rect.x_range(), rect.top(), Stroke::new(1.0, c.border));
        }
        let gutter = Rect::from_min_max(pos2(rect.left() + ACCENT_WIDTH, rect.top()), pos2(rect.left() + GUTTER, rect.bottom()));
        let gutter_painter = painter.with_clip_rect(gutter);
        let (hash, author) = if uncommitted {
            ("Uncommitted".to_string(), "Your working changes")
        } else {
            (commit.short_hash().to_string(), commit.author_name.as_str())
        };
        gutter_painter.text(pos2(gutter.left() + 8.0, middle), Align2::LEFT_CENTER, hash, FontId::monospace(11.0), c.muted);
        gutter_painter.text(pos2(gutter.left() + 84.0, middle), Align2::LEFT_CENTER, author, FontId::proportional(12.0), text_color);
        if let (false, Some(time)) = (uncommitted, commit.author_time) {
            gutter_painter.text(
                pos2(gutter.right() - 8.0, middle),
                Align2::RIGHT_CENTER,
                relative_time(time),
                FontId::proportional(10.5),
                c.muted,
            );
        }
    }

    let number_right = rect.left() + GUTTER + NUMBER_WIDTH;
    painter.text(pos2(number_right, middle), Align2::RIGHT_CENTER, line.number.to_string(), FontId::monospace(11.0), c.muted);
    painter.text(pos2(number_right + 12.0, middle), Align2::LEFT_CENTER, displayed(&line.content), FontId::monospace(12.0), text_color);

    let clicked = response.clicked();
    // Described for screen readers and UI tests as the commit, author, and line text.
    let described = if uncommitted { "Uncommitted".to_string() } else { commit.short_hash().to_string() };
    let author_text = if uncommitted { "Your working changes" } else { commit.author_name.as_str() };
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, true, format!("{described} · {author_text} · {}", displayed(&line.content)))
    });
    let tooltip = if uncommitted {
        "Not committed yet".to_string()
    } else {
        format!("{}\n{} <{}> · {}", commit.summary, commit.author_name, commit.author_email, commit.short_hash())
    };
    let cursor = if uncommitted { CursorIcon::Default } else { CursorIcon::PointingHand };
    response.on_hover_cursor(cursor).on_hover_text(tooltip);
    clicked
}

/// The change to the file in the selected commit. Returns true when its close button is pressed.
fn diff_pane(ui: &mut Ui, selection: &mut Selected, split: &mut bool) -> bool {
    let mut close = false;
    ui.horizontal(|ui| {
        widgets::hash_label(ui, &selection.hash);
        ui.add(egui::Label::new(RichText::new(&selection.summary).strong()).truncate());
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if widgets::icon_button(ui, icon::X, "Close this change", true).clicked() {
                close = true;
            }
            crate::tools::widgets::checkbox(ui, true, split, "Side by side");
        });
    });
    ui.separator();
    match selection.diff.get() {
        None => widgets::loading(ui, "Loading change"),
        Some(Err(error)) => widgets::error(ui, &error.to_string()),
        Some(Ok(content)) => diff_view::show(ui, content, *split),
    }
    close
}

/// A colour with the given strength, for tinting rows and bars.
pub fn tint(color: Color32, strength: f32) -> Color32 {
    let alpha = (strength.clamp(0.0, 1.0) * 255.0).round() as u8;
    Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), alpha)
}

/// A revision as shown to people: its first eight characters, or the whole name when shorter.
pub fn short_revision(revision: &str) -> &str {
    revision.get(..revision.len().min(8)).unwrap_or(revision)
}

/// The last component of a path.
pub fn file_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// A commit time as "3 days ago", measured from now.
pub fn relative_time(time: i64) -> String {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|elapsed| elapsed.as_secs() as i64).unwrap_or(time);
    let seconds = (now - time).max(0);
    let (value, unit) = match seconds {
        0..=59 => return "just now".to_string(),
        60..=3_599 => (seconds / 60, "minute"),
        3_600..=86_399 => (seconds / 3_600, "hour"),
        86_400..=2_591_999 => (seconds / 86_400, "day"),
        2_592_000..=31_535_999 => (seconds / 2_592_000, "month"),
        _ => (seconds / 31_536_000, "year"),
    };
    let plural = if value == 1 { "" } else { "s" };
    format!("{value} {unit}{plural} ago")
}

/// Tabs keep their width, and the carriage return of a CRLF line stays invisible.
fn displayed(content: &str) -> String {
    content.strip_suffix('\r').unwrap_or(content).replace('\t', "    ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(number: usize, hash: &str, time: i64) -> BlameLine {
        BlameLine {
            number,
            content: String::new(),
            commit: nicegit_core::blame::BlameCommit {
                hash: hash.to_string(),
                author_name: String::new(),
                author_email: String::new(),
                author_time: Some(time),
                summary: String::new(),
                path: String::new(),
            },
        }
    }

    #[test]
    fn newer_commits_get_a_stronger_age_tint() {
        let lines = [line(1, "old", 100), line(2, "middle", 200), line(3, "new", 300), line(4, "old", 100)];
        let ranks = age_ranks(&lines);
        assert_eq!(ranks["old"], 0.0, "the oldest commit is palest");
        assert_eq!(ranks["middle"], 0.5);
        assert_eq!(ranks["new"], 1.0, "the newest commit is strongest");
        assert_eq!(ranks.len(), 3, "each commit is ranked once, however many lines it has");
    }

    #[test]
    fn a_file_from_one_commit_is_fully_tinted() {
        let ranks = age_ranks(&[line(1, "only", 100), line(2, "only", 100)]);
        assert_eq!(ranks["only"], 1.0);
    }
}
