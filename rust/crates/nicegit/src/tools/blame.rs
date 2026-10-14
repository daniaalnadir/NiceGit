#![allow(dead_code)]
//! Blame: each line of a file beside the commit that last changed it. Newer changes are tinted
//! more strongly, and selecting a line shows that commit's change to the file.

use std::collections::HashMap;
use std::path::Path;

use egui::{pos2, vec2, Align, Align2, Color32, CursorIcon, FontId, Layout, Rect, RichText, Sense, Stroke, TextStyle, Ui};
use egui_phosphor::regular as icon;
use nicegit_core::blame::{BlameLine, BlameReport, IgnoredRevs};
use nicegit_core::diff::{parse_diff, DiffRequest};
use nicegit_core::Snapshot;

use crate::diff_view::{self, DiffContent};
use crate::theme;
use crate::tools::line_history::{lines_text, LineHistoryWindow};
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
    /// The diff options the commit's change was loaded with: whitespace and context.
    request: DiffRequest,
    diff: Task<nicegit_core::Result<DiffContent>>,
}

/// An earlier view to return to: its revision, its path, and the line to scroll to.
type View = (Option<String>, String, usize);

/// Blame for a file in the working copy, or at a revision.
pub struct BlameWindow {
    path: String,
    revision: Option<String>,
    /// The window's identity, fixed when it opens, so stepping through history keeps the window.
    identity: String,
    ignore_whitespace: bool,
    /// Whether blame passes over the commits the repository lists in `.git-blame-ignore-revs`.
    ignore_revs: bool,
    /// Whether the repository has a `.git-blame-ignore-revs` file, which decides whether its
    /// checkbox is shown. Updated with each loaded blame.
    ignore_list: bool,
    /// A one-based line to highlight and scroll to, such as a content search match.
    focus_line: Option<usize>,
    /// Scrolls to the focus line on the first frame that shows the lines.
    focus_pending: bool,
    /// A one-based line to scroll to once a view's lines arrive.
    scroll_line: Option<usize>,
    /// The views that "Blame before this change" stepped back from, newest last.
    history: Vec<View>,
    /// The blame and whether the repository has an ignore list.
    lines: Option<Task<nicegit_core::Result<(BlameReport, bool)>>>,
    /// The summary of the revision's commit, shown beside its hash.
    summary: Option<Task<nicegit_core::Result<String>>>,
    /// Each commit's age tint, from 0 (oldest) to 1 (newest), computed once the lines arrive.
    ages: Option<HashMap<String, f32>>,
    selected: Option<Selected>,
    split: bool,
}

/// What a line's context menu asked for. The window applies it once the rows are painted.
enum Choice {
    /// Blame `path` at `revision`, the commit before the clicked line's change.
    BlameBefore {
        revision: String,
        path: String,
        line: usize,
    },
    /// Blame `path` at HEAD, for a line that is not committed yet.
    BlameAtHead {
        path: String,
        line: usize,
    },
    ShowCommit(String),
    CopyCommit(String),
    /// Show the commits that changed lines `start` to `end` of `path`, at `revision` or in the
    /// working copy when `revision` is `None`.
    LineHistory {
        path: String,
        revision: Option<String>,
        start: usize,
        end: usize,
    },
}

/// The blamed lines and how they are drawn.
struct Rows<'a> {
    lines: &'a [BlameLine],
    ages: &'a HashMap<String, f32>,
    selected: Option<&'a str>,
    highlight: Option<usize>,
    scroll_row: Option<usize>,
    /// The path the lines were blamed at, used when a line's own path is empty.
    path: &'a str,
    /// The revision the lines were blamed at, or `None` for the working copy.
    revision: Option<&'a str>,
    repo: &'a Path,
}

impl BlameWindow {
    /// Blame for `path` at `revision`, or in the working file when `revision` is `None`.
    pub fn new(path: String, revision: Option<String>) -> Self {
        let identity = format!("blame\0{}\0{}", path, revision.as_deref().unwrap_or(""));
        Self {
            path,
            revision,
            identity,
            ignore_whitespace: false,
            ignore_revs: true,
            ignore_list: false,
            focus_line: None,
            focus_pending: false,
            scroll_line: None,
            history: Vec::new(),
            lines: None,
            summary: None,
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
        let ignore_revs = self.ignore_revs;
        self.lines = Some(query(ctx, repo, move |git, directory| {
            let report = git.blame_report(&path, revision.as_deref(), ignore_whitespace, ignore_revs, directory)?;
            Ok((report, git.has_blame_ignore_revs(directory)))
        }));
        self.summary = self.revision.clone().map(|revision| {
            query(ctx, repo, move |git, directory| {
                Ok(git.commit_message(&revision, directory)?.lines().next().unwrap_or_default().to_string())
            })
        });
        self.ages = None;
    }

    /// Blames `path` at `revision`, after pushing the current view so Back can return to it.
    /// `line` is the clicked line; Back scrolls to it, and the new view starts there.
    fn step_into(&mut self, ctx: &egui::Context, repo: &Path, revision: Option<String>, path: String, line: usize) {
        let previous_revision = std::mem::replace(&mut self.revision, revision);
        let previous_path = std::mem::replace(&mut self.path, path);
        self.history.push((previous_revision, previous_path, line));
        self.show_view(ctx, repo, line);
    }

    fn step_back(&mut self, ctx: &egui::Context, repo: &Path) {
        let Some((revision, path, line)) = self.history.pop() else { return };
        self.revision = revision;
        self.path = path;
        self.show_view(ctx, repo, line);
    }

    /// Loads the current revision and path, and clears what the replaced view showed.
    fn show_view(&mut self, ctx: &egui::Context, repo: &Path, line: usize) {
        self.reload(ctx, repo);
        self.selected = None;
        self.focus_line = None;
        self.focus_pending = false;
        self.scroll_line = Some(line);
    }

    fn header(&mut self, ui: &mut Ui, reload: &mut bool, back: &mut bool) {
        let c = theme::of(ui);
        ui.horizontal(|ui| {
            if widgets::icon_button(ui, icon::ARROW_LEFT, "Back to the newer blame", !self.history.is_empty()).clicked() {
                *back = true;
            }
            let scope = match &self.revision {
                Some(revision) => match self.summary.as_mut().and_then(|task| task.get()) {
                    Some(Ok(summary)) if !summary.is_empty() => format!("at {} ({summary})", short_revision(revision)),
                    _ => format!("at {}", short_revision(revision)),
                },
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
                if self.ignore_list {
                    let skip = crate::tools::widgets::checkbox(ui, true, &mut self.ignore_revs, "Skip commits in .git-blame-ignore-revs")
                        .on_hover_text(
                            "Lines changed only by the commits this repository lists in .git-blame-ignore-revs, such as \
                             reformatting, are attributed to the change before them, as GitHub's blame does.",
                        );
                    if skip.changed() {
                        *reload = true;
                    }
                }
            });
        });
    }
}

impl ToolWindow for BlameWindow {
    fn id(&self) -> String {
        self.identity.clone()
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
        let mut back = false;
        self.header(ui, &mut reload, &mut back);
        if back {
            self.step_back(ui.ctx(), cx.repo);
        } else if reload {
            self.reload(ui.ctx(), cx.repo);
        }
        ui.add_space(6.0);
        ui.separator();

        let Self { path, revision, lines, ages, selected, split, focus_line, focus_pending, scroll_line, ignore_list, .. } = self;
        // A change to the whitespace or context setting reloads the commit's diff.
        let request = cx.diff_request();
        if let Some(current) = selected.as_mut().filter(|current| current.request != request) {
            current.request = request;
            current.diff = commit_diff(ui.ctx(), cx.repo, &current.hash, &current.file, request);
        }
        let Some(task) = lines.as_mut() else { return };
        let Some(result) = task.get() else {
            widgets::loading(ui, "Reading blame");
            return;
        };
        let (report, has_ignore_list) = match result {
            Ok(loaded) => loaded,
            Err(error) => {
                widgets::error(ui, &error.to_string());
                return;
            }
        };
        *ignore_list = *has_ignore_list;
        let lines = &report.lines;
        if let IgnoredRevs::Unusable(reason) = &report.ignored_revs {
            let c = theme::of(ui);
            ui.label(
                RichText::new(format!("The ignore list could not be used, so every commit is shown: {reason}")).small().color(c.muted),
            );
        }
        if lines.is_empty() {
            widgets::empty_state(ui, icon::FILE_TEXT, "This file is empty.");
            return;
        }
        let ages = ages.get_or_insert_with(|| age_ranks(lines));

        let highlight = *focus_line;
        let scroll_row = match scroll_line.take() {
            Some(line) => Some(line.saturating_sub(1).min(lines.len() - 1)),
            None if *focus_pending => highlight.and_then(|line| lines.iter().position(|item| item.number == line)),
            None => None,
        };
        let selected_hash = selected.as_ref().map(|selection| selection.hash.clone());
        let rows = Rows {
            lines,
            ages,
            selected: selected_hash.as_deref(),
            highlight,
            scroll_row,
            path: path.as_str(),
            revision: revision.as_deref(),
            repo: cx.repo,
        };
        let mut clicked = None;
        let mut choice = None;
        let mut close = false;
        if let Some(selection) = selected.as_mut() {
            ui.columns(2, |columns| {
                clicked = blame_rows(&mut columns[0], &rows, &mut choice);
                close = diff_pane(&mut columns[1], selection, split);
            });
        } else {
            clicked = blame_rows(ui, &rows, &mut choice);
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
                        request,
                        diff: commit_diff(ui.ctx(), cx.repo, &commit.hash, file, request),
                    });
                }
            }
        }

        match choice {
            Some(Choice::BlameBefore { revision, path: file, line }) => self.step_into(ui.ctx(), cx.repo, Some(revision), file, line),
            Some(Choice::BlameAtHead { path: file, line }) => self.step_into(ui.ctx(), cx.repo, Some("HEAD".to_string()), file, line),
            Some(Choice::ShowCommit(hash)) => cx.select_commit(hash),
            Some(Choice::CopyCommit(hash)) => ui.ctx().copy_text(hash),
            Some(Choice::LineHistory { path, revision, start, end }) => {
                cx.open(Box::new(LineHistoryWindow::new(path, revision, start, end)));
            }
            None => {}
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
    request: DiffRequest,
) -> Task<nicegit_core::Result<DiffContent>> {
    let (hash, path) = (hash.to_string(), path.to_string());
    query(ctx, repo, move |git, directory| {
        let patch = git.commit_file_diff(&hash, &path, request, directory)?;
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
fn blame_rows(ui: &mut Ui, rows: &Rows, choice: &mut Option<Choice>) -> Option<usize> {
    let row_height = ui.text_style_height(&TextStyle::Monospace) + 6.0;
    let mut area = egui::ScrollArea::vertical().auto_shrink([false, false]);
    if let Some(row) = rows.scroll_row {
        // Keep the target a third of the way down, so the lines around it stay in view.
        let offset = (row as f32 * row_height - ui.available_height() / 3.0).max(0.0);
        area = area.vertical_scroll_offset(offset);
    }
    let mut clicked = None;
    area.show_rows(ui, row_height, rows.lines.len(), |ui, range| {
        for index in range {
            let focus = rows.highlight == Some(rows.lines[index].number);
            if paint_row(ui, rows, index, choice, focus, row_height) {
                clicked = Some(index);
            }
        }
    });
    clicked
}

/// Paints one line with its gutter, and reports whether it was clicked. Its context menu writes
/// the choice it made into `choice`.
fn paint_row(ui: &mut Ui, rows: &Rows, index: usize, choice: &mut Option<Choice>, focus: bool, row_height: f32) -> bool {
    let c = theme::of(ui);
    let line = &rows.lines[index];
    let commit = &line.commit;
    let uncommitted = commit.is_uncommitted();
    let accent = if uncommitted { c.conflict } else { c.accent };
    let strength = if uncommitted { 0.9 } else { 0.2 + 0.7 * rows.ages.get(&commit.hash).copied().unwrap_or(0.5) };

    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), row_height), Sense::click());
    let painter = ui.painter_at(rect);
    if rows.selected == Some(commit.hash.as_str()) {
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
    let starts_group = index == 0 || rows.lines[index - 1].commit.hash != commit.hash;
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
    let response = response.on_hover_cursor(cursor).on_hover_text(tooltip);
    let line_number = line.number;
    response.context_menu(|ui| {
        if uncommitted {
            if ui.button("Blame at HEAD").clicked() {
                *choice = Some(Choice::BlameAtHead { path: rows.path.to_string(), line: line_number });
                ui.close();
            }
            ui.add_enabled(false, egui::Button::new("Show commit in history")).on_disabled_hover_text("Not committed yet");
            ui.add_enabled(false, egui::Button::new("Copy commit ID")).on_disabled_hover_text("Not committed yet");
            ui.separator();
            let ((first, last), _) = group_bounds(rows.lines, index);
            ui.add_enabled(false, egui::Button::new(format!("History of {}…", lines_text(first, last))))
                .on_disabled_hover_text("Not committed yet");
            return;
        }
        // Git names the version these lines came from, with the file's path there, which
        // follows a rename made in this very commit.
        let before = match &commit.previous {
            Some((revision, path)) => Ok((revision.clone(), path.clone())),
            None => Err("This commit has no earlier version of the file".to_string()),
        };
        match before {
            Ok((revision, path)) => {
                if ui.button("Blame before this change").clicked() {
                    *choice = Some(Choice::BlameBefore { revision, path, line: line_number });
                    ui.close();
                }
            }
            Err(reason) => {
                ui.add_enabled(false, egui::Button::new("Blame before this change")).on_disabled_hover_text(reason);
            }
        }
        if ui.button("Show commit in history").clicked() {
            *choice = Some(Choice::ShowCommit(commit.hash.clone()));
            ui.close();
        }
        if ui.button("Copy commit ID").clicked() {
            *choice = Some(Choice::CopyCommit(commit.hash.clone()));
            ui.close();
        }
        ui.separator();
        let ((first, last), (original_first, original_last)) = group_bounds(rows.lines, index);
        if ui.button(format!("History of {}…", lines_text(first, last))).clicked() {
            // Read at the lines' own commit, by their numbers and path there: blame says no later
            // commit changed them, and edits above them in the working file cannot shift the range.
            let path = if commit.path.is_empty() { rows.path } else { commit.path.as_str() };
            *choice = Some(Choice::LineHistory {
                path: path.to_string(),
                revision: Some(commit.hash.clone()),
                start: original_first,
                end: original_last,
            });
            ui.close();
        }
    });
    clicked
}

/// The group that line `index` belongs to: the consecutive lines sharing its commit. Returns the
/// one-based first and last lines as shown, and as numbered in that commit's version of the file.
fn group_bounds(lines: &[BlameLine], index: usize) -> ((usize, usize), (usize, usize)) {
    let hash = lines[index].commit.hash.as_str();
    let mut first = index;
    while first > 0 && lines[first - 1].commit.hash == hash {
        first -= 1;
    }
    let mut last = index;
    while last + 1 < lines.len() && lines[last + 1].commit.hash == hash {
        last += 1;
    }
    let group = &lines[first..=last];
    let original_first = group.iter().map(|line| line.original_number).min().unwrap_or(1);
    let original_last = group.iter().map(|line| line.original_number).max().unwrap_or(original_first);
    ((lines[first].number, lines[last].number), (original_first, original_last))
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
            original_number: number,
            number,
            content: String::new(),
            commit: nicegit_core::blame::BlameCommit {
                hash: hash.to_string(),
                author_name: String::new(),
                author_email: String::new(),
                author_time: Some(time),
                summary: String::new(),
                path: String::new(),
                previous: None,
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
