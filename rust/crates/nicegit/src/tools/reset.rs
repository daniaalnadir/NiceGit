//! Reset confirmation: moves the current branch to a chosen commit, keeping or discarding changes.

#![allow(dead_code)]

use egui::RichText;
use egui_phosphor::regular as icon;
use nicegit_core::models::short;
use nicegit_core::{ResetMode, Snapshot};

use super::{widgets, Ctx, ToolWindow};
use crate::theme;

/// Confirms a reset of the branch that was checked out when the window opened. The branch and
/// HEAD are captured then, so the reset refuses if the checkout changed in the meantime.
pub struct ResetWindow {
    target: String,
    subject: String,
    branch: String,
    head: Option<String>,
    mode: ResetMode,
    closing: bool,
}

impl ResetWindow {
    pub fn new(target: String, subject: String, snapshot: &Snapshot) -> Self {
        Self {
            target,
            subject,
            branch: snapshot.current_branch.clone(),
            head: snapshot.head_hash.clone(),
            mode: ResetMode::Mixed,
            closing: false,
        }
    }

    fn short_target(&self) -> &str {
        short(&self.target)
    }
}

/// What a reset mode does, as shown beside its choice.
fn mode_title(mode: ResetMode) -> &'static str {
    match mode {
        ResetMode::Soft => "Soft: keep changes staged",
        ResetMode::Mixed => "Mixed: keep changes unstaged",
        ResetMode::Hard => "Hard: discard local changes",
    }
}

fn mode_detail(mode: ResetMode) -> &'static str {
    match mode {
        ResetMode::Soft => "Moves the branch without changing the index or working files. Changes from the removed commits remain staged.",
        ResetMode::Mixed => "Moves the branch and resets the index. Working files are kept, and the changes become unstaged.",
        ResetMode::Hard => {
            "Moves the branch and replaces the index and tracked files. Uncommitted tracked changes are lost. Untracked files that block restored paths may also be deleted."
        }
    }
}

/// One reset mode, as a selectable card with its explanation.
fn mode_card(ui: &mut egui::Ui, mode: &mut ResetMode, option: ResetMode) {
    let c = theme::of(ui);
    let selected = *mode == option;
    let is_hard = option == ResetMode::Hard;
    egui::Frame::new()
        .fill(if selected { c.subtle_bg } else { c.card_bg })
        .stroke(egui::Stroke::new(1.0, if selected { c.accent } else { c.border }))
        .corner_radius(6.0)
        .inner_margin(egui::Margin::symmetric(12, 10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            let title_color = if is_hard { c.danger } else { ui.visuals().text_color() };
            if ui.radio(selected, RichText::new(mode_title(option)).strong().color(title_color)).clicked() {
                *mode = option;
            }
            ui.add_space(2.0);
            ui.label(RichText::new(mode_detail(option)).small().color(c.muted));
        });
    ui.add_space(6.0);
}

impl ToolWindow for ResetWindow {
    fn id(&self) -> String {
        format!("reset:{}", self.target)
    }

    fn title(&self) -> String {
        format!("Reset to {}", self.short_target())
    }

    fn default_size(&self) -> egui::Vec2 {
        egui::vec2(600.0, 480.0)
    }

    fn ui(&mut self, ui: &mut egui::Ui, cx: &mut Ctx) {
        let c = theme::of(ui);
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            widgets::hash_label(ui, &self.target);
            ui.label(RichText::new(&self.subject).color(ui.visuals().text_color()).strong());
        });
        ui.label(RichText::new(format!("{}  Moves {} to this commit.", icon::ARROW_COUNTER_CLOCKWISE, self.branch)).small().color(c.muted));
        ui.add_space(10.0);

        widgets::section(ui, "Mode");
        ui.add_space(2.0);
        for option in [ResetMode::Soft, ResetMode::Mixed, ResetMode::Hard] {
            mode_card(ui, &mut self.mode, option);
        }

        if self.mode == ResetMode::Hard {
            widgets::callout(
                ui,
                "Hard reset discards uncommitted changes to tracked files. This cannot be undone from NiceGit's Changes panel.",
                true,
            );
            ui.add_space(6.0);
        }
        ui.label(
            RichText::new(
                "The branch will point to the selected commit. Commits outside its history will no longer be on this branch. Remote branches are not changed.",
            )
            .small()
            .color(c.muted),
        );

        ui.add_space(12.0);
        ui.separator();
        ui.add_space(8.0);
        let mut confirmed = false;
        ui.horizontal(|ui| {
            let enabled = cx.idle && self.head.is_some();
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                // Hard discards local changes, so it gets the danger style and a label that says so.
                let clicked = if self.mode == ResetMode::Hard {
                    widgets::danger_button(ui, "Reset and discard changes", enabled).clicked()
                } else {
                    widgets::primary_button(ui, "Reset branch", enabled).clicked()
                };
                confirmed = clicked;
                if ui.button("Cancel").clicked() {
                    self.closing = true;
                }
                if !cx.idle {
                    ui.spinner();
                    ui.label(RichText::new("Waiting for the current action").small().color(c.muted));
                }
            });
        });

        if confirmed {
            if let Some(head) = self.head.clone() {
                let (target, branch, mode) = (self.target.clone(), self.branch.clone(), self.mode);
                let short_target = self.short_target().to_string();
                let undo = nicegit_core::undo::UndoMode::for_reset(mode);
                cx.act_recording(format!("Reset {}", self.branch), "Reset", undo, move |client, repo| {
                    client.reset(&target, mode, &branch, &head, repo)?;
                    Ok(Some(format!("Reset {branch} to {short_target}.")))
                });
                self.closing = true;
            }
        }
    }

    fn wants_close(&self) -> bool {
        self.closing
    }
}
