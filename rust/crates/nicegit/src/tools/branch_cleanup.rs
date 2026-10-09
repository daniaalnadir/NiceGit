//! Clean up branches: lists local branches merged into the current branch, or with no commit for a
//! while, and deletes the chosen ones in one step that Undo can reverse.
#![allow(dead_code)]

use std::collections::BTreeSet;
use std::path::Path;
use std::sync::{Arc, Mutex};

use egui::{Margin, RichText, Stroke, Ui};
use egui_phosphor::regular as icon;
use nicegit_core::cleanup::{BranchCandidate, BranchDeletion};
use nicegit_core::{GitClient, GitError, Snapshot};

use crate::theme;
use crate::tools::reflog::age_text;
use crate::tools::{query, widgets, Ctx, Task, ToolWindow};

const DEFAULT_INACTIVE_DAYS: i64 = 90;

/// Branch names that are a repository's main line, whatever its settings.
const MAIN_LINE_NAMES: [&str; 4] = ["main", "master", "develop", "trunk"];

type Found = nicegit_core::Result<Vec<BranchCandidate>>;

/// The main line branches of a repository: the usual names, the branch each remote's HEAD points
/// to, and the GitFlow production and development branches. Reading the settings is best effort;
/// the usual names always count.
fn main_line_branches(client: &GitClient, remotes: &[String], directory: &Path) -> BTreeSet<String> {
    let mut names: BTreeSet<String> = MAIN_LINE_NAMES.iter().map(|name| name.to_string()).collect();
    if let Ok(Some(flow)) = client.gitflow_configuration(directory) {
        names.insert(flow.main_branch);
        names.insert(flow.develop_branch);
    }
    // Remote HEAD aliases are left out of the snapshot's branch list, so they are read here.
    if let Ok(output) = client.run(&["for-each-ref", "--format=%(refname)%09%(symref)", "refs/remotes/"], directory) {
        for line in output.lines() {
            let Some((refname, symref)) = line.split_once('\t') else { continue };
            if !refname.ends_with("/HEAD") || symref.is_empty() {
                continue;
            }
            // With remote names that contain slashes, the longest matching remote name gives the branch.
            let branch =
                remotes.iter().filter_map(|remote| symref.strip_prefix(&format!("refs/remotes/{remote}/"))).min_by_key(|rest| rest.len());
            if let Some(branch) = branch {
                names.insert(branch.to_string());
            }
        }
    }
    names
}

/// Deletions that Undo can restore, shared with the background action that made them.
type UndoSlot = Arc<Mutex<Option<Vec<BranchDeletion>>>>;

pub struct BranchCleanupWindow {
    /// The inactivity threshold the user has chosen, in days.
    inactive_days: i64,
    /// A listing in progress, and the threshold it was started with.
    listing: Option<Task<Found>>,
    listing_days: i64,
    /// The latest completed listing and the threshold it used.
    found: Option<Found>,
    found_days: Option<i64>,
    /// Names of the branches to delete.
    selected: BTreeSet<String>,
    confirming: bool,
    deleted: UndoSlot,
    /// The last deletion, which Undo restores.
    undo: Option<Vec<BranchDeletion>>,
    /// The main line branches, once read. Listed branches with these names are never preselected.
    main_line: Option<BTreeSet<String>>,
    main_line_task: Option<Task<nicegit_core::Result<BTreeSet<String>>>>,
}

impl Default for BranchCleanupWindow {
    fn default() -> Self {
        Self::new()
    }
}

impl BranchCleanupWindow {
    pub fn new() -> Self {
        Self {
            inactive_days: DEFAULT_INACTIVE_DAYS,
            listing: None,
            listing_days: DEFAULT_INACTIVE_DAYS,
            found: None,
            found_days: None,
            selected: BTreeSet::new(),
            confirming: false,
            deleted: Arc::new(Mutex::new(None)),
            undo: None,
            main_line: None,
            main_line_task: None,
        }
    }

    /// Whether `name` is a main line branch.
    fn is_main_line(&self, name: &str) -> bool {
        self.main_line.as_ref().is_some_and(|names| names.contains(name))
    }

    fn candidates(&self) -> &[BranchCandidate] {
        match &self.found {
            Some(Ok(candidates)) => candidates.as_slice(),
            _ => &[],
        }
    }
}

impl ToolWindow for BranchCleanupWindow {
    fn id(&self) -> String {
        "branch-cleanup".to_string()
    }

    fn title(&self) -> String {
        "Clean Up Branches".to_string()
    }

    fn default_size(&self) -> egui::Vec2 {
        egui::vec2(600.0, 560.0)
    }

    fn repository_changed(&mut self, _snapshot: &Snapshot) {
        // Read the list again; the rows already shown stay until the new list arrives.
        self.listing = None;
        self.found_days = None;
        // The remotes and GitFlow settings may have changed too.
        self.main_line = None;
        self.main_line_task = None;
    }

    fn ui(&mut self, ui: &mut Ui, cx: &mut Ctx) {
        // Deletions made by a finished action become available to Undo.
        if let Ok(mut slot) = self.deleted.lock() {
            if let Some(deletions) = slot.take() {
                // The deletions are also one step for the toolbar's Undo, as in the Mac app.
                let step = nicegit_core::undo::UndoStep::BranchDeletions(
                    deletions
                        .iter()
                        .map(|d| nicegit_core::undo::BranchDeletion {
                            name: d.name.clone(),
                            tip: d.tip.clone(),
                            upstream_remote: d.upstream_remote.clone(),
                            upstream_merge: d.upstream_merge.clone(),
                        })
                        .collect(),
                );
                cx.record_undo(format!("Clean up {} {}", deletions.len(), noun(deletions.len())), step);
                self.undo = Some(deletions);
                self.confirming = false;
            }
        }

        // Read the main line branches first, so a listing never preselects one of them.
        if self.main_line.is_none() && self.main_line_task.is_none() {
            let remotes = cx.snapshot.remotes.clone();
            self.main_line_task =
                Some(query(ui.ctx(), cx.repo, move |client, directory| Ok(main_line_branches(client, &remotes, directory))));
        }
        let ready = self.main_line_task.as_mut().and_then(|task| task.get().cloned());
        if let Some(Ok(names)) = ready {
            self.main_line = Some(names);
            self.main_line_task = None;
        }

        let mut slider_dragging = false;
        let c = theme::of(ui);
        let current = cx.snapshot.current_branch.as_str();
        ui.label(RichText::new(format!("Local branches merged into {current}, or with no commits for a while.")).small().color(c.muted));
        ui.add_space(6.0);

        ui.horizontal(|ui| {
            ui.label("Also list unmerged branches with no commit for");
            let response = ui.add(egui::Slider::new(&mut self.inactive_days, 7..=730).suffix(" days"));
            slider_dragging = response.dragged();
        });
        ui.add_space(6.0);

        // Start a listing when the threshold has changed and none is running. Waiting until the
        // slider is released avoids a listing for every value passed over while dragging.
        if self.listing.is_none() && self.found_days != Some(self.inactive_days) && !slider_dragging {
            let days = self.inactive_days;
            self.listing_days = days;
            self.listing = Some(query(ui.ctx(), cx.repo, move |client, directory| client.branch_cleanup_candidates(days, directory)));
        }
        // A listing waits for the main line branches, so they are known before anything is preselected.
        let finished = if self.main_line.is_some() { self.listing.as_mut().and_then(|task| task.get().cloned()) } else { None };
        if let Some(found) = finished {
            self.listing = None;
            self.found_days = Some(self.listing_days);
            if let Ok(candidates) = &found {
                // Merged branches start selected, except the main line; unmerged ones need a deliberate choice.
                let previous = std::mem::take(&mut self.selected);
                self.selected = candidates
                    .iter()
                    .filter(|candidate| (candidate.is_merged && !self.is_main_line(&candidate.name)) || previous.contains(&candidate.name))
                    .map(|candidate| candidate.name.clone())
                    .collect();
            }
            self.found = Some(found);
        }

        let idle = cx.idle;
        let selected_candidates: Vec<BranchCandidate> =
            self.candidates().iter().filter(|candidate| self.selected.contains(&candidate.name)).cloned().collect();
        let selected_unmerged = selected_candidates.iter().filter(|candidate| !candidate.is_merged).count();
        let selected_main = selected_candidates.iter().filter(|candidate| self.is_main_line(&candidate.name)).count();

        if let Some(deletions) = self.undo.clone() {
            egui::Frame::new().fill(c.subtle_bg).corner_radius(6.0).inner_margin(Margin::symmetric(10, 8)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.label(RichText::new(format!("{}  Deleted {} {}.", icon::TRASH, deletions.len(), noun(deletions.len()))));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if widgets::labeled_button(ui, icon::ARROW_COUNTER_CLOCKWISE, "Undo", idle).clicked() {
                            self.undo = None;
                            cx.act(format!("Restore {} {}", deletions.len(), noun(deletions.len())), move |client, directory| {
                                restore_all(client, &deletions, directory)
                            });
                        }
                    });
                });
            });
            ui.add_space(6.0);
        }

        match &self.found {
            None => widgets::loading(ui, "Finding branches…"),
            Some(Err(error)) => widgets::error(ui, &error.to_string()),
            Some(Ok(candidates)) if candidates.is_empty() => {
                widgets::empty_state(ui, icon::CHECK_CIRCLE, "No branches to clean up.");
            }
            Some(Ok(candidates)) => {
                let main_line = self.main_line.clone().unwrap_or_default();
                let merged: Vec<String> = candidates
                    .iter()
                    .filter(|candidate| candidate.is_merged && !self.is_main_line(&candidate.name))
                    .map(|candidate| candidate.name.clone())
                    .collect();
                ui.horizontal(|ui| {
                    widgets::section(ui, &format!("{} to review", candidates.len()));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.add_enabled(!merged.is_empty(), egui::Button::new("Select merged")).clicked() {
                            self.selected = merged.iter().cloned().collect();
                        }
                        if ui.add_enabled(!self.selected.is_empty(), egui::Button::new("Clear")).clicked() {
                            self.selected.clear();
                        }
                    });
                });
                egui::ScrollArea::vertical().id_salt("cleanup_candidates").max_height(300.0).auto_shrink([false, false]).show(ui, |ui| {
                    for candidate in candidates {
                        let is_main = main_line.contains(&candidate.name);
                        ui.horizontal(|ui| {
                            let mut checked = self.selected.contains(&candidate.name);
                            // A bordered box, so the checkbox stands out from the list.
                            let response = ui
                                .scope(|ui| {
                                    let visuals = ui.visuals_mut();
                                    for widget in [&mut visuals.widgets.inactive, &mut visuals.widgets.hovered, &mut visuals.widgets.active]
                                    {
                                        widget.bg_stroke = Stroke::new(1.0, c.muted);
                                    }
                                    ui.add(egui::Checkbox::new(&mut checked, RichText::new(&candidate.name).monospace()))
                                })
                                .inner
                                .on_hover_text(candidate.subject.as_str());
                            if response.changed() {
                                if checked {
                                    self.selected.insert(candidate.name.clone());
                                } else {
                                    self.selected.remove(&candidate.name);
                                }
                            }
                            if is_main {
                                widgets::pill(ui, "Main branch", c.modified)
                                    .on_hover_text("The main line is never selected automatically. Tick it only if you mean to delete it.");
                            }
                            let (label, fill) = if candidate.is_merged { ("Merged", c.added) } else { ("Not merged", c.conflict) };
                            widgets::pill(ui, label, fill);
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if let Some(time) = candidate.last_commit_time {
                                    ui.label(RichText::new(age_text(time)).small().color(c.muted));
                                }
                            });
                        });
                        ui.add_space(3.0);
                    }
                });
            }
        }

        ui.add_space(8.0);
        if selected_main > 0 {
            let (branches, verb) = if selected_main == 1 { ("branch", "is") } else { ("branches", "are") };
            widgets::callout(
                ui,
                &format!("{selected_main} selected {branches} {verb} the main line. Deleting it removes that line of work locally."),
                true,
            );
            ui.add_space(6.0);
        }
        if selected_unmerged > 0 {
            let (branches, verb, them) = if selected_unmerged == 1 { ("branch", "has", "it") } else { ("branches", "have", "them") };
            widgets::callout(
                ui,
                &format!(
                    "{selected_unmerged} selected {branches} {verb} commits on no other branch. Undo can restore {them} until another deletion."
                ),
                true,
            );
            ui.add_space(6.0);
        }

        if self.confirming {
            self.confirm(ui, cx, &selected_candidates, selected_unmerged > 0);
        } else {
            ui.horizontal(|ui| {
                ui.label(RichText::new(format!("{} selected", self.selected.len())).small().color(c.muted));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let label = format!("{}  Delete {} {}…", icon::TRASH, self.selected.len(), noun(self.selected.len()));
                    if widgets::danger_button(ui, &label, idle && !self.selected.is_empty()).clicked() {
                        self.confirming = true;
                    }
                });
            });
        }
    }
}

impl BranchCleanupWindow {
    /// The confirmation before deleting. Shown in place of the Delete button, so nothing is deleted
    /// by a single click.
    fn confirm(&mut self, ui: &mut Ui, cx: &mut Ctx, chosen: &[BranchCandidate], includes_unmerged: bool) {
        let c = theme::of(ui);
        let count = chosen.len();
        egui::Frame::new().fill(c.banner_bg).corner_radius(6.0).inner_margin(Margin::symmetric(12, 10)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(format!("Delete {count} local {}?", noun(count))).strong());
            let mut detail = String::from("Remote branches are not changed. ");
            if includes_unmerged {
                detail.push_str("Unmerged branches hold commits found on no other branch. ");
            }
            detail.push_str("Undo restores them until you take another undoable action.");
            ui.label(RichText::new(detail).small());
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if ui.button("Cancel").clicked() {
                    self.confirming = false;
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let label = if includes_unmerged { "Delete, including unmerged" } else { "Delete branches" };
                    if widgets::danger_button(ui, label, cx.idle).clicked() {
                        let chosen = chosen.to_vec();
                        let slot = Arc::clone(&self.deleted);
                        let total = chosen.len();
                        cx.act(format!("Delete {total} {}", noun(total)), move |client, directory| {
                            let report = client.delete_branches_keeping_undo(&chosen, includes_unmerged, directory);
                            let deleted = report.deleted.len();
                            if deleted > 0 {
                                if let Ok(mut slot) = slot.lock() {
                                    *slot = Some(report.deleted.clone());
                                }
                            }
                            match report.failure {
                                None => Ok(Some(format!("Deleted {deleted} {}. Undo restores them.", noun(deleted)))),
                                Some(error) => Err(GitError::failed(
                                    "delete branches",
                                    format!("Deleted {deleted} of {total} before stopping. Undo restores those. {error}"),
                                )),
                            }
                        });
                        self.selected.clear();
                        self.confirming = false;
                    }
                });
            });
        });
    }
}

/// Recreates each deleted branch. Stops at the first failure, reporting how many came back.
fn restore_all(client: &GitClient, deletions: &[BranchDeletion], directory: &Path) -> Result<Option<String>, GitError> {
    for (restored, deletion) in deletions.iter().enumerate() {
        if let Err(error) = client.restore_deleted_branch(deletion, directory) {
            return Err(GitError::failed(
                "restore branches",
                format!("Restored {restored} of {} before stopping. {error}", deletions.len()),
            ));
        }
    }
    Ok(Some(format!("Restored {} {}.", deletions.len(), noun(deletions.len()))))
}

fn noun(count: usize) -> &'static str {
    if count == 1 {
        "branch"
    } else {
        "branches"
    }
}
