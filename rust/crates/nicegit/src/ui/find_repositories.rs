//! Finding the repositories inside a folder, as Sourcetree's folder scan does. The scan runs on a
//! background thread; its results list each repository with a checkbox, and the chosen ones are
//! added to Recent or opened as tabs.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use egui::RichText;
use nicegit_core::discover::find_repositories;

use crate::app::NiceGitApp;
use crate::settings::RECENT_LIMIT;
use crate::theme;
use crate::tools::{widgets, Task};
use crate::ui::dialogs::Dialog;

/// How many folders below the chosen one the scan looks into.
pub const MAX_DEPTH: usize = 5;

/// The most repositories one scan lists; past this it stops and says so.
pub const MAX_REPOSITORIES: usize = 1000;

/// Above this many chosen repositories, opening them as tabs warns that it takes a while.
const MANY_TABS: usize = 10;

/// One repository the scan found.
pub struct FoundRepository {
    pub path: PathBuf,
    /// The path relative to the chosen folder, as shown in the list.
    pub label: String,
    pub checked: bool,
}

/// A scan running in the background. Dropping it asks the scan to stop, so closing the dialog
/// (by Cancel, Escape, or clicking away) stops the search.
pub struct ScanJob {
    cancel: Arc<AtomicBool>,
    task: Task<Vec<PathBuf>>,
}

impl Drop for ScanJob {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

pub enum FindState {
    Scanning(ScanJob),
    Done { found: Vec<FoundRepository>, limited: bool },
}

/// What the person chose in the scan dialog this frame.
pub enum Choice {
    Pending,
    Close,
    Add,
}

/// Moves a finished scan to its results, with every repository checked.
fn poll(state: &mut FindState, folder: &Path) {
    let FindState::Scanning(job) = state else { return };
    let Some(paths) = job.task.get().cloned() else { return };
    let limited = paths.len() >= MAX_REPOSITORIES;
    let found = paths
        .into_iter()
        .map(|path| {
            let relative = path.strip_prefix(folder).unwrap_or(&path);
            let label = if relative.as_os_str().is_empty() {
                path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default()
            } else {
                relative.display().to_string()
            };
            FoundRepository { path, label, checked: true }
        })
        .collect();
    *state = FindState::Done { found, limited };
}

/// The scan dialog for `folder`, which starts looking at once.
fn scanning(ctx: &egui::Context, folder: PathBuf) -> FindState {
    let cancel = Arc::new(AtomicBool::new(false));
    let flag = cancel.clone();
    let search = folder.clone();
    let task = Task::spawn(ctx, move || find_repositories(&search, MAX_DEPTH, MAX_REPOSITORIES, &flag));
    FindState::Scanning(ScanJob { cancel, task })
}

/// Draws the scan dialog's body: the search in progress, or its results.
pub fn show(ui: &mut egui::Ui, folder: &Path, state: &mut FindState, open_tabs: &mut bool) -> Choice {
    let c = theme::of(ui);
    poll(state, folder);
    match state {
        FindState::Scanning(_) => {
            ui.heading("Find repositories");
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("Looking for repositories…");
            });
            ui.add_space(16.0);
            if ui.button("Cancel").clicked() {
                return Choice::Close;
            }
            Choice::Pending
        }
        FindState::Done { found, limited } => {
            if found.is_empty() {
                ui.heading("No repositories found");
                ui.add_space(8.0);
                ui.label(format!("No repositories found in {}.", folder.display()));
                ui.add_space(16.0);
                if ui.button("Close").clicked() {
                    return Choice::Close;
                }
                return Choice::Pending;
            }
            let noun = if found.len() == 1 { "repository" } else { "repositories" };
            ui.heading(format!("Found {} {noun} in {}", found.len(), folder.display()));
            if *limited {
                ui.add_space(4.0);
                ui.label(
                    RichText::new(format!("Showing the first {MAX_REPOSITORIES}. Choose a folder inside this one to find the rest."))
                        .small()
                        .color(c.muted),
                );
            }
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button("Select all").clicked() {
                    found.iter_mut().for_each(|repository| repository.checked = true);
                }
                if ui.button("Select none").clicked() {
                    found.iter_mut().for_each(|repository| repository.checked = false);
                }
            });
            egui::ScrollArea::vertical().max_height(320.0).auto_shrink([false, true]).show(ui, |ui| {
                for repository in found.iter_mut() {
                    ui.checkbox(&mut repository.checked, repository.label.as_str());
                }
            });
            ui.add_space(8.0);
            ui.checkbox(open_tabs, "Also open them as tabs");
            let chosen = found.iter().filter(|repository| repository.checked).count();
            if *open_tabs && chosen > MANY_TABS {
                ui.label(RichText::new("Opening many tabs reads each repository in the background.").small().color(c.muted));
            }
            ui.add_space(16.0);
            let mut choice = Choice::Pending;
            ui.horizontal(|ui| {
                if widgets::primary_button(ui, &format!("Add {chosen} to Recent"), chosen > 0).clicked() {
                    choice = Choice::Add;
                }
                if ui.button("Cancel").clicked() {
                    choice = Choice::Close;
                }
            });
            choice
        }
    }
}

/// The body of the offer shown when a folder that is not a repository is dropped on the window.
pub fn show_offer(ui: &mut egui::Ui) -> Choice {
    ui.heading("Not a repository");
    ui.add_space(8.0);
    ui.label("This folder isn't a repository. Look for repositories inside it?");
    ui.add_space(16.0);
    let mut choice = Choice::Pending;
    ui.horizontal(|ui| {
        if widgets::primary_button(ui, "Find", true).clicked() {
            choice = Choice::Add;
        }
        if ui.button("Cancel").clicked() {
            choice = Choice::Close;
        }
    });
    choice
}

impl NiceGitApp {
    /// Asks for a folder to search, then starts searching it.
    pub fn choose_folder_to_find(&mut self, ctx: &egui::Context) {
        if let Some(folder) = crate::file_dialog::pick_folder("Find repositories in a folder") {
            self.start_find(ctx, folder);
        }
    }

    /// Opens the scan dialog for `folder`, which starts searching it in the background.
    pub fn start_find(&mut self, ctx: &egui::Context, folder: PathBuf) {
        let state = scanning(ctx, folder.clone());
        self.dialog = Some(Dialog::FindRepositories { folder, state, open_tabs: false });
    }

    /// Adds the chosen repositories to Recent, and opens them as tabs when asked. Recent keeps
    /// only its newest repositories, so the notice says how many of the chosen ones were added.
    pub fn add_found_repositories(&mut self, chosen: Vec<PathBuf>, open_tabs: bool) {
        let added = chosen.len().min(RECENT_LIMIT);
        // Remembering puts a repository at the top, so the last is remembered first and the
        // first chosen ends up on top.
        for path in chosen.iter().take(added).rev() {
            self.settings.remember(path.clone());
        }
        let mut text = if added == chosen.len() {
            format!("Added {added} to Recent.")
        } else {
            format!("Added {added} of {} to Recent. Recent keeps only the {RECENT_LIMIT} newest.", chosen.len())
        };
        if open_tabs {
            if self.busy.is_some() {
                text.push_str(" The repositories were not opened as tabs, because an action is running.");
            } else if self.has_unsaved_editor() {
                text.push_str(" The repositories were not opened as tabs, because a file is being edited.");
            } else {
                // The tabs are added without reading them: the first is switched to and read now,
                // and the background reader reads the others a couple at a time, so choosing many
                // does not start many full reads at once.
                let count = chosen.len();
                let mut first = None;
                for path in chosen {
                    let path = crate::app::strip_verbatim(path.canonicalize().unwrap_or(path));
                    let index = match self.repos.iter().position(|repo| repo.path == path) {
                        Some(index) => index,
                        None => {
                            self.repos.push(crate::app::Repo::new(path));
                            self.repos.len() - 1
                        }
                    };
                    first.get_or_insert(index);
                }
                if let Some(index) = first {
                    self.switch_to(index);
                }
                text.push_str(&format!(" Opened {count} as tabs."));
            }
        }
        self.notify(text, false);
    }
}
