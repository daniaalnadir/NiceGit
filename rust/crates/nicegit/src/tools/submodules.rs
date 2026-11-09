#![allow(dead_code)]
//! Submodules: their checkout state, opening them, checking out the commit each one records,
//! updating and syncing them, adding a new one, and removing one.

use std::path::{Path, PathBuf};

use egui::{Context, RichText, Ui};
use egui_phosphor::regular as icon;
use nicegit_core::submodule::{submodule_path_problem, Submodule, SubmoduleState};
use nicegit_core::Snapshot;

use crate::theme;
use crate::tools::{query, widgets, Ctx, Task, ToolWindow};

/// A button press on one submodule card, handled once the card is drawn.
enum SubmoduleAction {
    Open(PathBuf),
    CheckOut(String),
    AskRemove(String),
    CancelRemove,
    Remove(String),
}

/// Lists the submodules recorded in the open repository, checks out the commit each records,
/// updates, syncs, adds, and removes them. The list is read in the background and read again
/// whenever the repository changes.
pub struct SubmodulesWindow {
    repo: Option<PathBuf>,
    list: Option<Task<nicegit_core::Result<Vec<Submodule>>>>,
    submodules: Vec<Submodule>,
    error: Option<String>,
    stale: bool,
    /// Whether the add form is shown, and what it holds. The fields are kept after an add so a
    /// refused one can be corrected.
    adding: bool,
    url: String,
    path: String,
    branch: String,
    /// The submodule whose removal is waiting for confirmation.
    confirming_removal: Option<String>,
}

impl SubmodulesWindow {
    pub fn new() -> Self {
        Self {
            repo: None,
            list: None,
            submodules: Vec::new(),
            error: None,
            stale: true,
            adding: false,
            url: String::new(),
            path: String::new(),
            branch: String::new(),
            confirming_removal: None,
        }
    }

    /// Starts reading the list when it is stale, and stores the result when it arrives.
    fn sync(&mut self, ctx: &Context, repo: &Path) {
        if self.repo.as_deref() != Some(repo) {
            self.repo = Some(repo.to_path_buf());
            self.list = None;
            self.submodules.clear();
            self.error = None;
            self.confirming_removal = None;
            self.stale = true;
        }
        if let Some(task) = self.list.as_mut() {
            if let Some(result) = task.get().cloned() {
                self.list = None;
                match result {
                    Ok(submodules) => {
                        self.submodules = submodules;
                        self.error = None;
                    }
                    Err(error) => self.error = Some(error.to_string()),
                }
            }
        }
        if self.stale && self.list.is_none() {
            self.stale = false;
            self.list = Some(query(ctx, repo, |client, dir| client.submodules(dir)));
        }
    }

    fn submodule_card(&self, ui: &mut Ui, idle: bool, root: &Path, submodule: &Submodule) -> Option<SubmoduleAction> {
        let c = theme::of(ui);
        let mut action = None;
        let confirming = self.confirming_removal.as_deref() == Some(submodule.path.as_str());
        egui::Frame::new()
            .fill(c.card_bg)
            .stroke(egui::Stroke::new(1.0, c.border))
            .corner_radius(8.0)
            .inner_margin(egui::Margin::same(12))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new(icon::PACKAGE).color(c.muted));
                    ui.label(RichText::new(&submodule.path).monospace().strong());
                    match &submodule.state {
                        SubmoduleState::NotCheckedOut => {
                            widgets::pill(ui, "Not checked out", c.muted);
                        }
                        SubmoduleState::AtRecordedCommit => {
                            widgets::pill(ui, "At recorded commit", c.accent);
                        }
                        SubmoduleState::OnAnotherCommit(_) => {
                            widgets::pill(ui, "Different commit", c.warning);
                        }
                    }
                    if submodule.has_local_changes {
                        widgets::pill(ui, "Local changes", c.danger);
                    }
                });
                ui.label(
                    RichText::new(format!("Recorded  {}", short(&submodule.recorded_commit))).monospace().small().color(c.muted),
                );
                if let SubmoduleState::OnAnotherCommit(head) = &submodule.state {
                    ui.label(RichText::new(format!("Checked out  {}", short(head))).monospace().small().color(c.muted));
                }
                if submodule.has_local_changes {
                    ui.add_space(4.0);
                    widgets::callout(
                        ui,
                        "This submodule has uncommitted changes. Checking out the recorded commit keeps them when Git can, and Git refuses when they would be overwritten.",
                        true,
                    );
                }

                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    let checked_out = submodule.state != SubmoduleState::NotCheckedOut;
                    if ui.add_enabled(checked_out, egui::Button::new(format!("{}  Open", icon::ARROW_SQUARE_OUT))).clicked() {
                        action = Some(SubmoduleAction::Open(root.join(&submodule.path)));
                    }
                    let needs_checkout = submodule.state != SubmoduleState::AtRecordedCommit;
                    let label = if checked_out { "Check out recorded commit" } else { "Initialise and check out" };
                    let checkout = ui.add_enabled(idle && needs_checkout, egui::Button::new(format!("{}  {label}", icon::GIT_COMMIT)));
                    if checkout.clicked() {
                        action = Some(SubmoduleAction::CheckOut(submodule.path.clone()));
                    }
                    if !needs_checkout {
                        checkout.on_disabled_hover_text("Already at the recorded commit");
                    }
                    if !confirming {
                        let remove = ui.add_enabled(idle && !submodule.has_local_changes, egui::Button::new(format!("{}  Remove…", icon::TRASH)));
                        if remove.clicked() {
                            action = Some(SubmoduleAction::AskRemove(submodule.path.clone()));
                        }
                        if submodule.has_local_changes {
                            remove.on_disabled_hover_text("Commit or discard its changes before removing it");
                        }
                    }
                });

                if confirming {
                    ui.add_space(6.0);
                    widgets::callout(
                        ui,
                        "Removing deletes this submodule's checkout and stages the removal of its .gitmodules entry and its gitlink, so the change takes effect when you commit. The module's Git data in .git/modules is kept. Removal is refused while the checkout has uncommitted or ignored files.",
                        true,
                    );
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        if ui.button("Cancel").clicked() {
                            action = Some(SubmoduleAction::CancelRemove);
                        }
                        if widgets::danger_button(ui, &format!("{}  Remove submodule", icon::TRASH), idle).clicked() {
                            action = Some(SubmoduleAction::Remove(submodule.path.clone()));
                        }
                    });
                }
            });
        action
    }

    /// The form that adds a submodule: its URL, path inside the repository, and optional branch.
    fn add_form(&mut self, ui: &mut Ui, cx: &mut Ctx) {
        let c = theme::of(ui);
        let idle = cx.idle;
        widgets::section(ui, "Add a submodule");
        ui.label(
            RichText::new("Clones the repository at this URL into the path, then stages the new .gitmodules entry and gitlink for commit.")
                .color(c.muted),
        );
        ui.add_space(8.0);
        egui::Grid::new("submodule_add_form").num_columns(2).spacing([16.0, 10.0]).show(ui, |ui| {
            ui.label("Repository URL");
            ui.add_enabled(idle, egui::TextEdit::singleline(&mut self.url).hint_text("https://… or a local path").desired_width(360.0))
                .widget_info(|| egui::WidgetInfo::labeled(egui::Role::TextInput, true, "Repository URL"));
            ui.end_row();

            ui.label("Path");
            ui.add_enabled(idle, egui::TextEdit::singleline(&mut self.path).hint_text("vendor/library").desired_width(360.0))
                .widget_info(|| egui::WidgetInfo::labeled(egui::Role::TextInput, true, "Path"));
            ui.end_row();

            ui.label("Branch");
            ui.add_enabled(
                idle,
                egui::TextEdit::singleline(&mut self.branch).hint_text("Optional, the remote's default").desired_width(360.0),
            )
            .widget_info(|| egui::WidgetInfo::labeled(egui::Role::TextInput, true, "Branch"));
            ui.end_row();
        });

        let path = self.path.trim().to_string();
        let url = self.url.trim().to_string();
        let branch = self.branch.trim().to_string();
        let problem = if path.is_empty() { None } else { submodule_path_problem(cx.repo, &path) };
        ui.add_space(4.0);
        if let Some(problem) = &problem {
            widgets::error(ui, problem);
        }
        ui.add_space(8.0);
        let ready = idle && !url.is_empty() && !path.is_empty() && problem.is_none();
        if widgets::primary_button(ui, &format!("{}  Add submodule", icon::PLUS), ready).clicked() {
            let branch = (!branch.is_empty()).then_some(branch);
            let label = format!("Add submodule {path}");
            cx.act(label, move |client, dir| {
                client.add_submodule(&url, &path, branch.as_deref(), dir)?;
                Ok(Some(format!("Added {path}. Its .gitmodules entry and gitlink are staged for commit.")))
            });
        }
    }
}

impl Default for SubmodulesWindow {
    fn default() -> Self {
        Self::new()
    }
}

impl ToolWindow for SubmodulesWindow {
    fn id(&self) -> String {
        "submodules".to_string()
    }

    fn title(&self) -> String {
        "Submodules".to_string()
    }

    fn default_size(&self) -> egui::Vec2 {
        egui::vec2(660.0, 520.0)
    }

    fn ui(&mut self, ui: &mut Ui, cx: &mut Ctx) {
        let ctx = ui.ctx().clone();
        self.sync(&ctx, cx.repo);
        let c = theme::of(ui);
        let idle = cx.idle;
        let loading = self.list.is_some();
        let listed = !self.submodules.is_empty();

        ui.horizontal(|ui| {
            ui.label(
                RichText::new("Submodules are repositories nested in this one. Each is checked out at the commit this repository records.")
                    .color(c.muted),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button(format!("{}  Refresh", icon::ARROWS_CLOCKWISE)).clicked() {
                    self.stale = true;
                }
            });
        });
        ui.horizontal_wrapped(|ui| {
            if ui.add_enabled(idle && listed, egui::Button::new(format!("{}  Update all", icon::ARROWS_CLOCKWISE))).clicked() {
                cx.act("Update all submodules", |client, dir| {
                    client.update_submodules(dir)?;
                    Ok(Some("Checked out every submodule at its recorded commit.".to_string()))
                });
            }
            if ui.add_enabled(idle && listed, egui::Button::new(format!("{}  Sync URLs", icon::LINK))).clicked() {
                cx.act("Sync submodule URLs", |client, dir| {
                    client.sync_submodules(dir)?;
                    Ok(Some("Copied the submodule URLs from .gitmodules into their checkouts.".to_string()))
                });
            }
            let toggle = if self.adding { "Close" } else { "Add submodule…" };
            if ui.add_enabled(idle, egui::Button::new(format!("{}  {toggle}", icon::PLUS))).clicked() {
                self.adding = !self.adding;
            }
        });
        if !idle {
            ui.add_space(4.0);
            widgets::loading(ui, "An action is running. Controls return when it finishes.");
        }
        ui.add_space(6.0);
        if let Some(error) = self.error.clone() {
            widgets::error(ui, &error);
            ui.add_space(6.0);
        }

        let root = cx.repo.to_path_buf();
        let mut actions = Vec::new();
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.set_width(ui.available_width());
            if self.adding {
                self.add_form(ui, cx);
                ui.add_space(14.0);
            }
            if loading && self.submodules.is_empty() {
                widgets::loading(ui, "Reading submodules");
            } else if self.submodules.is_empty() && self.error.is_none() {
                widgets::empty_state(ui, icon::PACKAGE, "This repository has no submodules.");
            }
            for submodule in &self.submodules {
                if let Some(action) = self.submodule_card(ui, idle, &root, submodule) {
                    actions.push(action);
                }
                ui.add_space(6.0);
            }
        });
        for action in actions {
            match action {
                SubmoduleAction::Open(path) => cx.open_repository(path),
                SubmoduleAction::CheckOut(path) => {
                    cx.act(format!("Check out {path}"), move |client, dir| {
                        client.update_submodule(&path, dir)?;
                        Ok(Some(format!("Checked out the recorded commit in {path}.")))
                    });
                }
                SubmoduleAction::AskRemove(path) => self.confirming_removal = Some(path),
                SubmoduleAction::CancelRemove => self.confirming_removal = None,
                SubmoduleAction::Remove(path) => {
                    self.confirming_removal = None;
                    cx.act(format!("Remove submodule {path}"), move |client, dir| {
                        client.remove_submodule(&path, dir)?;
                        Ok(Some(format!("Removed {path}. Its .gitmodules entry and gitlink are staged for commit.")))
                    });
                }
            }
        }
    }

    fn repository_changed(&mut self, _snapshot: &Snapshot) {
        self.stale = true;
    }
}

/// A short commit ID for display.
fn short(hash: &str) -> &str {
    &hash[..hash.len().min(8)]
}
