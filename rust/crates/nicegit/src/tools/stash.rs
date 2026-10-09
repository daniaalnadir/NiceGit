//! Stashes: save every change or only the selected files, then apply, pop, or delete saved
//! stashes and read what each one holds. Stashing selected files leaves every other file where
//! it is.

// The window is not opened from the menus yet, so nothing constructs it in the application.

use std::collections::BTreeSet;

use egui::{Color32, Margin, RichText, Sense, Ui};
use egui_phosphor::regular as icon;
use nicegit_core::diff::parse_diff;
use nicegit_core::models::{Operation, Stash, StatusEntry, StatusKind};
use nicegit_core::Snapshot;

use crate::diff_view::{self, DiffContent};
use crate::theme;
use crate::tools::{query, widgets, Ctx, Task, ToolWindow};

/// A changed file that can be stashed. Conflicted files are left out, because Git cannot stash them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct StashFile {
    pub path: String,
    pub untracked: bool,
}

/// What saving does with the files that are ticked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum SavePlan {
    /// Every change, including untracked files, goes into one stash.
    All,
    /// Only these paths go into the stash.
    Paths(Vec<String>),
}

/// The changed files that can be stashed, in the order Git lists them.
pub(crate) fn stashable_files(status: &[StatusEntry]) -> Vec<StashFile> {
    status
        .iter()
        .filter(|entry| entry.kind != StatusKind::Conflicted)
        .map(|entry| StashFile { path: entry.path.clone(), untracked: entry.kind == StatusKind::Untracked })
        .collect()
}

/// The ticked paths that saving stashes. Untracked files are dropped when they are excluded.
pub(crate) fn stash_paths(files: &[StashFile], selected: &BTreeSet<String>, include_untracked: bool) -> Vec<String> {
    files
        .iter()
        .filter(|file| selected.contains(&file.path) && (include_untracked || !file.untracked))
        .map(|file| file.path.clone())
        .collect()
}

/// How saving stashes the ticked files, or `None` when nothing would be stashed. Saving every
/// file, with untracked files included, uses Git's whole-tree stash; anything else names the
/// paths, so files that are not ticked stay in the working tree.
pub(crate) fn save_plan(files: &[StashFile], selected: &BTreeSet<String>, include_untracked: bool) -> Option<SavePlan> {
    let paths = stash_paths(files, selected, include_untracked);
    if paths.is_empty() {
        return None;
    }
    let every_file_ticked = files.iter().all(|file| selected.contains(&file.path));
    if every_file_ticked && include_untracked {
        Some(SavePlan::All)
    } else {
        Some(SavePlan::Paths(paths))
    }
}

/// Why saving is not possible right now, if it is not.
pub(crate) fn save_blocker(has_changes: bool, has_selection: bool, idle: bool, operation: Option<Operation>) -> Option<String> {
    if let Some(operation) = operation {
        return Some(format!("Finish or abort the {} in progress before stashing.", operation.name()));
    }
    if !idle {
        return Some("Wait for the current Git action to finish.".to_string());
    }
    if !has_changes {
        return Some("There are no changes to stash.".to_string());
    }
    if !has_selection {
        return Some("Select files to stash.".to_string());
    }
    None
}

/// What a confirmation dialog is asking about.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Confirm {
    Pop,
    Delete,
}

/// The preview of the selected stash, once it has been read.
struct Preview {
    hash: String,
    content: Result<DiffContent, String>,
}

/// A stash preview being read in the background.
struct Loading {
    hash: String,
    title: String,
    task: Task<nicegit_core::Result<String>>,
}

/// Saves changes as stashes and manages the saved ones.
pub struct StashWindow {
    message: String,
    include_untracked: bool,
    /// The paths ticked to be stashed.
    selected: BTreeSet<String>,
    /// The paths seen in the status so far. A path seen for the first time is ticked, because
    /// stashing everything is the default.
    known: BTreeSet<String>,
    /// The number of stashes when a save started. The message is cleared once a new stash appears.
    awaiting_save: Option<usize>,
    /// The hash of the stash whose changes are shown.
    selected_stash: Option<String>,
    loading: Option<Loading>,
    preview: Option<Preview>,
    /// A pop or delete waiting for confirmation.
    pending: Option<(Confirm, Stash)>,
}

impl Default for StashWindow {
    fn default() -> Self {
        Self::new()
    }
}

impl StashWindow {
    pub fn new() -> Self {
        Self {
            message: String::new(),
            include_untracked: true,
            selected: BTreeSet::new(),
            known: BTreeSet::new(),
            awaiting_save: None,
            selected_stash: None,
            loading: None,
            preview: None,
            pending: None,
        }
    }

    /// Ticks new files, drops ticks for files that are no longer changed, and forgets files that
    /// are gone.
    fn sync(&mut self, snapshot: &Snapshot) {
        let current: BTreeSet<String> = stashable_files(&snapshot.status).into_iter().map(|file| file.path).collect();
        for path in current.difference(&self.known) {
            self.selected.insert(path.clone());
        }
        self.selected.retain(|path| current.contains(path));
        self.known = current;
    }

    /// Starts reading the preview of `stash`, unless it is already shown.
    fn select(&mut self, stash: &Stash, ctx: &egui::Context, repo: &std::path::Path) {
        if self.selected_stash.as_deref() == Some(stash.hash.as_str()) {
            return;
        }
        self.selected_stash = Some(stash.hash.clone());
        self.preview = None;
        let title = format!("{} · {}", stash.reference, stash.message);
        let owned = stash.clone();
        self.loading = Some(Loading {
            hash: stash.hash.clone(),
            title,
            task: query(ctx, repo, move |client, directory| client.stash_diff(&owned, directory)),
        });
    }

    /// Takes the preview once it has been read. A result for a stash that is no longer selected is dropped.
    fn poll_preview(&mut self) {
        let Some(loading) = self.loading.as_mut() else { return };
        let Some(result) = loading.task.get().cloned() else { return };
        let Some(loading) = self.loading.take() else { return };
        if self.selected_stash.as_deref() != Some(loading.hash.as_str()) {
            return;
        }
        let content = match result {
            Ok(patch) => Ok(DiffContent::new(loading.title, parse_diff(&patch))),
            Err(error) => Err(error.to_string()),
        };
        self.preview = Some(Preview { hash: loading.hash, content });
    }

    /// Forgets the selected stash and anything read for it.
    fn clear_selected_stash(&mut self) {
        self.selected_stash = None;
        self.preview = None;
        self.loading = None;
    }

    fn save_section(&mut self, ui: &mut Ui, cx: &mut Ctx, files: &[StashFile]) {
        let c = theme::of(ui);
        widgets::section(ui, "Save");
        ui.add_space(4.0);
        let hint = format!("WIP on {}", cx.snapshot.current_branch);
        widgets::text_field(ui, &mut self.message, &hint);
        ui.add_space(4.0);

        ui.horizontal(|ui| {
            ui.checkbox(&mut self.include_untracked, "Include untracked files");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.add_enabled(!files.is_empty(), egui::Button::new("None")).clicked() {
                    self.selected.clear();
                }
                if ui.add_enabled(!files.is_empty(), egui::Button::new("All")).clicked() {
                    self.selected = files.iter().map(|file| file.path.clone()).collect();
                }
            });
        });

        if files.is_empty() {
            ui.label(RichText::new("No changed files.").small().color(c.muted));
        } else {
            egui::ScrollArea::vertical().id_salt("stash_files").auto_shrink([false, false]).max_height(130.0).show(ui, |ui| {
                for file in files {
                    // Untracked files are shown but cannot be ticked while they are excluded.
                    let disabled = file.untracked && !self.include_untracked;
                    let mut checked = !disabled && self.selected.contains(&file.path);
                    let mut name = file.path.clone();
                    if file.untracked {
                        name.push_str("  (untracked)");
                    }
                    let response = ui.add_enabled(!disabled, egui::Checkbox::new(&mut checked, RichText::new(name).monospace()));
                    if response.changed() {
                        if checked {
                            self.selected.insert(file.path.clone());
                        } else {
                            self.selected.remove(&file.path);
                        }
                    }
                }
            });
        }
        ui.add_space(6.0);

        let plan = save_plan(files, &self.selected, self.include_untracked);
        let blocker = save_blocker(!files.is_empty(), plan.is_some(), cx.idle, cx.snapshot.operation);
        let count = stash_paths(files, &self.selected, self.include_untracked).len();
        ui.horizontal(|ui| {
            let label = match &plan {
                Some(SavePlan::All) | None => "Stash changes".to_string(),
                Some(SavePlan::Paths(_)) => format!("Stash {count} {}", if count == 1 { "file" } else { "files" }),
            };
            if widgets::primary_button(ui, &label, blocker.is_none()).clicked() {
                if let Some(plan) = plan {
                    self.start_save(cx, plan);
                }
            }
            if let Some(blocker) = &blocker {
                ui.label(RichText::new(blocker).small().color(c.muted));
            }
        });
    }

    fn start_save(&mut self, cx: &mut Ctx, plan: SavePlan) {
        let message = self.message.clone();
        self.awaiting_save = Some(cx.snapshot.stashes.len());
        match plan {
            SavePlan::All => cx.act("Stash changes", move |client, directory| {
                client.save_stash(&message, directory).map(|()| Some("Saved your changes in a stash.".to_string()))
            }),
            SavePlan::Paths(paths) => {
                let label = format!("Stash {} selected files", paths.len());
                let count = paths.len();
                cx.act(label, move |client, directory| {
                    client.save_stash_paths(&paths, &message, directory).map(|()| Some(format!("Saved {count} selected files in a stash.")))
                });
            }
        }
    }

    fn stash_list(&mut self, ui: &mut Ui, cx: &mut Ctx) {
        let c = theme::of(ui);
        let idle = cx.idle;
        let snapshot = cx.snapshot;
        let blocked = !idle || snapshot.operation.is_some();
        let stashes = &snapshot.stashes;
        if stashes.is_empty() {
            widgets::empty_state(ui, icon::ARCHIVE_BOX, "No stashes");
            return;
        }
        egui::ScrollArea::vertical().id_salt("stash_list").auto_shrink([false, false]).max_height(190.0).show(ui, |ui| {
            for stash in stashes {
                let is_selected = self.selected_stash.as_deref() == Some(stash.hash.as_str());
                let fill = if is_selected { ui.visuals().selection.bg_fill } else { Color32::TRANSPARENT };
                egui::Frame::new().fill(fill).corner_radius(6.0).inner_margin(Margin::symmetric(8, 5)).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        let left = ui.vertical(|ui| {
                            ui.set_max_width((ui.available_width() - 200.0).max(120.0));
                            ui.add(egui::Label::new(RichText::new(&stash.message).strong()).truncate());
                            ui.label(RichText::new(&stash.reference).monospace().small().color(c.muted));
                        });
                        if left.response.interact(Sense::click()).clicked() {
                            self.select(stash, ui.ctx(), cx.repo);
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if widgets::icon_button(ui, icon::TRASH, "Delete stash", idle).clicked() {
                                self.pending = Some((Confirm::Delete, stash.clone()));
                            }
                            if action_button(ui, "Pop", "Restore the changes, then remove the stash if they apply", !blocked).clicked() {
                                self.pending = Some((Confirm::Pop, stash.clone()));
                            }
                            if action_button(ui, "Apply", "Restore the changes and keep the stash", !blocked).clicked() {
                                let reference = stash.reference.clone();
                                let stash = stash.clone();
                                cx.act(format!("Apply {reference}"), move |client, directory| {
                                    client.apply_stash(&stash, directory).map(|()| Some(format!("Applied {reference}.")))
                                });
                            }
                        });
                    });
                });
                ui.add_space(2.0);
            }
        });
    }

    /// The pop or delete waiting for confirmation, shown in the window until it is answered.
    fn confirmation(&mut self, ui: &mut Ui, cx: &mut Ctx) {
        let Some((kind, stash)) = self.pending.clone() else { return };
        let blocked = !cx.idle || cx.snapshot.operation.is_some();
        let (text, button) = match kind {
            Confirm::Pop => (
                format!("Apply {} and remove it? The stash is removed only after its changes apply. If applying fails or conflicts, it is kept.", stash.reference),
                "Pop stash",
            ),
            Confirm::Delete => (format!("Delete {} permanently? Its changes cannot be recovered.", stash.reference), "Delete stash"),
        };
        let mut confirmed = false;
        let mut cancelled = false;
        egui::Frame::new().fill(theme::of(ui).banner_bg).corner_radius(6.0).inner_margin(Margin::symmetric(10, 8)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(format!("{}  {text}", icon::WARNING)).color(theme::of(ui).warning));
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if kind == Confirm::Delete {
                    confirmed = widgets::danger_button(ui, button, !blocked).clicked();
                } else {
                    confirmed = widgets::primary_button(ui, button, !blocked).clicked();
                }
                if ui.button("Cancel").clicked() {
                    cancelled = true;
                }
            });
        });
        if cancelled {
            self.pending = None;
        }
        if confirmed {
            self.pending = None;
            let reference = stash.reference.clone();
            match kind {
                Confirm::Pop => cx.act(format!("Pop {reference}"), move |client, directory| {
                    client.pop_stash(&stash, directory).map(|()| Some(format!("Popped {reference}.")))
                }),
                Confirm::Delete => cx.act(format!("Delete {reference}"), move |client, directory| {
                    client.drop_stash(&stash, directory).map(|()| Some(format!("Deleted {reference}.")))
                }),
            }
        }
        ui.add_space(6.0);
    }

    fn preview(&mut self, ui: &mut Ui) {
        let c = theme::of(ui);
        let Some(hash) = self.selected_stash.clone() else {
            widgets::empty_state(ui, icon::EYE, "Select a stash to see its changes.");
            return;
        };
        if self.loading.as_ref().is_some_and(|loading| loading.hash == hash) {
            widgets::loading(ui, "Reading the stash...");
            return;
        }
        match &self.preview {
            Some(preview) if preview.hash == hash => match &preview.content {
                Ok(content) => diff_view::show(ui, content, false),
                Err(error) => widgets::error(ui, error),
            },
            _ => {
                ui.label(RichText::new("Select a stash to see its changes.").color(c.muted));
            }
        }
    }
}

/// A small text button whose disabled state explains itself on hover.
fn action_button(ui: &mut Ui, text: &str, tooltip: &str, enabled: bool) -> egui::Response {
    ui.add_enabled(enabled, egui::Button::new(text)).on_hover_text(tooltip).on_disabled_hover_text(tooltip)
}

impl ToolWindow for StashWindow {
    fn id(&self) -> String {
        "stashes".to_string()
    }

    fn title(&self) -> String {
        "Stashes".to_string()
    }

    fn default_size(&self) -> egui::Vec2 {
        egui::vec2(780.0, 720.0)
    }

    fn repository_changed(&mut self, snapshot: &Snapshot) {
        self.sync(snapshot);
        // A save that produced a new stash has finished, so its message is no longer needed.
        if self.awaiting_save.is_some_and(|count| snapshot.stashes.len() > count) {
            self.message.clear();
            self.awaiting_save = None;
        }
        if let Some(hash) = self.selected_stash.clone() {
            if !snapshot.stashes.iter().any(|stash| stash.hash == hash) {
                self.clear_selected_stash();
            }
        }
        if let Some((_, stash)) = &self.pending {
            if !snapshot.stashes.iter().any(|listed| listed.hash == stash.hash) {
                self.pending = None;
            }
        }
    }

    fn ui(&mut self, ui: &mut Ui, cx: &mut Ctx) {
        self.sync(cx.snapshot);
        self.poll_preview();
        let c = theme::of(ui);
        let files = stashable_files(&cx.snapshot.status);

        ui.add_space(4.0);
        ui.label(RichText::new("Stashes").size(16.0).strong());
        ui.label(RichText::new("Set changes aside to switch work or clean up, and restore them later.").small().color(c.muted));
        ui.add_space(8.0);

        self.save_section(ui, cx, &files);
        ui.add_space(8.0);
        ui.separator();
        ui.add_space(6.0);

        widgets::section(ui, "Saved stashes");
        ui.add_space(4.0);
        self.stash_list(ui, cx);
        ui.add_space(6.0);
        self.confirmation(ui, cx);
        ui.separator();
        ui.add_space(6.0);
        self.preview(ui);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(path: &str, untracked: bool) -> StashFile {
        StashFile { path: path.to_string(), untracked }
    }

    fn ticked(paths: &[&str]) -> BTreeSet<String> {
        paths.iter().map(|path| path.to_string()).collect()
    }

    fn sample() -> Vec<StashFile> {
        vec![file("a.rs", false), file("b.rs", false), file("new.txt", true)]
    }

    #[test]
    fn saving_every_file_with_untracked_uses_the_whole_tree_stash() {
        assert_eq!(save_plan(&sample(), &ticked(&["a.rs", "b.rs", "new.txt"]), true), Some(SavePlan::All));
    }

    #[test]
    fn saving_a_subset_names_only_those_paths() {
        assert_eq!(
            save_plan(&sample(), &ticked(&["b.rs", "new.txt"]), true),
            Some(SavePlan::Paths(vec!["b.rs".to_string(), "new.txt".to_string()]))
        );
    }

    #[test]
    fn excluding_untracked_names_the_tracked_paths_even_when_all_are_ticked() {
        assert_eq!(
            save_plan(&sample(), &ticked(&["a.rs", "b.rs", "new.txt"]), false),
            Some(SavePlan::Paths(vec!["a.rs".to_string(), "b.rs".to_string()]))
        );
    }

    #[test]
    fn untracked_ticks_are_dropped_when_untracked_is_excluded() {
        let files = sample();
        assert_eq!(save_plan(&files, &ticked(&["new.txt"]), false), None);
        assert_eq!(stash_paths(&files, &ticked(&["a.rs", "new.txt"]), false), vec!["a.rs".to_string()]);
        assert_eq!(stash_paths(&files, &ticked(&["a.rs", "new.txt"]), true), vec!["a.rs".to_string(), "new.txt".to_string()]);
    }

    #[test]
    fn nothing_ticked_means_no_save() {
        assert_eq!(save_plan(&sample(), &BTreeSet::new(), true), None);
        assert_eq!(save_plan(&[], &BTreeSet::new(), true), None);
    }

    #[test]
    fn ticks_for_files_no_longer_listed_are_ignored() {
        assert_eq!(save_plan(&sample(), &ticked(&["gone.rs"]), true), None);
    }

    #[test]
    fn conflicted_files_are_not_offered_for_stashing() {
        let status = vec![
            StatusEntry {
                path: "merge.rs".to_string(),
                original_path: None,
                kind: StatusKind::Conflicted,
                index_status: 'U',
                work_tree_status: 'U',
            },
            StatusEntry {
                path: "new.txt".to_string(),
                original_path: None,
                kind: StatusKind::Untracked,
                index_status: '?',
                work_tree_status: '?',
            },
        ];
        assert_eq!(stashable_files(&status), vec![file("new.txt", true)]);
    }

    #[test]
    fn save_is_blocked_with_a_reason() {
        assert_eq!(save_blocker(true, true, true, None), None);
        assert_eq!(save_blocker(true, false, true, None), Some("Select files to stash.".to_string()));
        assert_eq!(save_blocker(false, false, true, None), Some("There are no changes to stash.".to_string()));
        assert_eq!(save_blocker(true, true, false, None), Some("Wait for the current Git action to finish.".to_string()));
        assert_eq!(
            save_blocker(true, true, true, Some(Operation::Merge)),
            Some("Finish or abort the merge in progress before stashing.".to_string())
        );
    }
}
