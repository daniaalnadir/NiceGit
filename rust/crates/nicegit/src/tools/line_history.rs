#![allow(dead_code)]
//! History of a range of lines: the commits that changed those lines, beside the selected
//! commit's change to them. Blame opens it from a line group's right-click menu.

use std::path::Path;

use egui::{vec2, Align, Layout, RichText, Ui};
use egui_phosphor::regular as icon;
use nicegit_core::diff::parse_diff;
use nicegit_core::file_history::LineHistoryEntry;
use nicegit_core::Snapshot;

use crate::diff_view::{self, DiffContent};
use crate::theme;
use crate::tools::blame::file_name;
use crate::tools::file_history::result_row;
use crate::tools::widgets;
use crate::tools::{query, Ctx, Task, ToolWindow};

/// How many commits are loaded. Older history is not shown.
const LIMIT: usize = 200;

type Outcome = nicegit_core::Result<Vec<LineHistoryEntry>>;

/// The commit whose change to the lines is shown beside the list.
struct Selected {
    hash: String,
    subject: String,
    content: DiffContent,
}

/// The commits that changed a range of lines in one file, at a revision or in the working copy.
pub struct LineHistoryWindow {
    path: String,
    /// One-based, inclusive.
    start: usize,
    end: usize,
    /// The revision the lines were blamed at, or `None` for the working copy, whose history is
    /// read from HEAD.
    revision: Option<String>,
    needs_reload: bool,
    entries: Option<Task<Outcome>>,
    /// The last history read, shown while a newer one loads so the window does not flicker.
    shown: Option<Outcome>,
    selected: Option<Selected>,
    split: bool,
}

/// The lines as people read them: "line 4", or "lines 4–9" for a range.
pub fn lines_text(start: usize, end: usize) -> String {
    if start == end {
        format!("line {start}")
    } else {
        format!("lines {start}–{end}")
    }
}

impl LineHistoryWindow {
    pub fn new(path: String, revision: Option<String>, start: usize, end: usize) -> Self {
        Self { path, start, end, revision, needs_reload: true, entries: None, shown: None, selected: None, split: false }
    }

    fn reload(&mut self, ctx: &egui::Context, repo: &Path) {
        let path = self.path.clone();
        let (start, end) = (self.start, self.end);
        let revision = self.revision.clone();
        self.entries =
            Some(query(ctx, repo, move |git, directory| git.line_history(&path, start, end, revision.as_deref(), LIMIT, directory)));
    }
}

impl ToolWindow for LineHistoryWindow {
    fn id(&self) -> String {
        format!("line-history\0{}\0{}\0{}\0{}", self.path, self.start, self.end, self.revision.as_deref().unwrap_or(""))
    }

    fn title(&self) -> String {
        format!("History of {} {}", file_name(&self.path), lines_text(self.start, self.end))
    }

    fn default_size(&self) -> egui::Vec2 {
        vec2(980.0, 600.0)
    }

    fn ui(&mut self, ui: &mut Ui, cx: &mut Ctx) {
        if self.needs_reload {
            self.needs_reload = false;
            self.reload(ui.ctx(), cx.repo);
        }
        // Moves the value out of the finished task once, so it is not copied on every frame.
        let arrived = self.entries.as_mut().and_then(|task| task.get_mut()).map(|value| std::mem::replace(value, Ok(Vec::new())));
        if let Some(value) = arrived {
            self.shown = Some(value);
            self.entries = None;
        }

        let Self { path, revision, shown, selected, split, .. } = self;
        let outcome = shown.as_ref();
        // Select the newest commit once the history arrives.
        if selected.is_none() {
            if let Some(Ok(list)) = outcome {
                if let Some(first) = list.first() {
                    *selected = Some(select(first));
                }
            }
        }

        header(ui, path.as_str(), outcome, revision.is_none());
        ui.add_space(4.0);
        ui.separator();

        ui.horizontal_top(|ui| {
            let height = ui.available_height();
            ui.allocate_ui_with_layout(vec2(340.0, height), Layout::top_down(Align::Min), |ui| {
                list(ui, outcome, selected);
            });
            ui.separator();
            ui.allocate_ui_with_layout(vec2(ui.available_width(), height), Layout::top_down(Align::Min), |ui| {
                match (outcome, selected.as_ref()) {
                    (None, _) => widgets::loading(ui, "Reading history"),
                    (Some(_), Some(current)) => diff_pane(ui, current, split),
                    (Some(_), None) => widgets::empty_state(ui, icon::GIT_DIFF, "Select a commit to see how it changed these lines."),
                }
            });
        });
    }

    fn repository_changed(&mut self, _snapshot: &Snapshot) {
        // HEAD may have moved, so a history read at HEAD is reloaded. A revision never changes.
        if self.revision.is_none() {
            self.needs_reload = true;
        }
    }
}

/// Selects a commit, keeping its change to the lines ready to show.
fn select(entry: &LineHistoryEntry) -> Selected {
    Selected {
        hash: entry.commit.hash.clone(),
        subject: entry.commit.subject.clone(),
        content: DiffContent::new(entry.commit.hash.clone(), parse_diff(&entry.patch)),
    }
}

/// The file, the number of commits, and, for the working copy, what the history leaves out.
fn header(ui: &mut Ui, path: &str, outcome: Option<&Outcome>, working_copy: bool) {
    let c = theme::of(ui);
    ui.vertical(|ui| {
        ui.label(RichText::new(path).monospace());
        let summary = match outcome {
            Some(Ok(list)) if list.len() == LIMIT => format!("Latest {LIMIT} commits"),
            Some(Ok(list)) if list.len() == 1 => "1 commit".to_string(),
            Some(Ok(list)) => format!("{} commits", list.len()),
            _ => String::new(),
        };
        ui.label(RichText::new(summary).small().color(c.muted));
        if working_copy {
            ui.label(RichText::new("Uncommitted changes are not included.").small().color(c.muted));
        }
    });
}

/// The commits that changed the lines, newest first. Clicking a row selects it.
fn list(ui: &mut Ui, outcome: Option<&Outcome>, selected: &mut Option<Selected>) {
    match outcome {
        None => widgets::loading(ui, "Reading history"),
        Some(Err(error)) => widgets::error(ui, &error.to_string()),
        Some(Ok(entries)) if entries.is_empty() => widgets::empty_state(ui, icon::FILE_TEXT, "No commits changed these lines."),
        Some(Ok(entries)) => {
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                for entry in entries {
                    let commit = &entry.commit;
                    let is_selected = selected.as_ref().is_some_and(|current| current.hash == commit.hash);
                    let detail = format!("{} · {} · {}", commit.author_name, commit.relative_date, commit.short_hash);
                    let response = result_row(ui, &commit.subject, &detail, None, None, is_selected);
                    if response.clicked() && !is_selected {
                        *selected = Some(select(entry));
                    }
                    response.context_menu(|ui| {
                        if ui.button("Copy commit hash").clicked() {
                            ui.ctx().copy_text(commit.hash.clone());
                            ui.close();
                        }
                    });
                }
            });
        }
    }
}

/// The selected commit's change to the lines.
fn diff_pane(ui: &mut Ui, selection: &Selected, split: &mut bool) {
    ui.horizontal(|ui| {
        widgets::hash_label(ui, &selection.hash);
        ui.add(egui::Label::new(RichText::new(&selection.subject).strong()).truncate());
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            crate::tools::widgets::checkbox(ui, true, split, "Side by side");
        });
    });
    ui.separator();
    diff_view::show(ui, &selection.content, *split);
}
