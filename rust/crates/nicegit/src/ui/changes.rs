use std::collections::BTreeMap;

use egui::{Color32, RichText, Ui};
use egui_phosphor::regular as icon;
use nicegit_core::{Snapshot, StatusEntry, StatusKind};

use crate::app::{NiceGitApp, Selection};
use crate::settings::FileView;
use crate::theme;
use crate::tools::{self, widgets};
use crate::ui::dialogs::Pending;

/// The icon and colour for a changed file.
pub fn kind_icon(ui: &Ui, kind: StatusKind) -> (&'static str, Color32) {
    let c = theme::of(ui);
    match kind {
        StatusKind::Added => (icon::PLUS, c.added),
        StatusKind::Untracked => (icon::PLUS, c.added),
        StatusKind::Deleted => (icon::MINUS, c.removed),
        StatusKind::Renamed => (icon::ARROW_RIGHT, c.renamed),
        StatusKind::Conflicted => (icon::WARNING, c.conflict),
        StatusKind::Modified => (icon::PENCIL_SIMPLE, c.warning),
    }
}

/// An outlined button in a semantic colour, as used for "Stage All Changes".
fn outlined(ui: &mut Ui, text: &str, color: Color32, enabled: bool) -> egui::Response {
    ui.add_enabled(
        enabled,
        egui::Button::new(RichText::new(text).small().strong().color(color))
            .fill(Color32::TRANSPARENT)
            .stroke(egui::Stroke::new(1.0, color)),
    )
}

/// A directory in the tree view: files directly inside it and its subdirectories.
#[derive(Default)]
struct Folder {
    files: Vec<StatusEntry>,
    folders: BTreeMap<String, Folder>,
}

impl Folder {
    /// Groups entries by path. A deleted file and an untracked folder can share a prefix,
    /// so files and folders are kept apart rather than merged by name.
    fn build(entries: &[StatusEntry]) -> Folder {
        let mut root = Folder::default();
        for entry in entries {
            let trimmed = entry.path.trim_end_matches('/');
            let mut parts: Vec<&str> = trimmed.split('/').collect();
            parts.pop();
            let mut folder = &mut root;
            for part in parts {
                folder = folder.folders.entry(part.to_string()).or_default();
            }
            folder.files.push(entry.clone());
        }
        root
    }

    fn count(&self) -> usize {
        self.files.len() + self.folders.values().map(Folder::count).sum::<usize>()
    }
}

impl NiceGitApp {
    pub fn changes_panel(&mut self, ui: &mut Ui) {
        let Some(snapshot) = self.snapshot().cloned() else { return };
        let c = theme::of(ui);
        let idle = self.idle();
        let unstaged: Vec<StatusEntry> = sorted(snapshot.status.iter().filter(|e| e.is_unstaged()).cloned().collect());
        let staged: Vec<StatusEntry> = sorted(snapshot.status.iter().filter(|e| e.is_staged()).cloned().collect());

        egui::Frame::new().inner_margin(egui::Margin { left: 16, right: 14, top: 14, bottom: 6 }).show(ui, |ui| {
            ui.horizontal(|ui| {
                let count = snapshot.status.len();
                ui.label(RichText::new(format!("{count} file change{} on", if count == 1 { "" } else { "s" })).strong());
                widgets::pill(ui, &snapshot.current_branch, c.local_branch);
            });
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new("File view").color(c.muted));
                ui.selectable_value(&mut self.settings.file_view, FileView::Path, "Path");
                ui.selectable_value(&mut self.settings.file_view, FileView::Tree, "Tree");
            });
        });
        ui.separator();

        egui::Panel::bottom("commit_box")
            .resizable(false)
            .frame(egui::Frame::new().inner_margin(egui::Margin { left: 16, right: 14, top: 10, bottom: 14 }))
            .show(ui, |ui| self.commit_box(ui, &snapshot, staged.len()));

        if let Some(undo) = self.repo().and_then(|r| r.discard_undo.last().cloned()) {
            let more = self.repo().map(|r| r.discard_undo.len().saturating_sub(1)).unwrap_or(0);
            egui::Frame::new().fill(c.subtle_bg).inner_margin(egui::Margin::symmetric(16, 8)).show(ui, |ui| {
                ui.horizontal(|ui| {
                    let more = if more > 0 { format!(" (and {more} earlier)") } else { String::new() };
                    ui.add(
                        egui::Label::new(
                            RichText::new(format!("{}  Discarded changes to {}{more}", icon::TRASH, undo.path)).color(c.muted),
                        )
                        .truncate(),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if widgets::icon_button(ui, icon::X, "Dismiss", true).clicked() {
                            self.repos[self.active].discard_undo.clear();
                        }
                        if ui.add_enabled(idle, egui::Button::new(format!("{}  Undo discard", icon::ARROW_COUNTER_CLOCKWISE))).clicked() {
                            self.undo_discard();
                        }
                    });
                });
            });
        }
        let available = ui.available_height();
        let tree = self.settings.file_view == FileView::Tree;
        egui::ScrollArea::vertical().id_salt("changes").auto_shrink(false).max_height(available).show(ui, |ui| {
            egui::Frame::new().inner_margin(egui::Margin { left: 10, right: 10, top: 6, bottom: 6 }).show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(format!("{}  Unstaged files ({})", icon::CARET_DOWN, unstaged.len())).strong());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if outlined(ui, "Stage All Changes", c.accent, idle && !unstaged.is_empty()).clicked() {
                            self.act("Stage all", |client, path| client.stage_all(path).map(|_| None));
                        }
                        // Keeps the discard icon clear of the Stage All button.
                        ui.add_space(8.0);
                        let discardable: Vec<StatusEntry> = unstaged.iter().filter(|e| e.kind != StatusKind::Conflicted).cloned().collect();
                        let discard_all_enabled = idle && !discardable.is_empty();
                        let discard_all = widgets::icon_button(ui, icon::TRASH, "Discard all changes…", discard_all_enabled);
                        discard_all.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, discard_all_enabled, "Discard all changes…"));
                        if discard_all.clicked() {
                            self.confirm(
                                "Discard all changes?",
                                format!("Staged and unstaged changes to {} file(s) are lost, and untracked files are deleted. This cannot be undone.", discardable.len()),
                                "Discard all",
                                Pending::DiscardAll(discardable),
                            );
                        }
                    });
                });
                ui.add_space(4.0);
                if unstaged.is_empty() {
                    ui.label(RichText::new("   No unstaged changes").small().color(c.muted));
                } else if tree {
                    self.folder_rows(ui, &Folder::build(&unstaged), false, idle, &snapshot, 0);
                } else {
                    for entry in &unstaged {
                        self.change_row(ui, entry, false, idle, &snapshot, entry.path.clone());
                    }
                }
                ui.add_space(14.0);
                ui.separator();
                ui.horizontal(|ui| {
                    ui.label(RichText::new(format!("{}  Staged files ({})", icon::CARET_DOWN, staged.len())).strong());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if outlined(ui, "Unstage All Changes", c.accent, idle && !staged.is_empty()).clicked() {
                            self.act("Unstage all", |client, path| client.unstage_all(path).map(|_| None));
                        }
                    });
                });
                ui.add_space(4.0);
                if staged.is_empty() {
                    ui.label(RichText::new("   Stage files to include them in the next commit").small().color(c.muted));
                } else if tree {
                    self.folder_rows(ui, &Folder::build(&staged), true, idle, &snapshot, 0);
                } else {
                    for entry in &staged {
                        self.change_row(ui, entry, true, idle, &snapshot, entry.path.clone());
                    }
                }
            });
        });
    }

    fn folder_rows(&mut self, ui: &mut Ui, folder: &Folder, staged: bool, idle: bool, snapshot: &Snapshot, depth: usize) {
        let c = theme::of(ui);
        for (name, child) in &folder.folders {
            let id = ui.make_persistent_id(("folder", staged, depth, name));
            egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, true)
                .show_header(ui, |ui| {
                    ui.label(RichText::new(format!("{}  {name}", icon::FOLDER)).color(c.muted));
                    ui.label(RichText::new(child.count().to_string()).small().color(c.muted));
                })
                .body(|ui| self.folder_rows(ui, child, staged, idle, snapshot, depth + 1));
        }
        for entry in &folder.files {
            self.change_row(ui, entry, staged, idle, snapshot, entry.file_name().to_string());
        }
    }

    fn change_row(&mut self, ui: &mut Ui, entry: &StatusEntry, staged: bool, idle: bool, snapshot: &Snapshot, label: String) {
        let _c = theme::of(ui);
        let selected =
            matches!(self.repo().map(|r| &r.selection), Some(Selection::Change { entry: e, staged: s }) if e == entry && *s == staged);
        let height = 28.0;
        let (rect, response) = ui.allocate_exact_size(egui::vec2(ui.available_width(), height), egui::Sense::click());
        if selected {
            ui.painter().rect_filled(rect, 6.0, ui.visuals().selection.bg_fill);
        } else if response.hovered() {
            ui.painter().rect_filled(rect, 6.0, ui.visuals().widgets.hovered.weak_bg_fill);
        }
        let mut child = ui.new_child(
            egui::UiBuilder::new().max_rect(rect.shrink2(egui::vec2(8.0, 0.0))).layout(egui::Layout::left_to_right(egui::Align::Center)),
        );
        let (glyph, color) = kind_icon(&child, entry.kind);
        child.label(RichText::new(glyph).color(color)).on_hover_text(entry.kind.title());
        let text = match &entry.original_path {
            Some(original) if entry.kind == StatusKind::Renamed => {
                format!("{} {} {label}", original.rsplit('/').next().unwrap_or(original), egui_phosphor::regular::ARROW_RIGHT)
            }
            _ => label,
        };
        let mut stage_clicked = false;
        let mut discard_clicked = false;
        let mut resolve_clicked = false;
        let buttons = if staged { 1.0 } else { 2.0 };
        let label_width = (child.available_width() - buttons * 30.0).max(40.0);
        child.allocate_ui_with_layout(egui::vec2(label_width, height), egui::Layout::left_to_right(egui::Align::Center), |ui| {
            ui.add(egui::Label::new(RichText::new(text).color(ui.visuals().text_color())).truncate().selectable(false))
                .on_hover_text(&entry.path);
        });
        child.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if staged {
                stage_clicked = widgets::icon_button(ui, icon::MINUS_CIRCLE, "Unstage", idle).clicked();
            } else {
                discard_clicked =
                    widgets::icon_button(ui, icon::TRASH, "Discard changes…", idle && entry.kind != StatusKind::Conflicted).clicked();
                if entry.kind == StatusKind::Conflicted {
                    resolve_clicked = widgets::icon_button(ui, icon::GIT_DIFF, "Resolve conflict…", true).clicked();
                } else {
                    stage_clicked = widgets::icon_button(ui, icon::PLUS_CIRCLE, "Stage", idle).clicked();
                }
            }
        });
        if resolve_clicked {
            self.open_tool(Box::new(tools::conflict::ConflictWindow::new(entry.path.clone())));
        }
        if stage_clicked {
            let entry = entry.clone();
            if staged {
                self.act("Unstage", move |client, path| client.unstage(&entry, path).map(|_| None));
            } else {
                self.act("Stage", move |client, path| client.stage(&entry.path, path).map(|_| None));
            }
        } else if discard_clicked {
            self.confirm_discard(entry.clone());
        } else if response.clicked() {
            self.select_change(entry.clone(), staged);
        } else if response.double_clicked() && entry.kind == StatusKind::Conflicted {
            self.open_tool(Box::new(tools::conflict::ConflictWindow::new(entry.path.clone())));
        }
        response.context_menu(|ui| self.change_menu(ui, entry, staged, idle, snapshot));
    }

    fn confirm_discard(&mut self, entry: StatusEntry) {
        // Renames, conflicts, folders, and submodules are discarded without a saved copy.
        let undoable = entry.original_path.is_none()
            && !matches!(entry.kind, StatusKind::Conflicted | StatusKind::Renamed)
            && !entry.path.ends_with('/');
        let undo_note = if undoable {
            "You can undo this from the Changes panel until the file changes again."
        } else {
            "This cannot be undone: renames, conflicts, folders, and submodules are discarded without a saved copy."
        };
        let what = if entry.kind == StatusKind::Untracked {
            format!("{} is untracked and will be deleted. {undo_note}", entry.path)
        } else {
            format!("Staged and unstaged changes to {} are discarded. {undo_note}", entry.path)
        };
        self.confirm("Discard changes?", what, "Discard", Pending::Discard(entry));
    }

    fn change_menu(&mut self, ui: &mut Ui, entry: &StatusEntry, staged: bool, idle: bool, snapshot: &Snapshot) {
        ui.set_min_width(230.0);
        let repo_path = self.repo().map(|r| r.path.clone()).unwrap_or_default();
        if entry.kind == StatusKind::Conflicted && ui.button(format!("{}  Resolve conflict…", icon::GIT_DIFF)).clicked() {
            ui.close();
            self.open_tool(Box::new(tools::conflict::ConflictWindow::new(entry.path.clone())));
        }
        if staged {
            if ui.add_enabled(idle, egui::Button::new(format!("{}  Unstage", icon::MINUS_CIRCLE))).clicked() {
                ui.close();
                let entry = entry.clone();
                self.act("Unstage", move |client, path| client.unstage(&entry, path).map(|_| None));
            }
        } else if entry.kind != StatusKind::Conflicted {
            if ui.add_enabled(idle, egui::Button::new(format!("{}  Stage", icon::PLUS_CIRCLE))).clicked() {
                ui.close();
                let path_text = entry.path.clone();
                self.act("Stage", move |client, path| client.stage(&path_text, path).map(|_| None));
            }
            if ui.add_enabled(idle, egui::Button::new(format!("{}  Discard changes…", icon::TRASH))).clicked() {
                ui.close();
                self.confirm_discard(entry.clone());
            }
        }
        if !staged && entry.kind != StatusKind::Deleted && ui.button(format!("{}  Edit file", icon::PENCIL_SIMPLE)).clicked() {
            ui.close();
            self.open_tool(Box::new(tools::editor::EditorWindow::new(repo_path.clone(), entry.path.clone())));
        }
        if ui.add_enabled(idle && snapshot.operation.is_none(), egui::Button::new(format!("{}  Stash this file…", icon::ARCHIVE))).clicked()
        {
            ui.close();
            let path_text = entry.path.clone();
            self.act("Stash file", move |client, path| {
                client
                    .save_stash_paths(std::slice::from_ref(&path_text), &format!("Changes to {path_text}"), path)
                    .map(|_| Some(format!("Stashed {path_text}.")))
            });
        }
        if entry.kind == StatusKind::Untracked {
            ui.menu_button(format!("{}  Ignore", icon::EYE_SLASH), |ui| {
                use nicegit_core::settings::{IgnoreRule, IgnoreScope};
                let extension = std::path::Path::new(&entry.path).extension().map(|e| e.to_string_lossy().into_owned());
                let ignore = |ui: &mut Ui, app: &mut NiceGitApp, text: String, rule: IgnoreRule, scope: IgnoreScope| {
                    if ui.add_enabled(idle, egui::Button::new(text)).clicked() {
                        ui.close();
                        let file = entry.path.clone();
                        app.act("Ignore", move |client, path| {
                            client.ignore(&file, rule, scope, path).map(|_| Some("Updated the ignore rules.".into()))
                        });
                    }
                };
                ignore(ui, self, "This file in .gitignore".to_string(), IgnoreRule::Path, IgnoreScope::Shared);
                if let Some(extension) = &extension {
                    ignore(ui, self, format!("All .{extension} files in .gitignore"), IgnoreRule::FileExtension, IgnoreScope::Shared);
                }
                ignore(ui, self, "This file on this computer only".into(), IgnoreRule::Path, IgnoreScope::Local);
            });
        }
        ui.separator();
        if entry.kind != StatusKind::Untracked && ui.button(format!("{}  File history", icon::CLOCK_COUNTER_CLOCKWISE)).clicked() {
            ui.close();
            self.open_tool(Box::new(tools::file_history::FileHistoryWindow::new(entry.path.clone())));
        }
        if entry.kind != StatusKind::Untracked
            && entry.kind != StatusKind::Deleted
            && ui.button(format!("{}  Blame", icon::USER_LIST)).clicked()
        {
            ui.close();
            self.open_tool(Box::new(tools::blame::BlameWindow::new(entry.path.clone(), None)));
        }
        if ui.button(format!("{}  Copy path", icon::COPY)).clicked() {
            ui.close();
            ui.ctx().copy_text(entry.path.clone());
        }
        if ui.button(format!("{}  Show in file manager", icon::FOLDER_OPEN)).clicked() {
            ui.close();
            reveal(&repo_path.join(&entry.path));
        }
    }

    fn commit_box(&mut self, ui: &mut Ui, snapshot: &Snapshot, staged: usize) {
        let c = theme::of(ui);
        ui.horizontal(|ui| {
            ui.label(RichText::new(icon::GIT_COMMIT).size(18.0).color(c.accent));
            ui.label(RichText::new("Commit").size(15.0).strong());
        });
        ui.add_space(6.0);
        let can_amend = snapshot.is_on_branch() && snapshot.head_hash.is_some() && snapshot.operation.is_none();
        let head_published = self.head_published();
        let repo = &mut self.repos[self.active];
        egui::Frame::new()
            .fill(ui.visuals().extreme_bg_color)
            .stroke(egui::Stroke::new(1.0, c.border))
            .corner_radius(8.0)
            .inner_margin(egui::Margin::symmetric(10, 8))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut repo.draft.summary)
                            .hint_text("Commit summary")
                            .frame(egui::Frame::NONE)
                            .desired_width((ui.available_width() - 34.0).max(40.0)),
                    )
                    .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, "Commit summary"));
                    let length = repo.draft.summary.chars().count();
                    let color = if length > 72 { c.warning } else { c.muted };
                    ui.label(RichText::new((72_i64 - length as i64).to_string()).small().monospace().color(color))
                        .on_hover_text("Keep summaries to 72 characters or fewer");
                });
                ui.add(
                    egui::TextEdit::multiline(&mut repo.draft.description)
                        .hint_text("Description")
                        .frame(egui::Frame::NONE)
                        .desired_rows(4)
                        .desired_width(f32::INFINITY),
                )
                .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, "Commit description"));
            });
        ui.add_space(6.0);
        let mut amend = repo.amend;
        let amend_changed = crate::tools::widgets::checkbox(ui, can_amend, &mut amend, "Amend last commit").changed();
        if amend && head_published {
            widgets::callout(
                ui,
                "The last commit is already on a remote. Amending rewrites it, so others who pulled it will need to reconcile.",
                true,
            );
        }
        if amend_changed {
            self.repos[self.active].amend = amend;
            if amend && self.repos[self.active].draft.is_empty() {
                if let (Some(head), Some(repo)) = (snapshot.head_hash.clone(), self.repo()) {
                    if let Ok(message) = nicegit_core::GitClient::new().commit_message(&head, &repo.path) {
                        self.repos[self.active].draft = crate::settings::Draft::from_message(&message);
                    }
                }
            }
        }
        ui.add_space(6.0);
        let repo = &self.repos[self.active];
        let label = if repo.draft.summary.trim().is_empty() {
            format!("{}  Type a message to commit", icon::CHECK_CIRCLE)
        } else if repo.amend {
            format!("{}  Amend last commit", icon::CHECK_CIRCLE)
        } else if staged == 0 {
            format!("{}  Stage files to commit", icon::CHECK_CIRCLE)
        } else {
            format!("{}  Commit {staged} file{} to {}", icon::CHECK_CIRCLE, if staged == 1 { "" } else { "s" }, snapshot.current_branch)
        };
        let enabled = self.can_commit();
        let button = egui::Button::new(RichText::new(label).strong().color(if enabled { c.accent_text } else { c.muted }))
            .fill(if enabled { c.accent } else { ui.visuals().widgets.inactive.weak_bg_fill })
            .min_size(egui::vec2(ui.available_width(), 36.0));
        if ui.add_enabled(enabled, button).on_hover_text("Ctrl/Cmd-Enter").clicked() {
            self.commit();
        }
    }

    /// Whether HEAD is already on a remote-tracking branch, judged from the loaded history.
    pub(crate) fn head_published(&self) -> bool {
        let Some(snapshot) = self.snapshot() else { return false };
        let Some(head) = &snapshot.head_hash else { return false };
        snapshot
            .branches
            .iter()
            .filter(|b| b.is_remote)
            .any(|remote| &remote.tip == head || Self::branch_contains(snapshot, &remote.tip, head))
    }

    fn branch_contains(snapshot: &Snapshot, tip: &str, target: &str) -> bool {
        let fake = nicegit_core::Branch {
            name: String::new(),
            is_current: false,
            is_remote: true,
            tip: target.to_string(),
            subject: String::new(),
            upstream: None,
        };
        let mut shifted = snapshot.clone();
        shifted.head_hash = Some(tip.to_string());
        Self::branch_is_merged(&fake, &shifted)
    }
}

fn sorted(mut entries: Vec<StatusEntry>) -> Vec<StatusEntry> {
    entries.sort_by_key(|a| a.path.to_lowercase());
    entries
}

/// Shows a file in the platform's file manager.
pub fn reveal(path: &std::path::Path) {
    let target = if path.exists() { path.to_path_buf() } else { path.parent().map(|p| p.to_path_buf()).unwrap_or_default() };
    #[cfg(target_os = "macos")]
    let _ = std::process::Command::new("open").arg("-R").arg(&target).spawn();
    #[cfg(target_os = "windows")]
    let _ = std::process::Command::new("explorer").arg(format!("/select,{}", target.display())).spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let _ = std::process::Command::new("xdg-open").arg(target.parent().unwrap_or(&target)).spawn();
}
