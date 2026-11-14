//! The notice along the bottom of the window when a newer NiceGit is out, and the update controls in
//! Settings.

use egui::RichText;
use egui_phosphor::regular as icon;

use crate::app::NiceGitApp;
use crate::install::{manual_update_hint, offers_install};
use crate::theme;
use crate::tools::widgets;
use crate::updates::{UpdateStatus, RELEASES_PAGE};

impl NiceGitApp {
    /// The slim bar along the bottom of the window, shown while a newer version is known and was
    /// not dismissed. It is declared before the panels, so it spans the window's full width.
    pub fn update_notice(&mut self, ui: &mut egui::Ui) {
        let Some(version) = self.pending_update() else { return };
        let c = theme::of(ui);
        let installed = self.installed_update.as_ref().is_some_and(|(installed, _)| *installed == version);
        let progress = self.install_progress();
        let can_install = self.install_allowed();
        let offers = offers_install(&version);
        let hint = manual_update_hint();
        let (mut download, mut dismiss, mut install, mut restart) = (false, false, false, false);
        let frame = egui::Frame::new().fill(c.subtle_bg).inner_margin(egui::Margin::symmetric(14, 5));
        egui::Panel::bottom("update_notice").frame(frame).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new(icon::CLOUD_ARROW_DOWN).color(c.accent));
                let text = match (&progress, installed, hint) {
                    (Some((_, stage)), _, _) => format!("NiceGit {version}: {stage}"),
                    (None, true, _) => format!("NiceGit {version} is installed."),
                    (None, false, Some(hint)) => format!("NiceGit {version} is available. {hint}"),
                    (None, false, None) => format!("NiceGit {version} is available"),
                };
                ui.label(text);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    dismiss = widgets::icon_button(ui, icon::X, "Dismiss", true).clicked();
                    if installed {
                        restart = ui.button("Restart NiceGit").clicked();
                    } else if progress.is_some() {
                        ui.spinner();
                    } else {
                        download = ui.button("Download").on_hover_text(RELEASES_PAGE).clicked();
                        if offers {
                            install = ui.add_enabled(can_install, egui::Button::new("Download and install")).clicked();
                        }
                    }
                });
            });
        });
        if download {
            crate::ui::changes::open_url(RELEASES_PAGE);
        }
        if dismiss {
            self.settings.dismissed_update = Some(version.clone());
        }
        if install {
            self.open_install_dialog(&version);
        }
        if restart {
            self.restart_after_install(ui.ctx());
        }
    }

    /// The Check now button and what the latest check the person asked for found.
    pub fn update_check_controls(&mut self, ui: &mut egui::Ui) {
        let c = theme::of(ui);
        let checking = self.update_check.is_some();
        let can_install = self.install_allowed();
        let (mut download, mut restart) = (false, false);
        let mut install_version = None;
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
                    let installed = self.installed_update.as_ref().is_some_and(|(installed, _)| *installed == version);
                    if let Some((_, stage)) = self.install_progress() {
                        ui.spinner();
                        ui.label(RichText::new(format!("NiceGit {version}: {stage}")).color(c.muted));
                    } else if installed {
                        ui.label(format!("NiceGit {version} is installed."));
                        restart = ui.button("Restart NiceGit").clicked();
                    } else {
                        ui.label(format!("NiceGit {version} is available."));
                        download = ui.button("Download").on_hover_text(RELEASES_PAGE).clicked();
                        if offers_install(&version) && ui.add_enabled(can_install, egui::Button::new("Download and install")).clicked() {
                            install_version = Some(version);
                        }
                    }
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
        if let Some(version) = install_version {
            self.open_install_dialog(&version);
        }
        if restart {
            self.restart_after_install(ui.ctx());
        }
    }
}
