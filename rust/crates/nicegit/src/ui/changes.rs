use std::collections::BTreeMap;

use egui::{Color32, RichText, Ui};
use egui_phosphor::regular as icon;
use nicegit_core::line_counts::LineCount;
use nicegit_core::{Snapshot, StatusEntry, StatusKind};

use crate::app::{NiceGitApp, Repo, Selection};
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
        let all_unstaged: Vec<StatusEntry> = sorted(snapshot.status.iter().filter(|e| e.is_unstaged()).cloned().collect());
        let all_staged: Vec<StatusEntry> = sorted(snapshot.status.iter().filter(|e| e.is_staged()).cloned().collect());
        // A long list of changes can be narrowed by path. The field shows once there are many, and
        // stays while the filter holds text or the field has focus, so it never hides itself.
        let filter = self.repo().map(|r| r.change_filter.clone()).unwrap_or_default();
        let filtering = !filter.trim().is_empty();
        let focus_requested = filter_focus_requested(ui.ctx());
        let show_filter = snapshot.status.len() > FILTER_FROM || filtering || filter_had_focus(ui.ctx()) || focus_requested;
        let matches =
            |entries: &[StatusEntry]| -> Vec<StatusEntry> { entries.iter().filter(|e| matches_filter(e, &filter)).cloned().collect() };
        let unstaged = matches(&all_unstaged);
        let staged = matches(&all_staged);
        let shown = |part: &[StatusEntry], whole: &[StatusEntry]| {
            if filtering {
                format!("{} of {}", part.len(), whole.len())
            } else {
                whole.len().to_string()
            }
        };

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
            let mut filter_focused = false;
            if show_filter {
                ui.add_space(6.0);
                let field = widgets::search_field(ui, &mut self.repos[self.active].change_filter, "Filter changed files");
                field.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, "Filter changed files"));
                if focus_requested {
                    field.request_focus();
                }
                filter_focused = field.has_focus() || focus_requested;
            }
            // Stamped with the pass it was drawn in, so a panel no longer drawn leaves no stale focus.
            let pass = ui.ctx().cumulative_pass_nr();
            ui.ctx().data_mut(|data| data.insert_temp(filter_focus_key(), (pass, filter_focused)));
        });
        ui.separator();

        egui::Panel::bottom("commit_box")
            .resizable(false)
            .frame(egui::Frame::new().inner_margin(egui::Margin { left: 16, right: 14, top: 10, bottom: 14 }))
            .show(ui, |ui| self.commit_box(ui, &snapshot, all_staged.len()));

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
                // A narrow panel drops "files" from the titles so their counts stay visible; if even
                // that does not fit, the title is truncated rather than running under the buttons.
                // The accessible label keeps the full wording.
                let compact = ui.available_width() < 380.0;
                let noun = if compact { "" } else { " files" };
                egui::containers::Sides::new().shrink_left().truncate().show(
                    ui,
                    |ui| {
                        let count = shown(&unstaged, &all_unstaged);
                        let full = format!("Unstaged files ({count})");
                        ui.label(RichText::new(format!("{}  Unstaged{noun} ({count})", icon::CARET_DOWN)).strong())
                            .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, &full));
                    },
                    |ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        // The bulk buttons act on every file in their section, not only the shown ones.
                        let enabled = idle && !all_unstaged.is_empty();
                        let stage_all = outlined(ui, if compact { "Stage All" } else { "Stage All Changes" }, c.accent, enabled);
                        stage_all.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, "Stage All Changes"));
                        let stage_all = if filtering {
                            stage_all.on_hover_text(format!(
                                "Stages all {} unstaged files, including those hidden by the filter",
                                all_unstaged.len()
                            ))
                        } else {
                            stage_all
                        };
                        if stage_all.clicked() {
                            self.act("Stage all", |client, path| client.stage_all(path).map(|_| None));
                        }
                        // Keeps the discard icon clear of the Stage All button.
                        ui.add_space(8.0);
                        let discardable: Vec<StatusEntry> = all_unstaged.iter().filter(|e| e.kind != StatusKind::Conflicted).cloned().collect();
                        let discard_all_enabled = idle && !discardable.is_empty();
                        let discard_tip = if filtering {
                            format!("Discards changes to all {} unstaged files, including those hidden by the filter", discardable.len())
                        } else {
                            "Discard all changes…".to_string()
                        };
                        let discard_all = widgets::icon_button(ui, icon::TRASH, &discard_tip, discard_all_enabled);
                        discard_all.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, discard_all_enabled, "Discard all changes…"));
                        if discard_all.clicked() {
                            let hidden = if filtering { " Files hidden by the filter are included." } else { "" };
                            self.confirm(
                                "Discard all changes?",
                                format!(
                                    "Staged and unstaged changes to {} file(s) are lost, and untracked files are deleted. This cannot be undone.{hidden}",
                                    discardable.len()
                                ),
                                "Discard all",
                                Pending::DiscardAll(discardable),
                            );
                        }
                        // Saves every unstaged change, untracked files included, for the person to keep or share.
                        let save_tip = if filtering {
                            "Save unstaged changes as a patch, including untracked files and those hidden by the filter"
                        } else {
                            "Save unstaged changes as a patch, including untracked files"
                        };
                        if widgets::icon_button(ui, icon::FLOPPY_DISK, save_tip, idle && !all_unstaged.is_empty()).clicked() {
                            let name = self.section_patch_name(false);
                            self.save_working_patch(false, None, true, &name);
                        }
                    });
                    },
                );
                ui.add_space(4.0);
                if unstaged.is_empty() && !all_unstaged.is_empty() {
                    ui.label(RichText::new("   No matching files").small().color(c.muted));
                } else if unstaged.is_empty() {
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
                egui::containers::Sides::new().shrink_left().truncate().show(
                    ui,
                    |ui| {
                        let count = shown(&staged, &all_staged);
                        let full = format!("Staged files ({count})");
                        ui.label(RichText::new(format!("{}  Staged{noun} ({count})", icon::CARET_DOWN)).strong())
                            .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, &full));
                    },
                    |ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let enabled = idle && !all_staged.is_empty();
                        let unstage_all = outlined(ui, if compact { "Unstage All" } else { "Unstage All Changes" }, c.accent, enabled);
                        unstage_all.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, "Unstage All Changes"));
                        let unstage_all = if filtering {
                            unstage_all.on_hover_text(format!(
                                "Unstages all {} staged files, including those hidden by the filter",
                                all_staged.len()
                            ))
                        } else {
                            unstage_all
                        };
                        if unstage_all.clicked() {
                            self.act("Unstage all", |client, path| client.unstage_all(path).map(|_| None));
                        }
                        let save_tip = if filtering {
                            "Save staged changes as a patch, including files hidden by the filter"
                        } else {
                            "Save staged changes as a patch"
                        };
                        if widgets::icon_button(ui, icon::FLOPPY_DISK, save_tip, idle && !all_staged.is_empty()).clicked() {
                            let name = self.section_patch_name(true);
                            self.save_working_patch(true, None, false, &name);
                        }
                    });
                    },
                );
                ui.add_space(4.0);
                if staged.is_empty() && !all_staged.is_empty() {
                    ui.label(RichText::new("   No matching files").small().color(c.muted));
                } else if staged.is_empty() {
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
        let filtering = self.repo().is_some_and(|r| !r.change_filter.trim().is_empty());
        for (name, child) in &folder.folders {
            // While filtering, folders open so their matches show. That uses its own open state, so
            // the folder's saved state is left as the user set it.
            let id = ui.make_persistent_id(("folder", staged, depth, name, filtering));
            let mut state = egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, true);
            if filtering {
                state.set_open(true);
            }
            state
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
        let counts = self.repos[self.active].line_counts.as_mut().and_then(|task| task.get()).and_then(|counts| counts.as_ref());
        let count =
            counts.and_then(|counts| if staged { counts.staged.get(&entry.path) } else { counts.unstaged.get(&entry.path) }).copied();
        let large = counts.and_then(|counts| counts.large.get(&entry.path)).copied();
        let buttons = if staged { 1.0 } else { 2.0 };
        let counts_width = if count.is_some() { 64.0 } else { 0.0 } + if large.is_some() { 64.0 } else { 0.0 };
        let label_width = (child.available_width() - buttons * 30.0 - counts_width).max(40.0);
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
            if let Some(count) = count {
                line_count(ui, count);
            }
            // A file this large is easy to commit by mistake and hard to remove from history.
            if let Some(bytes) = large {
                let c = theme::of(ui);
                ui.label(RichText::new(format!("{} {}", icon::WARNING, megabytes(bytes))).small().color(c.warning)).on_hover_text(
                    "A large file. GitHub warns about files over 50 MB and refuses files over 100 MB; consider Git LFS or ignoring it.",
                );
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

    pub(crate) fn confirm_discard(&mut self, entry: StatusEntry) {
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

    /// The default file name for a section's patch, such as `repo-staged.patch`.
    fn section_patch_name(&self, staged: bool) -> String {
        let folder = self.repo().map(|repo| tools::archive::folder_stem(&repo.path)).unwrap_or_else(|| "repository".into());
        format!("{folder}-{}.patch", if staged { "staged" } else { "unstaged" })
    }

    /// Asks where to save the staged or unstaged changes, or only those to `paths`, and writes the
    /// patch there. The patch is made when the person has chosen a file.
    fn save_working_patch(&mut self, staged: bool, paths: Option<Vec<String>>, include_untracked: bool, default_name: &str) {
        let Some(repo) = self.repo() else { return };
        let directory = repo.path.clone();
        let Some(file) = crate::file_dialog::save_file("Save as patch", default_name) else { return };
        match nicegit_core::GitClient::new().working_changes_patch(staged, paths.as_deref(), include_untracked, &directory) {
            Ok(patch) => match std::fs::write(&file, patch) {
                Ok(()) => {
                    let name = file.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
                    self.notify(format!("Saved {name}."), false);
                }
                Err(error) => self.notify(error.to_string(), true),
            },
            Err(error) => self.notify(error.to_string(), true),
        }
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
        // Only this file, on the side the row shows. A conflicted file has no patch that applies.
        if ui
            .add_enabled(idle && entry.kind != StatusKind::Conflicted, egui::Button::new(format!("{}  Save as patch…", icon::FLOPPY_DISK)))
            .clicked()
        {
            ui.close();
            // A renamed file's patch needs its source path too, or it would only add the new name.
            let mut paths = vec![entry.path.clone()];
            if staged && entry.kind == StatusKind::Renamed {
                paths.extend(entry.original_path.clone());
            }
            let name = format!("{}.patch", entry.file_name());
            self.save_working_patch(staged, Some(paths), true, &name);
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
        let file = repo_path.join(&entry.path);
        let editor = self.settings.external_editor.trim().to_string();
        if !editor.is_empty() && file.is_file() && ui.button(format!("{}  Open in {}", icon::CODE, editor_name(&editor))).clicked() {
            ui.close();
            if let Err(error) = open_in_editor(&editor, &file) {
                self.notify(error, true);
            }
        }
        if file.is_file() && ui.button(format!("{}  Open in default app", icon::ARROW_SQUARE_OUT)).clicked() {
            ui.close();
            open_in_default_app(&file);
        }
        if ui.button(format!("{}  Show in file manager", icon::FOLDER_OPEN)).clicked() {
            ui.close();
            reveal(&file);
        }
    }

    fn commit_box(&mut self, ui: &mut Ui, snapshot: &Snapshot, staged: usize) {
        let c = theme::of(ui);
        ui.horizontal(|ui| {
            ui.label(RichText::new(icon::GIT_COMMIT).size(18.0).color(c.accent));
            ui.label(RichText::new("Commit").size(15.0).strong());
        });
        ui.add_space(6.0);
        // A commit made away from any branch is easily lost; outside a rebase or similar, where
        // that is expected, offer a branch first.
        if !snapshot.is_on_branch() && snapshot.head_hash.is_some() && snapshot.operation.is_none() {
            widgets::callout(
                ui,
                "You are not on a branch, so a commit here would belong to no branch and could be lost. Create a branch first.",
                true,
            );
            ui.add_space(4.0);
            if ui.add_enabled(self.idle(), egui::Button::new(format!("{}  Create branch here…", icon::GIT_BRANCH))).clicked() {
                self.dialog = Some(crate::ui::dialogs::Dialog::input(
                    "New branch",
                    "Create a branch at HEAD and switch to it",
                    "",
                    crate::ui::dialogs::InputKind::CreateBranch {
                        branch: snapshot.current_branch.clone(),
                        head: snapshot.head_hash.clone(),
                    },
                ));
            }
            ui.add_space(6.0);
        }
        let can_amend = snapshot.is_on_branch() && snapshot.head_hash.is_some() && snapshot.operation.is_none();
        let head_published = self.head_published();
        let repo = &mut self.repos[self.active];
        let path = repo.path.clone();
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
                            .desired_width((ui.available_width() - 124.0).max(40.0)),
                    )
                    .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, "Commit summary"));
                    // Characters used out of the 72-character guideline, e.g. "0/72".
                    let length = repo.draft.summary.chars().count();
                    let color = if length > 72 { c.warning } else { c.muted };
                    ui.label(RichText::new(format!("{length}/72")).small().monospace().color(color))
                        .on_hover_text("Summaries read best under 72 characters");
                    // Recent summaries from this branch's history, to reuse one.
                    let mut recent: Vec<&str> = Vec::new();
                    for commit in snapshot.commits.iter().filter(|commit| commit.parents.len() <= 1) {
                        if recent.len() == 10 {
                            break;
                        }
                        if !recent.contains(&commit.subject.as_str()) {
                            recent.push(&commit.subject);
                        }
                    }
                    // Without a frame at rest, like the panel's other icon buttons.
                    let visuals = &mut ui.visuals_mut().widgets.inactive;
                    visuals.weak_bg_fill = egui::Color32::TRANSPARENT;
                    visuals.bg_stroke = egui::Stroke::NONE;
                    if !recent.is_empty() {
                        let menu = ui.menu_button(RichText::new(icon::CLOCK_COUNTER_CLOCKWISE).color(c.muted), |ui| {
                            ui.label(RichText::new("Use a recent summary").small().color(c.muted));
                            for subject in &recent {
                                if ui.add(egui::Button::new(*subject).truncate()).clicked() {
                                    repo.draft.summary = subject.to_string();
                                    ui.close();
                                }
                            }
                        });
                        menu.response
                            .on_hover_text("Recent commit summaries")
                            .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Recent commit summaries"));
                    }
                    // The menu is read only while open: its identity is cached until it closes.
                    let (menu, open) = egui::containers::menu::MenuButton::new(RichText::new(icon::USERS).color(c.muted))
                        .config(
                            egui::containers::menu::MenuConfig::new()
                                .close_behavior(egui::containers::PopupCloseBehavior::CloseOnClickOutside),
                        )
                        .ui(ui, |ui| {
                            let (name, email) =
                                repo.trailer_identity.get_or_insert_with(|| nicegit_core::GitClient::new().identity(&path)).clone();
                            add_trailers_menu(ui, repo, snapshot, &name, &email);
                        });
                    if open.is_none() {
                        repo.trailer_identity = None;
                    }
                    menu.on_hover_text("Add trailers")
                        .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Add trailers"));
                });
                ui.add(
                    egui::TextEdit::multiline(&mut repo.draft.description)
                        .hint_text("Description")
                        .frame(egui::Frame::NONE)
                        .desired_rows(4)
                        .desired_width(f32::INFINITY),
                )
                .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, "Commit description"));
                if repo.draft_from_template.as_ref() == Some(&repo.draft) {
                    ui.label(RichText::new("From commit.template").small().color(c.muted));
                }
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
            // An untouched commit template gives way to the last commit's message, as an empty draft does.
            let repo = &self.repos[self.active];
            let replaceable = repo.draft.is_empty() || repo.draft_from_template.as_ref() == Some(&repo.draft);
            if amend && replaceable {
                if let (Some(head), Some(repo)) = (snapshot.head_hash.clone(), self.repo()) {
                    if let Ok(message) = nicegit_core::GitClient::new().commit_message(&head, &repo.path) {
                        self.repos[self.active].draft = crate::settings::Draft::from_message(&message);
                        self.repos[self.active].draft_from_template = None;
                    }
                }
            }
        }
        ui.add_space(6.0);
        let repo = &self.repos[self.active];
        let label = if repo.draft.summary.trim().is_empty() {
            format!("{}  Write a summary to commit", icon::CHECK_CIRCLE)
        } else if repo.draft_is_untouched_template() {
            format!("{}  Fill in the template to commit", icon::CHECK_CIRCLE)
        } else if repo.amend {
            format!("{}  Amend last commit", icon::CHECK_CIRCLE)
        } else if staged == 0 {
            format!("{}  Stage files to commit", icon::CHECK_CIRCLE)
        } else {
            format!("{}  Commit {staged} file{} to {}", icon::CHECK_CIRCLE, if staged == 1 { "" } else { "s" }, snapshot.current_branch)
        };
        let enabled = self.can_commit();
        // Outlined and centred even while disabled, so it reads as the commit button rather
        // than another text field.
        let button = egui::Button::new(RichText::new(label).strong().color(if enabled { c.accent_text } else { c.muted }))
            .fill(if enabled { c.accent } else { ui.visuals().widgets.inactive.weak_bg_fill })
            .stroke(egui::Stroke::new(1.0, if enabled { c.accent } else { c.border }))
            .min_size(egui::vec2(ui.available_width(), 36.0));
        let clicked = ui
            .with_layout(egui::Layout::top_down_justified(egui::Align::Center), |ui| ui.add_enabled(enabled, button))
            .inner
            .on_hover_text("Ctrl/Cmd-Enter")
            .clicked();
        if clicked {
            self.commit();
        }
        // With a summary written but nothing staged, everything can be staged and committed at once.
        if self.can_stage_all_and_commit() {
            ui.add_space(4.0);
            let changes = snapshot.status.len();
            let text = RichText::new(format!("{}  Stage all {changes} and commit", icon::CHECK_CIRCLE)).color(c.accent);
            let button = egui::Button::new(text)
                .fill(egui::Color32::TRANSPARENT)
                .stroke(egui::Stroke::new(1.0, c.accent))
                .min_size(egui::vec2(ui.available_width(), 28.0));
            let all = ui
                .with_layout(egui::Layout::top_down_justified(egui::Align::Center), |ui| ui.add(button))
                .inner
                .on_hover_text("Stage every change, as Stage All Changes does, then commit");
            if all.clicked() {
                self.stage_all_and_commit();
            }
        }
        // Committing and pushing in one step, for a branch that already has an upstream.
        if let (Some(upstream), false) = (snapshot.upstream.as_deref(), self.repos[self.active].amend) {
            ui.add_space(4.0);
            let enabled = self.can_commit_and_push();
            // The upstream is in the tooltip, so a long branch name cannot widen the panel.
            let text = RichText::new(format!("{}  Commit and push", icon::ARROW_LINE_UP)).color(if enabled { c.accent } else { c.muted });
            let button = egui::Button::new(text)
                .fill(egui::Color32::TRANSPARENT)
                .stroke(egui::Stroke::new(1.0, if enabled { c.accent } else { c.border }))
                .min_size(egui::vec2(ui.available_width(), 28.0));
            let push = ui
                .with_layout(egui::Layout::top_down_justified(egui::Align::Center), |ui| ui.add_enabled(enabled, button))
                .inner
                .on_hover_text(format!("Commit, then push to {upstream} (Shift-Ctrl/Cmd-Enter)"));
            if push.clicked() {
                self.commit_and_push();
            }
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

/// The Add trailers menu: Signed-off-by from the Git identity, and Co-authored-by for recent
/// authors. Each row toggles its line in the description, so the text shows what will be
/// committed. The menu stays open while several rows are toggled.
fn add_trailers_menu(ui: &mut Ui, repo: &mut Repo, snapshot: &Snapshot, name: &str, email: &str) {
    let c = theme::of(ui);
    let (name, email) = (name.trim(), email.trim());
    let identity_set = !name.is_empty() && !email.is_empty();
    // Headed like the co-author list below, so the row says which line it adds.
    ui.label(RichText::new("Signed-off-by").small().color(c.muted));
    let signed_off = format!("Signed-off-by: {name} <{email}>");
    let on = identity_set && repo.draft.has_trailer(&signed_off);
    let label = signed_off.clone();
    let response = ui
        .add_enabled(identity_set, egui::Button::new(trailer_row(on, &format!("{name} <{email}>"))).truncate())
        .on_disabled_hover_text("Set your Git name and email first");
    response.widget_info(move || egui::WidgetInfo::labeled(egui::WidgetType::Button, identity_set, label.clone()));
    if response.clicked() {
        repo.draft.toggle_trailer(&signed_off);
    }

    ui.separator();
    ui.label(RichText::new("Co-authored-by").small().color(c.muted));
    // Recent distinct authors, most recent first, leaving out the current user and bots.
    let current = email.to_lowercase();
    let mut seen = std::collections::HashSet::new();
    let mut authors: Vec<(String, String)> = Vec::new();
    for commit in &snapshot.commits {
        if authors.len() == 20 {
            break;
        }
        let (author, address) = (commit.author_name.trim(), commit.author_email.trim());
        if author.is_empty() || address.is_empty() {
            continue;
        }
        let key = address.to_lowercase();
        if !seen.insert(key.clone()) || key == current || is_bot(author, &key) {
            continue;
        }
        authors.push((author.to_string(), address.to_string()));
    }
    if authors.is_empty() {
        ui.label(RichText::new("No other authors in the loaded history").small().color(c.muted));
        return;
    }
    if authors.len() > 8 {
        ui.add(egui::TextEdit::singleline(&mut repo.trailer_filter).hint_text("Filter authors").desired_width(220.0))
            .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, "Filter authors"));
    }
    let needle = repo.trailer_filter.trim().to_lowercase();
    let shown: Vec<&(String, String)> = authors
        .iter()
        .filter(|(author, address)| needle.is_empty() || format!("{author} <{address}>").to_lowercase().contains(&needle))
        .collect();
    if shown.is_empty() {
        ui.label(RichText::new("No authors match").small().color(c.muted));
    }
    for (author, address) in shown {
        let text = format!("{author} <{address}>");
        let trailer = format!("Co-authored-by: {text}");
        let on = repo.draft.has_trailer(&trailer);
        let label = trailer.clone();
        let response = ui.add(egui::Button::new(trailer_row(on, &text)).truncate());
        response.widget_info(move || egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label.clone()));
        if response.clicked() {
            repo.draft.toggle_trailer(&trailer);
        }
    }
}

/// A menu row's text, led by a check when the row's trailer is in the description.
fn trailer_row(on: bool, text: &str) -> String {
    format!("{}  {text}", if on { icon::CHECK } else { "   " })
}

/// Bot accounts that should not be offered as co-authors: GitHub's bot noreply addresses and
/// the GitHub Actions identity. `email` is lower-cased.
fn is_bot(name: &str, email: &str) -> bool {
    (email.contains("noreply@github.com") && name.contains("[bot]")) || email.contains("github-actions")
}

/// A size in whole megabytes, as file sizes are usually given.
fn megabytes(bytes: u64) -> String {
    format!("{} MB", bytes / 1_000_000)
}

/// The number of changed files above which the Changes panel offers a filter.
const FILTER_FROM: usize = 5;

/// Whether a changed file matches the Changes panel's filter. Every whitespace-separated term must
/// appear, ignoring case, in the file's path or, for a rename or copy, in its original path. An
/// empty filter matches every file.
pub(crate) fn matches_filter(entry: &StatusEntry, filter: &str) -> bool {
    let path = entry.path.to_lowercase();
    let original = entry.original_path.as_deref().map(str::to_lowercase);
    filter.split_whitespace().all(|term| {
        let term = term.to_lowercase();
        path.contains(&term) || original.as_deref().is_some_and(|original| original.contains(&term))
    })
}

/// Temporary context data: whether the filter field had focus when the panel was last drawn, and
/// whether a shortcut asked for it to be focused. Egui drops a field's focus on Escape before the
/// shortcuts run, so the state from the previous frame is what tells Escape to clear the field.
fn filter_focus_key() -> egui::Id {
    egui::Id::new("changes_filter_focused")
}

fn filter_focus_request_key() -> egui::Id {
    egui::Id::new("changes_filter_focus_request")
}

/// Whether the filter field had focus when the Changes panel was drawn in the previous pass. A
/// panel that was not drawn then, because an inspector took its place, counts as unfocused.
pub(crate) fn filter_had_focus(ctx: &egui::Context) -> bool {
    let current = ctx.cumulative_pass_nr();
    ctx.data(|data| data.get_temp::<(u64, bool)>(filter_focus_key())).is_some_and(|(pass, focused)| focused && pass + 1 >= current)
}

/// Asks the Changes panel to show the filter field and focus it on the next frame.
pub(crate) fn request_filter_focus(ctx: &egui::Context) {
    ctx.data_mut(|data| data.insert_temp(filter_focus_request_key(), true));
}

/// Takes a pending request to focus the filter field.
fn filter_focus_requested(ctx: &egui::Context) -> bool {
    let requested = ctx.data(|data| data.get_temp::<bool>(filter_focus_request_key())).unwrap_or(false);
    if requested {
        ctx.data_mut(|data| data.insert_temp(filter_focus_request_key(), false));
    }
    requested
}

/// How many lines a file's change adds and removes, laid out from the right.
pub(crate) fn line_count(ui: &mut Ui, count: LineCount) {
    let c = theme::of(ui);
    ui.add_space(4.0);
    let (text, tip) = match count {
        LineCount::Binary => (RichText::new("binary").small().color(c.muted), "A binary file has no lines".to_string()),
        LineCount::Text { added, removed } => {
            let lines = |n: usize| if n == 1 { "line" } else { "lines" };
            let tip = format!("{added} {} added, {removed} {} removed", lines(added), lines(removed));
            // Right to left: the removed count sits to the right of the added one.
            if removed > 0 {
                ui.label(RichText::new(format!("−{removed}")).small().monospace().color(c.removed)).on_hover_text(&tip);
            }
            if added == 0 && removed > 0 {
                return;
            }
            (RichText::new(format!("+{added}")).small().monospace().color(c.added), tip)
        }
    };
    ui.label(text).on_hover_text(tip);
}

fn sorted(mut entries: Vec<StatusEntry>) -> Vec<StatusEntry> {
    entries.sort_by_key(|a| a.path.to_lowercase());
    entries
}

#[cfg(test)]
thread_local! {
    /// What interface tests asked the file manager to show, instead of opening its windows.
    pub static REVEALED: std::cell::RefCell<Vec<std::path::PathBuf>> = const { std::cell::RefCell::new(Vec::new()) };
    /// Files the interface asked the system to open, recorded under test instead of opened.
    pub static OPENED: std::cell::RefCell<Vec<std::path::PathBuf>> = const { std::cell::RefCell::new(Vec::new()) };
    /// Files the interface asked an external editor to open, with its command, under test.
    pub static EDITED: std::cell::RefCell<Vec<(String, std::path::PathBuf)>> = const { std::cell::RefCell::new(Vec::new()) };
    /// Web addresses the interface asked the browser to open, recorded under test instead.
    pub static OPENED_URLS: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// Shows a file in the platform's file manager.
/// Opens a file in the app the system uses for its type. `path` is absolute, so no file name
/// can be read as an option.
pub fn open_in_default_app(path: &std::path::Path) {
    #[cfg(test)]
    OPENED.with(|opened| opened.borrow_mut().push(path.to_path_buf()));
    #[cfg(all(not(test), target_os = "macos"))]
    let _ = std::process::Command::new("open").arg(path).spawn();
    #[cfg(all(not(test), target_os = "windows"))]
    let _ = std::process::Command::new("explorer").arg(path).spawn();
    #[cfg(all(not(test), unix, not(target_os = "macos")))]
    let _ = std::process::Command::new("xdg-open").arg(path).spawn();
}

/// What to call an external editor command in menus: a well-known editor's name, or the
/// program's own name.
pub fn editor_name(command: &str) -> String {
    let program = command.split_whitespace().next().unwrap_or_default();
    let base = std::path::Path::new(program).file_stem().and_then(|stem| stem.to_str()).unwrap_or(program);
    match base {
        "code" => "VS Code".into(),
        "code-insiders" => "VS Code Insiders".into(),
        "cursor" => "Cursor".into(),
        "zed" => "Zed".into(),
        "subl" => "Sublime Text".into(),
        "idea" => "IntelliJ IDEA".into(),
        "mate" => "TextMate".into(),
        "nova" => "Nova".into(),
        other => other.to_string(),
    }
}

/// Opens a file with the external editor `command`: its first word is the program, later words
/// are passed before the file's absolute path. The program is looked up on NiceGit's search
/// path, which includes common install folders an app opened from the desktop may not inherit.
pub fn open_in_editor(command: &str, file: &std::path::Path) -> Result<(), String> {
    let mut words = command.split_whitespace();
    let Some(program) = words.next() else { return Err("Set an external editor in Settings first.".into()) };
    let arguments: Vec<&str> = words.collect();
    #[cfg(test)]
    {
        let _ = (program, &arguments);
        EDITED.with(|edited| edited.borrow_mut().push((command.to_string(), file.to_path_buf())));
        Ok(())
    }
    #[cfg(not(test))]
    std::process::Command::new(program)
        .args(&arguments)
        .arg(file)
        .env("PATH", nicegit_core::runner::search_path())
        .stdin(std::process::Stdio::null())
        .spawn()
        .map(drop)
        .map_err(|error| format!("Could not start {}: {error}", editor_name(command)))
}

/// Opens a web address in the default browser. Only `https://` addresses are opened.
pub fn open_url(url: &str) {
    if !url.starts_with("https://") {
        return;
    }
    #[cfg(test)]
    OPENED_URLS.with(|opened| opened.borrow_mut().push(url.to_string()));
    #[cfg(all(not(test), target_os = "macos"))]
    let _ = std::process::Command::new("open").arg(url).spawn();
    #[cfg(all(not(test), target_os = "windows"))]
    let _ = std::process::Command::new("explorer").arg(url).spawn();
    #[cfg(all(not(test), unix, not(target_os = "macos")))]
    let _ = std::process::Command::new("xdg-open").arg(url).spawn();
}

pub fn reveal(path: &std::path::Path) {
    let target = if path.exists() { path.to_path_buf() } else { path.parent().map(|p| p.to_path_buf()).unwrap_or_default() };
    #[cfg(test)]
    REVEALED.with(|revealed| revealed.borrow_mut().push(target));
    #[cfg(all(not(test), target_os = "macos"))]
    let _ = std::process::Command::new("open").arg("-R").arg(&target).spawn();
    #[cfg(all(not(test), target_os = "windows"))]
    let _ = std::process::Command::new("explorer").arg(format!("/select,{}", target.display())).spawn();
    #[cfg(all(not(test), unix, not(target_os = "macos")))]
    let _ = std::process::Command::new("xdg-open").arg(target.parent().unwrap_or(&target)).spawn();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(path: &str) -> StatusEntry {
        StatusEntry { path: path.into(), original_path: None, kind: StatusKind::Modified, index_status: ' ', work_tree_status: 'M' }
    }

    #[test]
    fn filter_empty_matches_everything() {
        assert!(matches_filter(&entry("src/app.rs"), ""));
        assert!(matches_filter(&entry("src/app.rs"), "   "));
    }

    #[test]
    fn filter_is_case_insensitive() {
        assert!(matches_filter(&entry("docs/Guide.md"), "guide"));
        assert!(matches_filter(&entry("docs/guide.md"), "GUIDE.MD"));
        assert!(!matches_filter(&entry("docs/guide.md"), "readme"));
    }

    #[test]
    fn filter_needs_every_term() {
        let file = entry("src/app/test_utils.rs");
        assert!(matches_filter(&file, "src test"));
        assert!(matches_filter(&file, "  test   utils  "));
        assert!(!matches_filter(&file, "src docs"));
    }

    #[test]
    fn filter_matches_a_rename_by_its_original_path() {
        let renamed = StatusEntry {
            path: "lib/new_name.rs".into(),
            original_path: Some("src/old_name.rs".into()),
            kind: StatusKind::Renamed,
            index_status: 'R',
            work_tree_status: ' ',
        };
        assert!(matches_filter(&renamed, "old_name"));
        assert!(matches_filter(&renamed, "lib old"), "terms can match the path and the original in turn");
        assert!(!matches_filter(&renamed, "missing"));
    }

    #[test]
    fn filter_matches_paths_with_spaces() {
        let file = entry("notes/my file.txt");
        assert!(matches_filter(&file, "my file"));
        assert!(matches_filter(&file, "file.txt"));
        assert!(!matches_filter(&file, "other"));
    }

    #[test]
    fn filter_matches_unicode_ignoring_case() {
        let file = entry("café/Ünïcode.txt");
        assert!(matches_filter(&file, "ünï"));
        assert!(matches_filter(&file, "ÜNÏCODE"));
        assert!(matches_filter(&file, "CAFÉ"));
    }
}
