//! Snapshots: the saved copies of the working files, newest first, with what differs between a
//! chosen snapshot and the files now. Files can be restored from the snapshot, and a file's change
//! is shown as a diff. Nothing is restored without a confirmation.

use std::collections::BTreeSet;
use std::time::{Duration, Instant};

use egui::{Color32, Margin, RichText, Sense, Ui};
use egui_phosphor::regular as icon;
use nicegit_core::diff::parse_diff;
use nicegit_core::snapshots::{
    delete_snapshot, differences_from_now, list_snapshots, restore_from_snapshot, snapshot_changes, snapshot_file_patch, working_tree_now,
    SnapshotChange, SnapshotInfo, DEFAULT_MAX_FILE_BYTES,
};
use nicegit_core::StatusKind;

use crate::diff_view::{self, DiffContent};
use crate::i18n::{tr, tr_count, tr_fmt};
use crate::theme;
use crate::tools::reflog::age_text;
use crate::tools::{query, widgets, Ctx, Task, ToolWindow};
use crate::ui::changes::kind_icon;

/// How long the list is kept before a repository change reads it again.
const RELOAD_EVERY: Duration = Duration::from_secs(30);

/// One snapshot as listed, with how many files differ between it and the files now.
#[derive(Clone, Debug)]
struct Listed {
    info: SnapshotInfo,
    differences: usize,
}

/// A restore or delete waiting for the person's confirmation.
#[derive(Clone, Debug)]
enum Confirm {
    Restore(Vec<String>),
    Delete,
}

#[derive(Default)]
pub struct SnapshotsWindow {
    /// The snapshots, read in the background.
    listing: Option<Task<nicegit_core::Result<Vec<Listed>>>>,
    needs_load: bool,
    snapshots: Vec<Listed>,
    error: Option<String>,
    /// The selected snapshot's full ref name.
    selected: Option<String>,
    /// The files that differ for the selected snapshot, read in the background.
    changes: Option<Task<nicegit_core::Result<Vec<SnapshotChange>>>>,
    needs_changes: bool,
    listed_changes: Vec<SnapshotChange>,
    /// When the snapshots were last read.
    loaded_at: Option<Instant>,
    /// Set when this window starts an action, so the list is read again once it has run.
    awaiting_action: bool,
    /// The files ticked for restoring. Only files that the snapshot holds can be ticked.
    ticked: BTreeSet<String>,
    /// The file whose diff is shown, and its patch being read.
    viewing: Option<String>,
    patch: Option<Task<nicegit_core::Result<String>>>,
    diff: Option<(String, DiffContent)>,
    confirm: Option<Confirm>,
}

impl SnapshotsWindow {
    pub fn new() -> Self {
        Self { needs_load: true, needs_changes: true, ..Self::default() }
    }

    fn selected_listing(&self) -> Option<&Listed> {
        let reference = self.selected.as_deref()?;
        self.snapshots.iter().find(|listed| listed.info.reference == reference)
    }

    /// Whether a file's version in the snapshot can be restored: a changed or deleted file is in it.
    fn restorable(change: &SnapshotChange) -> bool {
        change.kind != StatusKind::Added
    }

    fn select(&mut self, reference: Option<String>) {
        self.selected = reference;
        self.needs_changes = true;
        self.listed_changes.clear();
        self.ticked.clear();
        self.viewing = None;
        self.patch = None;
        self.diff = None;
    }
}

impl ToolWindow for SnapshotsWindow {
    fn id(&self) -> String {
        "snapshots".to_string()
    }

    fn title(&self) -> String {
        tr("Snapshots").to_string()
    }

    fn default_size(&self) -> egui::Vec2 {
        egui::vec2(860.0, 560.0)
    }

    /// Reads the snapshots again after an action of this window, and otherwise at most every
    /// `RELOAD_EVERY`: each read scans the working files, which the app's own refreshes do often.
    fn repository_changed(&mut self, _snapshot: &nicegit_core::Snapshot) {
        let due = self.awaiting_action || self.loaded_at.is_none_or(|at| at.elapsed() >= RELOAD_EVERY);
        if due {
            self.awaiting_action = false;
            self.needs_load = true;
        }
    }

    fn ui(&mut self, ui: &mut Ui, cx: &mut Ctx) {
        if self.needs_load && self.listing.is_none() {
            self.needs_load = false;
            self.listing = Some(query(ui.ctx(), cx.repo, |client, directory| {
                let snapshots = list_snapshots(client, directory)?;
                let differences = differences_from_now(client, directory, &snapshots)?;
                Ok(snapshots.into_iter().zip(differences).map(|(info, differences)| Listed { info, differences }).collect())
            }));
        }
        if let Some(result) = self.listing.as_mut().and_then(|task| task.get().cloned()) {
            self.listing = None;
            self.loaded_at = Some(Instant::now());
            match result {
                Ok(snapshots) => {
                    self.snapshots = snapshots;
                    self.error = None;
                    let still_listed = self.selected_listing().is_some();
                    if !still_listed {
                        let first = self.snapshots.first().map(|listed| listed.info.reference.clone());
                        self.select(first);
                    } else {
                        self.needs_changes = true;
                    }
                }
                Err(error) => self.error = Some(error.to_string()),
            }
        }
        if self.needs_changes && self.changes.is_none() {
            self.needs_changes = false;
            if let Some(info) = self.selected_listing().map(|listed| listed.info.clone()) {
                self.changes = Some(query(ui.ctx(), cx.repo, move |client, directory| {
                    let current = working_tree_now(client, directory, DEFAULT_MAX_FILE_BYTES)?;
                    snapshot_changes(client, directory, &info, &current)
                }));
            }
        }
        if let Some(result) = self.changes.as_mut().and_then(|task| task.get().cloned()) {
            self.changes = None;
            match result {
                Ok(changes) => {
                    self.ticked.retain(|path| changes.iter().any(|change| change.path == *path && Self::restorable(change)));
                    self.listed_changes = changes;
                }
                Err(error) => self.error = Some(error.to_string()),
            }
        }
        if let Some(result) = self.patch.as_mut().and_then(|task| task.get().cloned()) {
            self.patch = None;
            if let (Some(path), Ok(patch)) = (self.viewing.clone(), &result) {
                self.diff = Some((path.clone(), DiffContent::new(path, parse_diff(patch))));
            }
            if let Err(error) = result {
                self.error = Some(error.to_string());
            }
        }

        let c = theme::of(ui);
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(tr(
                    "NiceGit saves copies of your working files while they change. Pick a snapshot to see what differs from now.",
                ))
                .small()
                .color(c.muted),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if widgets::icon_button(ui, icon::ARROW_CLOCKWISE, tr("Read the snapshots again"), self.listing.is_none()).clicked() {
                    self.needs_load = true;
                }
            });
        });
        ui.add_space(6.0);
        if self.listing.is_some() {
            widgets::loading(ui, tr("Reading the snapshots…"));
        } else if let Some(error) = &self.error {
            widgets::error(ui, error);
        }

        let idle = cx.idle;
        let editor_unsaved = cx.editor_unsaved;
        ui.columns(2, |columns| {
            self.list(&mut columns[0]);
            self.detail(&mut columns[1], cx, idle, editor_unsaved);
        });
        self.confirmation(ui, cx, idle);
    }
}

impl SnapshotsWindow {
    fn list(&mut self, ui: &mut Ui) {
        let c = theme::of(ui);
        if self.snapshots.is_empty() && self.listing.is_none() && self.error.is_none() {
            widgets::empty_state(ui, icon::FLOPPY_DISK, tr("No snapshots yet. NiceGit saves them as your files change."));
            return;
        }
        let mut chosen = None;
        egui::ScrollArea::vertical().id_salt("snapshot_list").auto_shrink([false, false]).show(ui, |ui| {
            for listed in &self.snapshots {
                let is_selected = self.selected.as_deref() == Some(listed.info.reference.as_str());
                let fill = if is_selected { ui.visuals().selection.bg_fill } else { Color32::TRANSPARENT };
                let response = egui::Frame::new()
                    .fill(fill)
                    .corner_radius(6.0)
                    .inner_margin(Margin::symmetric(8, 6))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.label(RichText::new(age_text((listed.info.time / 1000) as i64)).strong());
                        let differences = tr_count("{count} file differs from now", "{count} files differ from now", listed.differences);
                        ui.label(RichText::new(differences).small().color(c.muted));
                    })
                    .response
                    .interact(Sense::click());
                if response.clicked() {
                    chosen = Some(listed.info.reference.clone());
                }
                ui.add_space(2.0);
            }
        });
        if let Some(reference) = chosen {
            if self.selected.as_deref() != Some(reference.as_str()) {
                self.select(Some(reference));
            }
        }
    }

    fn detail(&mut self, ui: &mut Ui, cx: &mut Ctx, idle: bool, editor_unsaved: bool) {
        let c = theme::of(ui);
        let Some(listed) = self.selected_listing().cloned() else {
            widgets::empty_state(ui, icon::ARROW_U_UP_LEFT, tr("Select a snapshot to see its files."));
            return;
        };
        ui.label(RichText::new(age_text((listed.info.time / 1000) as i64)).strong());
        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            let ticked = self.ticked.len();
            let can_restore = idle && !editor_unsaved && ticked > 0;
            let restore_label = if ticked > 0 {
                tr_fmt!("Restore {count} selected…", count = ticked)
            } else {
                tr("Restore selected files…").to_string()
            };
            let hover = if editor_unsaved {
                tr("Save or close the file you are editing before restoring files.")
            } else {
                tr("Ticked files are written back to their saved versions. Your current versions are kept in a snapshot first.")
            };
            if widgets::primary_button(ui, &format!("{}  {restore_label}", icon::ARROW_COUNTER_CLOCKWISE), can_restore)
                .on_hover_text(hover)
                .clicked()
            {
                self.confirm = Some(Confirm::Restore(self.ticked.iter().cloned().collect()));
            }
            if widgets::labeled_button(ui, icon::TRASH, tr("Delete snapshot"), idle).clicked() {
                self.confirm = Some(Confirm::Delete);
            }
        });
        ui.add_space(6.0);

        // A reread keeps the files on screen, so the ticks and the shown diff survive a refresh.
        if self.changes.is_some() && self.listed_changes.is_empty() {
            widgets::loading(ui, tr("Comparing the files…"));
            return;
        }
        if self.listed_changes.is_empty() {
            ui.label(RichText::new(tr("Nothing differs from now.")).color(c.muted));
            return;
        }
        if self.changes.is_some() {
            widgets::loading(ui, tr("Comparing the files…"));
        }
        let changes = self.listed_changes.clone();
        egui::ScrollArea::vertical().id_salt("snapshot_changes").max_height(170.0).auto_shrink([false, true]).show(ui, |ui| {
            for change in &changes {
                ui.horizontal(|ui| {
                    let restorable = Self::restorable(change);
                    let mut ticked = self.ticked.contains(&change.path);
                    let path = change.path.clone();
                    let checkbox = ui.add_enabled(restorable && idle && !editor_unsaved, egui::Checkbox::without_text(&mut ticked));
                    checkbox.widget_info(move || egui::WidgetInfo::labeled(egui::Role::CheckBox, restorable, path.clone()));
                    let checkbox = if restorable {
                        checkbox
                    } else {
                        checkbox.on_disabled_hover_text(tr("This file is not in the snapshot, so there is nothing to restore."))
                    };
                    if checkbox.changed() {
                        if ticked {
                            self.ticked.insert(change.path.clone());
                        } else {
                            self.ticked.remove(&change.path);
                        }
                    }
                    let (glyph, colour) = kind_icon(ui, change.kind);
                    ui.label(RichText::new(glyph).color(colour));
                    let is_viewing = self.viewing.as_deref() == Some(change.path.as_str());
                    let label = RichText::new(&change.path).monospace();
                    if ui.selectable_label(is_viewing, label).clicked() && !is_viewing {
                        self.viewing = Some(change.path.clone());
                        self.diff = None;
                        self.patch = Some(query(ui.ctx(), cx.repo, {
                            let snapshot = listed.info.clone();
                            let path = change.path.clone();
                            let request = cx.diff_request();
                            move |client, directory| snapshot_file_patch(client, directory, &snapshot, &path, request)
                        }));
                    }
                    ui.label(RichText::new(crate::ui::changes::kind_title(change.kind)).small().color(c.muted));
                });
            }
        });
        ui.separator();
        match &self.diff {
            Some((path, content)) if self.viewing.as_deref() == Some(path.as_str()) => diff_view::show(ui, content, false),
            _ if self.patch.is_some() => widgets::loading(ui, tr("Reading the difference…")),
            _ => {
                ui.label(RichText::new(tr("Select a file to see what changed since the snapshot.")).color(c.muted));
            }
        }
    }

    /// The confirmation for a restore or a delete, which names what will change.
    fn confirmation(&mut self, ui: &mut Ui, cx: &mut Ctx, idle: bool) {
        let Some(confirm) = self.confirm.clone() else { return };
        let mut open = true;
        let mut close = false;
        let title = match confirm {
            Confirm::Restore(_) => tr("Restore selected files"),
            Confirm::Delete => tr("Delete snapshot"),
        };
        egui::Window::new(title)
            .id(ui.scope_id().with("snapshot_confirm"))
            .collapsible(false)
            .resizable(false)
            .default_width(420.0)
            .open(&mut open)
            .show(ui.ctx(), |ui| match &confirm {
                Confirm::Restore(paths) => {
                    ui.label(tr_count(
                        "This file in your working tree is replaced with its saved version.",
                        "These {count} files in your working tree are replaced with their saved versions.",
                        paths.len(),
                    ));
                    ui.label(RichText::new(tr("Your current versions are saved as a snapshot first, so this can be undone.")).small());
                    ui.add_space(6.0);
                    egui::ScrollArea::vertical().id_salt("snapshot_restore_paths").max_height(160.0).show(ui, |ui| {
                        for path in paths {
                            ui.label(RichText::new(path).monospace());
                        }
                    });
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if ui.button(tr("Cancel")).clicked() {
                            close = true;
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if widgets::primary_button(ui, tr("Restore"), idle).clicked() {
                                let Some(listed) = self.selected_listing().cloned() else { return };
                                let paths = paths.clone();
                                let count = paths.len();
                                self.awaiting_action = true;
                                cx.act(tr("Restore snapshot files"), move |client, directory| {
                                    restore_from_snapshot(client, &listed.info, &paths, directory)
                                        .map(|()| Some(tr_count("Restored {count} file.", "Restored {count} files.", count)))
                                });
                                close = true;
                            }
                        });
                    });
                }
                Confirm::Delete => {
                    ui.label(tr("The snapshot is removed from this repository. Its files cannot be restored from it afterwards."));
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if ui.button(tr("Cancel")).clicked() {
                            close = true;
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if widgets::danger_button(ui, tr("Delete snapshot"), idle).clicked() {
                                if let Some(listed) = self.selected_listing().cloned() {
                                    self.awaiting_action = true;
                                    cx.act(tr("Delete snapshot"), move |client, directory| {
                                        delete_snapshot(client, directory, &listed.info)
                                            .map(|()| Some(tr("Deleted the snapshot.").to_string()))
                                    });
                                }
                                close = true;
                            }
                        });
                    });
                }
            });
        if !open || close {
            self.confirm = None;
        }
    }
}
