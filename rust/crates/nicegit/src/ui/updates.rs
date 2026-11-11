//! The notice along the bottom of the window when a newer NiceGit is out, and the update controls in
//! Settings.

use egui::RichText;
use egui_phosphor::regular as icon;

use crate::app::NiceGitApp;
use crate::theme;
use crate::tools::widgets;
use crate::updates::{UpdateStatus, RELEASES_PAGE};

impl NiceGitApp {
    /// The slim bar along the bottom of the window, shown while a newer version is known and was
    /// not dismissed. It is declared before the panels, so it spans the window's full width.
    pub fn update_notice(&mut self, ui: &mut egui::Ui) {
        let Some(version) = self.pending_update() else { return };
        let c = theme::of(ui);
        let (mut download, mut dismiss) = (false, false);
        let frame = egui::Frame::new().fill(c.subtle_bg).inner_margin(egui::Margin::symmetric(14, 5));
        egui::Panel::bottom("update_notice").frame(frame).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new(icon::CLOUD_ARROW_DOWN).color(c.accent));
                ui.label(format!("NiceGit {version} is available"));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    dismiss = widgets::icon_button(ui, icon::X, "Dismiss", true).clicked();
                    download = ui.button("Download").on_hover_text(RELEASES_PAGE).clicked();
                });
            });
        });
        if download {
            crate::ui::changes::open_url(RELEASES_PAGE);
        }
        if dismiss {
            self.settings.dismissed_update = Some(version);
        }
    }

    /// The Check now button and what the latest check the person asked for found.
    pub fn update_check_controls(&mut self, ui: &mut egui::Ui) {
        let c = theme::of(ui);
        let checking = self.update_check.is_some();
        let mut download = false;
        ui.horizontal(|ui| {
            if ui.add_enabled(!checking, egui::Button::new("Check now")).clicked() {
                self.start_update_check(true);
            }
            match self.update_status.clone() {
                Some(UpdateStatus::Checking) => {
                    ui.spinner();
                    ui.label(RichText::new("Checking…").color(c.muted));
                }
                Some(UpdateStatus::UpToDate) => {
                    ui.label(RichText::new("NiceGit is up to date.").color(c.muted));
                }
                Some(UpdateStatus::Available(version)) => {
                    ui.label(format!("NiceGit {version} is available."));
                    download = ui.button("Download").on_hover_text(RELEASES_PAGE).clicked();
                }
                Some(UpdateStatus::Failed(reason)) => {
                    ui.label(RichText::new(format!("Could not check: {reason}")).color(c.warning));
                }
                None => {}
            }
        });
        if download {
            crate::ui::changes::open_url(RELEASES_PAGE);
        }
    }
}
