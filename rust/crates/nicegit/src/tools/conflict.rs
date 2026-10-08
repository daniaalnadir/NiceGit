//! The conflict editor. The base, current, and incoming versions of a conflicted file show
//! read-only above an editable result. Saving checks that no conflict markers remain, and
//! whole-file resolutions take one side or delete the file.

#![allow(dead_code)]

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use egui::{RichText, ScrollArea, TextEdit, Ui};
use nicegit_core::conflict::{has_conflict_markers, ConflictDocument};
use nicegit_core::{GitClient, Operation};

use crate::tools::{query, widgets, Ctx, Task, ToolWindow};

/// One of the versions shown beside the result.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Version {
    Base,
    Current,
    Incoming,
}

/// A whole-file resolution that replaces the working file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WholeFile {
    Current,
    Incoming,
    Delete,
}

/// A choice waiting for the user's confirmation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Pending {
    /// Replace the result with one version's text, discarding edits.
    Copy(Version),
    /// Resolve the whole file, discarding edits.
    Whole(WholeFile),
    /// Close the editor, discarding edits.
    Close,
}

/// The conflict editor for one conflicted file.
pub struct ConflictWindow {
    path: String,
    /// Reads the file and its versions in the background, once.
    load: Option<Task<nicegit_core::Result<ConflictDocument>>>,
    document: Option<ConflictDocument>,
    load_error: Option<String>,
    /// The editable result, which starts as the file's current text.
    result: String,
    pending: Option<Pending>,
    /// Set by a background resolution that succeeded, so the window closes after it.
    resolved: Arc<AtomicBool>,
    close_requested: bool,
}

impl ConflictWindow {
    /// The editor for the conflicted file at `path`, relative to the repository root.
    pub fn new(path: String) -> Self {
        Self {
            path,
            load: None,
            document: None,
            load_error: None,
            result: String::new(),
            pending: None,
            resolved: Arc::new(AtomicBool::new(false)),
            close_requested: false,
        }
    }
}

impl ToolWindow for ConflictWindow {
    fn id(&self) -> String {
        format!("conflict:{}", self.path)
    }

    fn title(&self) -> String {
        format!("Resolve conflict: {}", self.path)
    }

    fn default_size(&self) -> egui::Vec2 {
        egui::vec2(980.0, 660.0)
    }

    fn ui(&mut self, ui: &mut Ui, cx: &mut Ctx) {
        if self.load.is_none() && self.document.is_none() {
            let path = self.path.clone();
            self.load = Some(query(ui.ctx(), cx.repo, move |client, repo| client.load_conflict(&path, repo)));
        }
        if self.document.is_none() && self.load_error.is_none() {
            if let Some(loaded) = self.load.as_mut().and_then(|task| task.get()) {
                match loaded {
                    Ok(document) => {
                        self.result = document.content.clone();
                        self.document = Some(document.clone());
                    }
                    Err(error) => self.load_error = Some(error.to_string()),
                }
            }
        }

        ui.label(RichText::new(&self.path).strong());
        if let Some(error) = &self.load_error {
            widgets::error(ui, error);
            return;
        }
        let Some(document) = self.document.as_ref() else {
            widgets::loading(ui, "Reading the conflicted file...");
            return;
        };

        let idle = cx.idle;
        let rebase = cx.snapshot.operation == Some(Operation::Rebase);
        let (current, incoming) = if rebase { ("Destination", "Replayed commit") } else { ("Current", "Incoming") };
        let markers = has_conflict_markers(&self.result, document.marker_size);
        let dirty = self.result != document.content;

        // Read-only versions, side by side. Each can be copied into the result.
        let versions_height = (ui.available_height() * 0.34).clamp(110.0, 250.0);
        let panes = [
            ("Base", document.base.as_deref(), Version::Base),
            (current, document.current.as_deref(), Version::Current),
            (incoming, document.incoming.as_deref(), Version::Incoming),
        ];
        let mut copy_from: Option<Version> = None;
        ui.columns(3, |columns| {
            for (column, (label, text, version)) in columns.iter_mut().zip(panes) {
                if version_pane(column, label, text, versions_height, idle) {
                    copy_from = Some(version);
                }
            }
        });
        if let Some(version) = copy_from {
            self.pending = Some(Pending::Copy(version));
        }

        ui.add_space(6.0);
        ui.label(RichText::new("Result").strong());
        if markers {
            widgets::callout(ui, "The result still has conflict markers. Remove them before saving.", true);
        }
        // The editor wraps to the window's width, so only the height needs to scroll.
        ScrollArea::vertical().id_salt(("conflict-result", self.path.as_str())).auto_shrink([false, false]).show(ui, |ui| {
            let width = ui.available_width();
            ui.add(TextEdit::multiline(&mut self.result).code_editor().desired_width(width).desired_rows(14));
        });

        // A confirmation for the choice waiting on the user, if any.
        if let Some(pending) = self.pending {
            let (message, label, enabled) = match pending {
                Pending::Copy(version) => {
                    let name = match version {
                        Version::Base => "base",
                        Version::Current => current,
                        Version::Incoming => incoming,
                    };
                    (
                        format!("Replace the result with the {} version? Edits in the result are lost.", name.to_lowercase()),
                        "Use version",
                        true,
                    )
                }
                Pending::Whole(WholeFile::Current) => (
                    format!(
                        "Take the {} version of the whole file? This replaces the working file and discards editor changes.",
                        current.to_lowercase()
                    ),
                    "Take version",
                    idle,
                ),
                Pending::Whole(WholeFile::Incoming) => (
                    format!(
                        "Take the {} version of the whole file? This replaces the working file and discards editor changes.",
                        incoming.to_lowercase()
                    ),
                    "Take version",
                    idle,
                ),
                Pending::Whole(WholeFile::Delete) => {
                    ("Delete this file? This removes the working file and discards editor changes.".to_string(), "Delete file", idle)
                }
                Pending::Close => ("Close the editor and discard its changes?".to_string(), "Discard changes", true),
            };
            widgets::callout(ui, &message, true);
            let mut confirmed = false;
            let mut cancelled = false;
            ui.horizontal(|ui| {
                let delete = matches!(pending, Pending::Whole(WholeFile::Delete) | Pending::Close);
                let confirm = if delete {
                    widgets::danger_button(ui, label, enabled).clicked()
                } else {
                    widgets::primary_button(ui, label, enabled).clicked()
                };
                confirmed = confirm;
                cancelled = ui.button("Cancel").clicked();
            });
            if confirmed {
                match pending {
                    Pending::Copy(version) => {
                        if let Some(text) = version_text(document, version) {
                            self.result = text;
                        }
                    }
                    Pending::Whole(choice) => {
                        whole_file(self.path.clone(), Arc::clone(&self.resolved), choice, cx);
                    }
                    Pending::Close => self.close_requested = true,
                }
                self.pending = None;
            } else if cancelled {
                self.pending = None;
            }
        }

        // Whole-file resolutions and the commands that finish the conflict.
        let mut whole: Option<WholeFile> = None;
        let mut save = false;
        let mut close = false;
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new("Whole file").weak());
            if ui.add_enabled(idle, egui::Button::new(format!("Take {}", current.to_lowercase()))).clicked() {
                whole = Some(WholeFile::Current);
            }
            if ui.add_enabled(idle, egui::Button::new(format!("Take {}", incoming.to_lowercase()))).clicked() {
                whole = Some(WholeFile::Incoming);
            }
            if widgets::danger_button(ui, "Delete file", idle).clicked() {
                whole = Some(WholeFile::Delete);
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                save = widgets::primary_button(ui, "Save and mark resolved", idle && !markers)
                    .on_disabled_hover_text(if markers { "Remove the conflict markers first." } else { "Wait for the current action." })
                    .clicked();
                close = ui.button("Close").clicked();
            });
        });

        if let Some(choice) = whole {
            self.pending = Some(Pending::Whole(choice));
        }
        if close {
            if dirty {
                self.pending = Some(Pending::Close);
            } else {
                self.close_requested = true;
            }
        }
        if save {
            let document = document.clone();
            let text = self.result.clone();
            let resolved = Arc::clone(&self.resolved);
            let path = self.path.clone();
            cx.act(format!("Resolve {}", self.path), move |client, repo| {
                client.resolve_conflict(&document, &text, repo).map(|()| {
                    resolved.store(true, Ordering::SeqCst);
                    Some(format!("Resolved {path}"))
                })
            });
        }
    }

    fn wants_close(&self) -> bool {
        self.close_requested || self.resolved.load(Ordering::SeqCst)
    }
}

/// One read-only version, with a button that copies it into the result. Returns true when the
/// button was clicked.
fn version_pane(ui: &mut Ui, label: &str, text: Option<&str>, height: f32, idle: bool) -> bool {
    ui.label(RichText::new(label).strong());
    ScrollArea::both().id_salt(("conflict-version", label)).max_height(height).auto_shrink([false, false]).show(ui, |ui| match text {
        Some(text) => {
            ui.add(egui::Label::new(RichText::new(text).monospace()).extend());
        }
        None => {
            ui.label(RichText::new("Not available for this file.").weak());
        }
    });
    ui.add_enabled(idle && text.is_some(), egui::Button::new("Copy into result")).clicked()
}

fn version_text(document: &ConflictDocument, version: Version) -> Option<String> {
    match version {
        Version::Base => document.base.clone(),
        Version::Current => document.current.clone(),
        Version::Incoming => document.incoming.clone(),
    }
}

/// Starts a whole-file resolution. The resolution is marked done only if Git succeeds.
fn whole_file(path: String, resolved: Arc<AtomicBool>, choice: WholeFile, cx: &mut Ctx) {
    let label = match choice {
        WholeFile::Current => format!("Take current version of {path}"),
        WholeFile::Incoming => format!("Take incoming version of {path}"),
        WholeFile::Delete => format!("Delete {path}"),
    };
    cx.act(label, move |client: &GitClient, repo| {
        let outcome: nicegit_core::Result<()> = match choice {
            WholeFile::Current => client.resolve_conflict_side(&path, false, repo),
            WholeFile::Incoming => client.resolve_conflict_side(&path, true, repo),
            WholeFile::Delete => client.resolve_conflict_deletion(&path, repo),
        };
        outcome.map(|()| {
            resolved.store(true, Ordering::SeqCst);
            None
        })
    });
}
