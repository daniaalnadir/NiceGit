#![allow(dead_code)]
//! Submodules: their checkout state, opening them, and checking out the commit each one records.

use std::path::{Path, PathBuf};

use egui::{Context, RichText, Ui};
use egui_phosphor::regular as icon;
use nicegit_core::submodule::{Submodule, SubmoduleState};
use nicegit_core::Snapshot;

use crate::theme;
use crate::tools::{query, widgets, Ctx, Task, ToolWindow};

/// A button press on one submodule card, handled once the card is drawn.
enum SubmoduleAction {
    Open(PathBuf),
    CheckOut(String),
}

/// Lists the submodules recorded in the open repository, and checks out the commit each records.
/// The list is read in the background and read again whenever the repository changes.
pub struct SubmodulesWindow {
    repo: Option<PathBuf>,
    list: Option<Task<nicegit_core::Result<Vec<Submodule>>>>,
    submodules: Vec<Submodule>,
    error: Option<String>,
    stale: bool,
}

impl SubmodulesWindow {
    pub fn new() -> Self {
        Self { repo: None, list: None, submodules: Vec::new(), error: None, stale: true }
    }

    /// Starts reading the list when it is stale, and stores the result when it arrives.
    fn sync(&mut self, ctx: &Context, repo: &Path) {
        if self.repo.as_deref() != Some(repo) {
            self.repo = Some(repo.to_path_buf());
            self.list = None;
            self.submodules.clear();
            self.error = None;
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
                });
            });
        action
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
        if !idle {
            ui.add_space(4.0);
            widgets::loading(ui, "An action is running. Controls return when it finishes.");
        }
        ui.add_space(6.0);
        if let Some(error) = self.error.clone() {
            widgets::error(ui, &error);
            ui.add_space(6.0);
        }

        if loading && self.submodules.is_empty() {
            widgets::loading(ui, "Reading submodules");
            return;
        }
        if self.submodules.is_empty() && self.error.is_none() {
            widgets::empty_state(ui, icon::PACKAGE, "This repository has no submodules.");
            return;
        }

        let root = cx.repo.to_path_buf();
        let mut actions = Vec::new();
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.set_width(ui.available_width());
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
