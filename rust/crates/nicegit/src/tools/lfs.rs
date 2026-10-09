//! Git LFS: which file patterns are stored with LFS, which files it holds, and whether their
//! content is on this machine.
#![allow(dead_code)]

use egui::{Margin, RichText, Ui};
use egui_phosphor::regular as icon;
use nicegit_core::lfs::LfsStatus;
use nicegit_core::Snapshot;

use crate::theme;
use crate::tools::{query, widgets, Ctx, Task, ToolWindow};

type Loaded = nicegit_core::Result<LfsStatus>;

#[derive(Default)]
pub struct LfsWindow {
    /// The status, read in the background. None until read, and again after the repository changes.
    status: Option<Task<Loaded>>,
    /// The pattern typed for tracking.
    pattern: String,
    /// The pattern a track was requested for; the field is cleared once it appears in the list.
    tracking: Option<String>,
}

impl LfsWindow {
    pub fn new() -> Self {
        Self::default()
    }
}

impl ToolWindow for LfsWindow {
    fn id(&self) -> String {
        "lfs".to_string()
    }

    fn title(&self) -> String {
        "Git LFS".to_string()
    }

    fn default_size(&self) -> egui::Vec2 {
        egui::vec2(560.0, 560.0)
    }

    fn repository_changed(&mut self, _snapshot: &Snapshot) {
        self.status = None;
    }

    fn ui(&mut self, ui: &mut Ui, cx: &mut Ctx) {
        if self.status.is_none() {
            self.status = Some(query(ui.ctx(), cx.repo, |client, directory| client.lfs_status(directory)));
        }
        let idle = cx.idle;
        let c = theme::of(ui);
        let mut reload = false;
        let mut track: Option<String> = None;
        let mut untrack: Option<String> = None;

        ui.horizontal(|ui| {
            ui.label(RichText::new(icon::CUBE).size(22.0).color(c.accent));
            ui.vertical(|ui| {
                ui.heading("Git LFS");
                ui.label(
                    RichText::new("Stores large files outside Git's history. Git keeps a small pointer in their place.")
                        .small()
                        .color(c.muted),
                );
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if widgets::icon_button(ui, icon::ARROW_CLOCKWISE, "Read Git LFS again", true).clicked() {
                    reload = true;
                }
            });
        });
        ui.add_space(8.0);

        egui::ScrollArea::vertical().id_salt("lfs_body").auto_shrink([false, false]).show(ui, |ui| {
            let state = self.status.as_mut().and_then(|task| task.get());
            let Some(state) = state else {
                widgets::loading(ui, "Reading Git LFS status…");
                return;
            };
            let status = match state {
                Ok(status) => status,
                Err(error) => {
                    widgets::error(ui, &error.to_string());
                    return;
                }
            };

            if let Some(version) = &status.version {
                ui.label(RichText::new(format!("{}  Installed, {version}", icon::CHECK_CIRCLE)).small().color(c.added));
            } else {
                widgets::callout(
                    ui,
                    "Git LFS is not installed, so new files cannot be stored with it. Install it from https://git-lfs.com, then run `git lfs install` once.",
                    true,
                );
            }
            ui.add_space(10.0);

            widgets::section(ui, "Tracked patterns");
            ui.add_space(4.0);
            if status.patterns.is_empty() {
                ui.label(RichText::new("No patterns yet. Files that match a pattern are stored with Git LFS.").small().color(c.muted));
            }
            for pattern in &status.patterns {
                egui::Frame::new()
                    .fill(c.subtle_bg)
                    .corner_radius(6.0)
                    .inner_margin(Margin::symmetric(10, 5))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(pattern).monospace());
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                let button = ui
                                    .add_enabled(idle, egui::Button::new("Untrack"))
                                    .on_hover_text("Store new versions of matching files in Git itself. Files already committed are unchanged.");
                                if button.clicked() {
                                    untrack = Some(pattern.clone());
                                }
                            });
                        });
                    });
                ui.add_space(3.0);
            }

            ui.add_space(10.0);
            widgets::section(ui, "Track a pattern");
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                let width = (ui.available_width() - 90.0).max(140.0);
                ui.add_enabled(
                    idle,
                    egui::TextEdit::singleline(&mut self.pattern).hint_text("*.psd, docs/*.pdf").desired_width(width),
                )
                .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, "LFS pattern"));
                let can_track = idle && status.version.is_some() && !self.pattern.trim().is_empty();
                if widgets::primary_button(ui, "Track", can_track).clicked() {
                    track = Some(self.pattern.trim().to_string());
                }
            });
            ui.add_space(4.0);
            ui.label(
                RichText::new("Tracking changes .gitattributes. Commit that file so everyone stores these files the same way.")
                    .small()
                    .color(c.muted),
            );

            // Clear the field once its pattern is in the list.
            if let Some(tracking) = &self.tracking {
                if status.patterns.iter().any(|pattern| pattern == tracking) {
                    self.pattern.clear();
                    self.tracking = None;
                }
            }

            ui.add_space(12.0);
            widgets::section(ui, &format!("Files in Git LFS ({})", status.files.len()));
            ui.add_space(4.0);
            if status.files.is_empty() {
                ui.label(RichText::new("No tracked files use Git LFS yet.").small().color(c.muted));
            }
            for file in &status.files {
                ui.horizontal(|ui| {
                    ui.add(egui::Label::new(RichText::new(&file.path).monospace()).truncate());
                    if file.is_pointer_only {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            widgets::pill(ui, "Not downloaded", c.conflict).on_hover_text(
                                "Only the LFS pointer is here. Install Git LFS and pull to download the content.",
                            );
                        });
                    }
                });
            }
        });

        if reload {
            self.status = None;
        }
        if let Some(pattern) = track {
            self.tracking = Some(pattern.clone());
            cx.act(format!("Track {pattern}"), move |client, directory| {
                client.track_lfs(&pattern, directory).map(|()| Some(format!("Tracking {pattern}.")))
            });
        }
        if let Some(pattern) = untrack {
            cx.act(format!("Untrack {pattern}"), move |client, directory| {
                client.untrack_lfs(&pattern, directory).map(|()| Some(format!("No longer tracking {pattern}.")))
            });
        }
    }
}
