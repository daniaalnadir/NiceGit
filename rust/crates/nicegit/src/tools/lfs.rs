//! Git LFS: which file patterns are stored with LFS, which files it holds, and whether their
//! content is on this machine.
#![allow(dead_code)]

use egui::{Margin, RichText, Ui};
use egui_phosphor::regular as icon;
use nicegit_core::lfs::{LfsLock, LfsLocks, LfsStatus, LockTarget};
use nicegit_core::{GitError, Snapshot};

use crate::theme;
use crate::tools::{blame::relative_time, query, widgets, Ctx, Task, ToolWindow};

type Loaded = nicegit_core::Result<LfsStatus>;
type LocksLoaded = nicegit_core::Result<LfsLocks>;

#[derive(Default)]
pub struct LfsWindow {
    /// The status, read in the background. None until read, and again after the repository changes.
    status: Option<Task<Loaded>>,
    /// The last status read, shown while a newer one loads so the window does not flicker.
    shown: Option<Loaded>,
    /// The pattern typed for tracking.
    pattern: String,
    /// The pattern a track was requested for; the field is cleared once it appears in the list.
    tracking: Option<String>,
    /// The locks on the remote, read in the background. Read when the window opens, on Refresh,
    /// and after a lock changes; a repository refresh alone does not contact the server.
    locks: Option<Task<LocksLoaded>>,
    /// The last locks read, shown while a newer read loads.
    shown_locks: Option<LocksLoaded>,
    /// A lock held by someone else, waiting for its force unlock to be confirmed.
    force_unlock: Option<LfsLock>,
}

/// A lock message for the user: Git's own text, without the command prefix.
fn lock_message(error: &GitError) -> String {
    match error {
        GitError::CommandFailed { message, .. } => message.clone(),
        other => other.to_string(),
    }
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
        if self.locks.is_none() {
            self.locks = Some(query(ui.ctx(), cx.repo, |client, directory| client.list_locks(directory)));
        }
        if let Some(state) = self.locks.as_mut().and_then(|task| task.get()) {
            self.shown_locks = Some(state.clone());
        }
        let idle = cx.idle;
        let c = theme::of(ui);
        let mut reload = false;
        let mut reload_locks = false;
        let mut track: Option<String> = None;
        let mut untrack: Option<String> = None;
        // The unlock to start: the lock's target, whether it is forced, and the path for the notice.
        let mut unlock: Option<(LockTarget, bool, String)> = None;

        ui.horizontal(|ui| {
            ui.label(RichText::new(icon::CUBE).size(16.0).color(c.accent));
            ui.label(
                RichText::new("Stores large files outside Git's history. Git keeps a small pointer in their place.").small().color(c.muted),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if widgets::icon_button(ui, icon::ARROW_CLOCKWISE, "Read Git LFS again", true).clicked() {
                    reload = true;
                }
            });
        });
        ui.add_space(8.0);

        egui::ScrollArea::vertical().id_salt("lfs_body").auto_shrink([false, false]).show(ui, |ui| {
            if let Some(state) = self.status.as_mut().and_then(|task| task.get()) {
                self.shown = Some(state.clone());
            }
            let Some(state) = &self.shown else {
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
                .widget_info(|| egui::WidgetInfo::labeled(egui::Role::TextInput, true, "LFS pattern"));
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

            ui.add_space(12.0);
            ui.horizontal(|ui| {
                widgets::section(ui, "Locks");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if widgets::labeled_button(ui, icon::ARROW_CLOCKWISE, "Refresh", true)
                        .on_hover_text("Read the locks from the remote again")
                        .clicked()
                    {
                        reload_locks = true;
                    }
                });
            });
            ui.add_space(4.0);
            match self.shown_locks.clone() {
                None => widgets::loading(ui, "Reading file locks…"),
                Some(Err(error)) => widgets::callout(ui, &lock_message(&error), true),
                Some(Ok(locks)) => {
                    if !locks.verified {
                        let who = if locks.user_name.is_empty() {
                            "No user.name is set, so no lock is shown as yours.".to_string()
                        } else {
                            format!("The server could not confirm ownership, so locks are yours when their owner is {}.", locks.user_name)
                        };
                        ui.label(RichText::new(who).small().color(c.muted));
                        ui.add_space(4.0);
                    }
                    if locks.locks.is_empty() {
                        ui.label(RichText::new("No files are locked.").small().color(c.muted));
                    }
                    for lock in &locks.locks {
                        ui.horizontal(|ui| {
                            ui.add(egui::Label::new(RichText::new(&lock.path).monospace()).truncate());
                            let age = lock.locked_at.map(relative_time).unwrap_or_default();
                            ui.label(RichText::new(format!("{}  {age}", lock.owner)).small().color(c.muted));
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if lock.ours {
                                    if ui.add_enabled(idle, egui::Button::new("Unlock")).clicked() {
                                        let target = LockTarget::Id(lock.id.clone());
                                        unlock = Some((target, false, lock.path.clone()));
                                    }
                                } else if ui
                                    .add_enabled(idle, egui::Button::new("Force unlock…"))
                                    .on_hover_text("Release a lock another person holds")
                                    .clicked()
                                {
                                    self.force_unlock = Some(lock.clone());
                                }
                            });
                        });
                    }
                    if let Some(lock) = self.force_unlock.clone() {
                        let owner = if locks.verified {
                            "The server confirmed the owner.".to_string()
                        } else {
                            format!("Ownership was compared with your user.name, \"{}\".", locks.user_name)
                        };
                        egui::Frame::new().fill(c.subtle_bg).corner_radius(6.0).inner_margin(Margin::symmetric(10, 8)).show(ui, |ui| {
                            ui.label(format!(
                                "Force unlock {}? It is locked by {}. They may lose unsaved work they meant to upload. {owner}",
                                lock.path, lock.owner
                            ));
                            ui.horizontal(|ui| {
                                if widgets::danger_button(ui, "Force unlock", idle).clicked() {
                                    self.force_unlock = None;
                                    unlock = Some((LockTarget::Id(lock.id.clone()), true, lock.path.clone()));
                                }
                                if ui.button("Cancel").clicked() {
                                    self.force_unlock = None;
                                }
                            });
                        });
                    }
                }
            }
        });

        if reload {
            self.status = None;
        }
        if reload_locks {
            self.locks = None;
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
        if let Some((target, force, path)) = unlock {
            // The lock list is read again once the change is made.
            self.locks = None;
            let label = if force { "Force unlock" } else { "Unlock" };
            cx.act(format!("{label} {path}"), move |client, directory| {
                client.unlock(&target, force, directory).map(|()| Some(format!("Unlocked {path}.")))
            });
        }
    }
}
