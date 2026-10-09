#![allow(dead_code)]
//! File history: the commits that changed one file, beside the selected commit's change to it.
//! A version can be restored from here, after confirming it against the current checkout.

use std::path::Path;

use egui::{pos2, vec2, Align, Align2, Color32, CursorIcon, FontId, Id, Layout, Rect, RichText, Sense, Ui};
use egui_phosphor::regular as icon;
use nicegit_core::file_history::FileHistoryEntry;
use nicegit_core::Snapshot;

use crate::diff_view::{self, DiffContent};
use crate::theme;
use crate::tools::blame::{commit_diff, file_name, short_revision, tint, BlameWindow};
use crate::tools::widgets;
use crate::tools::{query, Ctx, Task, ToolWindow};

/// How many commits are loaded. Older history is not shown.
const LIMIT: usize = 200;

/// A restore awaiting confirmation. It keeps the checkout the restore was chosen for, so a
/// confirmation cannot act on a different branch or commit.
struct PendingRestore {
    path: String,
    /// The commit whose version of the file is restored.
    source: String,
    /// The version as described to the user, such as "in commit abc1234".
    description: String,
    removes_file: bool,
    branch: String,
    head: Option<String>,
}

/// The commit whose change to the file is shown beside the list.
struct Selected {
    hash: String,
    subject: String,
    path: String,
    deletes_file: bool,
    diff: Task<nicegit_core::Result<DiffContent>>,
}

/// The commits that changed one file in the current checkout.
pub struct FileHistoryWindow {
    path: String,
    entries: Option<Task<nicegit_core::Result<Vec<FileHistoryEntry>>>>,
    selected: Option<Selected>,
    split: bool,
    restore: Option<PendingRestore>,
    close: bool,
}

impl FileHistoryWindow {
    pub fn new(path: String) -> Self {
        Self { path, entries: None, selected: None, split: false, restore: None, close: false }
    }

    fn reload(&mut self, ctx: &egui::Context, repo: &Path) {
        let path = self.path.clone();
        self.entries = Some(query(ctx, repo, move |git, directory| git.file_history(&path, LIMIT, directory)));
    }
}

impl ToolWindow for FileHistoryWindow {
    fn id(&self) -> String {
        format!("file-history\0{}", self.path)
    }

    fn title(&self) -> String {
        format!("File history: {}", file_name(&self.path))
    }

    fn default_size(&self) -> egui::Vec2 {
        egui::vec2(980.0, 600.0)
    }

    fn ui(&mut self, ui: &mut Ui, cx: &mut Ctx) {
        if self.entries.is_none() {
            self.reload(ui.ctx(), cx.repo);
        }
        let Self { path, entries, selected, split, restore, close } = self;
        let Some(task) = entries.as_mut() else { return };
        let outcome = task.get();

        // Select the newest commit once the history arrives, as the list would otherwise be empty.
        if selected.is_none() {
            if let Some(Ok(list)) = outcome {
                if let Some(first) = list.first() {
                    *selected = Some(select(ui.ctx(), cx.repo, first));
                }
            }
        }

        ui.horizontal(|ui| {
            let c = theme::of(ui);
            ui.vertical(|ui| {
                ui.label(RichText::new(path.as_str()).monospace());
                let summary = match outcome {
                    Some(Ok(list)) if list.len() == LIMIT => format!("Latest {LIMIT} commits"),
                    Some(Ok(list)) if list.len() == 1 => "1 commit".to_string(),
                    Some(Ok(list)) => format!("{} commits", list.len()),
                    _ => String::new(),
                };
                ui.label(RichText::new(summary).small().color(c.muted));
            });
        });
        ui.add_space(4.0);
        ui.separator();

        ui.horizontal_top(|ui| {
            let height = ui.available_height();
            let file = path.as_str();
            ui.allocate_ui_with_layout(vec2(340.0, height), Layout::top_down(Align::Min), |ui| {
                list(ui, cx, file, outcome, selected, restore);
            });
            ui.separator();
            ui.allocate_ui_with_layout(vec2(ui.available_width(), height), Layout::top_down(Align::Min), |ui| match selected.as_mut() {
                Some(selection) => diff_pane(ui, cx, file, selection, split, restore),
                None => widgets::empty_state(ui, icon::GIT_DIFF, "Select a commit to see how it changed this file."),
            });
        });

        confirm_restore(ui, cx, restore, close);
    }

    fn wants_close(&self) -> bool {
        self.close
    }

    fn repository_changed(&mut self, _snapshot: &Snapshot) {
        // A new commit may have changed the file; reload the history on the next frame.
        self.entries = None;
    }
}

/// Selects a commit and starts loading its change to the file.
fn select(ctx: &egui::Context, repo: &Path, entry: &FileHistoryEntry) -> Selected {
    Selected {
        hash: entry.commit.hash.clone(),
        subject: entry.commit.subject.clone(),
        path: entry.path.clone(),
        deletes_file: entry.deletes_file(),
        diff: commit_diff(ctx, repo, &entry.commit.hash, &entry.path),
    }
}

/// Whether a restore may be chosen now: no other action is running, and no Git operation is
/// unfinished.
fn can_restore(cx: &Ctx) -> bool {
    cx.idle && cx.snapshot.operation.is_none()
}

/// A restore of `path` to its version in commit `hash`, against the checkout current right now.
fn pending_restore(cx: &Ctx, path: &str, hash: &str, removes_file: bool) -> PendingRestore {
    PendingRestore {
        path: path.to_string(),
        source: hash.to_string(),
        description: format!("in commit {}", short_revision(hash)),
        removes_file,
        branch: cx.snapshot.current_branch.clone(),
        head: cx.snapshot.head_hash.clone(),
    }
}

/// The commit list. Clicking a row selects it; its menu restores, blames, or copies it.
fn list(
    ui: &mut Ui,
    cx: &mut Ctx,
    path: &str,
    outcome: Option<&nicegit_core::Result<Vec<FileHistoryEntry>>>,
    selected: &mut Option<Selected>,
    restore: &mut Option<PendingRestore>,
) {
    match outcome {
        None => widgets::loading(ui, "Reading history"),
        Some(Err(error)) => widgets::error(ui, &error.to_string()),
        Some(Ok(entries)) if entries.is_empty() => {
            widgets::empty_state(ui, icon::FILE_TEXT, "No commits in this checkout changed this file.")
        }
        Some(Ok(entries)) => {
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                for entry in entries {
                    let is_selected = selected.as_ref().is_some_and(|current| current.hash == entry.commit.hash);
                    let detail = format!("{} · {} · {}", entry.commit.author_name, entry.commit.relative_date, entry.commit.short_hash);
                    let moved = (entry.path != path).then(|| format!("As {}", entry.path));
                    let letter = entry.status.as_str();
                    let badge = Some((letter, widgets::change_letter_color(ui, letter)));
                    let response = result_row(ui, &entry.commit.subject, &detail, badge, moved.as_deref(), is_selected);
                    if response.clicked() && !is_selected {
                        *selected = Some(select(ui.ctx(), cx.repo, entry));
                    }
                    response.context_menu(|ui| {
                        let restorable = can_restore(cx) && entry.path == path;
                        let label = if entry.deletes_file() { "Delete file to this version…" } else { "Restore file to this version…" };
                        if ui.add_enabled(restorable, egui::Button::new(label)).clicked() {
                            *restore = Some(pending_restore(cx, path, &entry.commit.hash, entry.deletes_file()));
                            ui.close();
                        }
                        if ui.add_enabled(!entry.deletes_file(), egui::Button::new("Blame at this commit")).clicked() {
                            cx.open(Box::new(BlameWindow::new(entry.path.clone(), Some(entry.commit.hash.clone()))));
                            ui.close();
                        }
                        ui.separator();
                        if ui.button("Copy commit hash").clicked() {
                            ui.ctx().copy_text(entry.commit.hash.clone());
                            ui.close();
                        }
                    });
                }
            });
        }
    }
}

/// The selected commit's change to the file, with its restore and blame actions.
fn diff_pane(ui: &mut Ui, cx: &mut Ctx, path: &str, selection: &mut Selected, split: &mut bool, restore: &mut Option<PendingRestore>) {
    ui.horizontal(|ui| {
        widgets::hash_label(ui, &selection.hash);
        ui.add(egui::Label::new(RichText::new(&selection.subject).strong()).truncate());
    });
    ui.horizontal(|ui| {
        let same_file = selection.path == path;
        let enabled = can_restore(cx) && same_file;
        let label = if selection.deletes_file { "Delete file" } else { "Restore this version" };
        let hint = if !cx.idle {
            "Wait for the current action to finish."
        } else if cx.snapshot.operation.is_some() {
            "Finish the current Git operation first."
        } else if !same_file {
            "Git restores by path. This commit holds the file under another name."
        } else {
            "Replace this file with its version in the selected commit."
        };
        if widgets::labeled_button(ui, icon::ARROW_COUNTER_CLOCKWISE, label, enabled).on_hover_text(hint).clicked() {
            *restore = Some(pending_restore(cx, path, &selection.hash, selection.deletes_file));
        }
        let blame_enabled = !selection.deletes_file;
        if widgets::labeled_button(ui, icon::ROWS, "Blame at this commit", blame_enabled).clicked() {
            cx.open(Box::new(BlameWindow::new(path.to_string(), Some(selection.hash.clone()))));
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.checkbox(split, "Side by side");
        });
    });
    ui.add_space(2.0);
    ui.separator();
    match selection.diff.get() {
        None => widgets::loading(ui, "Loading change"),
        Some(Err(error)) => widgets::error(ui, &error.to_string()),
        Some(Ok(content)) => diff_view::show(ui, content, *split),
    }
}

/// The restore confirmation. Confirming queues the restore against the captured checkout and
/// closes the window, as the restore reports its outcome in the status bar.
fn confirm_restore(ui: &mut Ui, cx: &mut Ctx, restore: &mut Option<PendingRestore>, close: &mut bool) {
    let Some(pending) = restore.as_ref() else { return };
    let idle = cx.idle;
    let mut decision = None;
    let response = egui::Modal::new(Id::new(("file-history-restore", pending.path.as_str()))).show(ui.ctx(), |ui| {
        ui.set_width(440.0);
        let title = if pending.removes_file { format!("Delete {}?", pending.path) } else { format!("Restore {}?", pending.path) };
        ui.label(RichText::new(title).strong().size(16.0));
        ui.add_space(6.0);
        let message = if pending.removes_file {
            format!(
                "This file does not exist {}. It will be deleted from your working files and the deletion staged. Any uncommitted changes to it will be lost. Your commits and other files are not changed.",
                pending.description
            )
        } else {
            format!(
                "Your working copy of this file will be replaced with its version {}, and the change staged. Any staged or unstaged changes to it will be lost. Your commits and other files are not changed.",
                pending.description
            )
        };
        ui.label(message);
        ui.add_space(14.0);
        ui.horizontal(|ui| {
            if ui.button("Cancel").clicked() {
                decision = Some(false);
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let action = if pending.removes_file { "Delete file" } else { "Restore file" };
                if widgets::danger_button(ui, action, idle).clicked() {
                    decision = Some(true);
                }
            });
        });
    });
    if response.backdrop_response.clicked() {
        decision = Some(false);
    }
    match decision {
        Some(true) => {
            if let Some(pending) = restore.take() {
                let verb = if pending.removes_file { "Deleted" } else { "Restored" };
                let label = format!("Restore {}", pending.path);
                cx.act(label, move |git, directory| {
                    git.restore(&pending.path, &pending.source, &pending.branch, pending.head.as_deref(), directory)?;
                    Ok(Some(format!("{verb} {} {}.", pending.path, pending.description)))
                });
                *close = true;
            }
        }
        Some(false) => *restore = None,
        None => {}
    }
}

/// One row of a result list: a title with an optional badge, a detail line, and an optional
/// extra line in monospace. Highlights when selected or hovered.
pub fn result_row(
    ui: &mut Ui,
    title: &str,
    detail: &str,
    badge: Option<(&str, Color32)>,
    extra: Option<&str>,
    selected: bool,
) -> egui::Response {
    let c = theme::of(ui);
    let height = if extra.is_some() { 62.0 } else { 46.0 };
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::click());
    let painter = ui.painter_at(rect);
    if selected {
        painter.rect_filled(rect, 0.0, tint(c.accent, 0.14));
    } else if response.hovered() {
        painter.rect_filled(rect, 0.0, c.subtle_bg);
    }
    let mut title_right = rect.right() - 14.0;
    if let Some((letter, color)) = badge {
        painter.text(pos2(rect.right() - 14.0, rect.top() + 10.0), Align2::RIGHT_TOP, letter, FontId::monospace(12.0), color);
        title_right -= 26.0;
    }
    let title_clip = Rect::from_min_max(rect.min, pos2(title_right, rect.bottom()));
    painter.with_clip_rect(title_clip).text(
        pos2(rect.left() + 14.0, rect.top() + 10.0),
        Align2::LEFT_TOP,
        title,
        FontId::proportional(13.0),
        ui.visuals().text_color(),
    );
    painter.text(pos2(rect.left() + 14.0, rect.top() + 29.0), Align2::LEFT_TOP, detail, FontId::proportional(11.0), c.muted);
    if let Some(extra) = extra {
        painter.text(pos2(rect.left() + 14.0, rect.top() + 46.0), Align2::LEFT_TOP, extra, FontId::monospace(10.5), c.muted);
    }
    // Described for screen readers and UI tests as the title and detail together.
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, format!("{title} · {detail}")));
    response.on_hover_cursor(CursorIcon::PointingHand)
}
