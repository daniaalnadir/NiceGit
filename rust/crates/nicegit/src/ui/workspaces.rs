//! Workspaces: named groups of repositories that open together. Saving records the open tabs in
//! tab order; opening reuses `open`, which loads each repository in the background, and skips
//! the tabs already open; Manage workspaces renames and deletes saved workspaces. None of these
//! changes a folder on disk.

use std::path::PathBuf;

use egui::{Key, RichText, Ui};
use egui_phosphor::regular as icon;

use crate::app::NiceGitApp;
use crate::settings::Workspace;
use crate::theme;
use crate::tools::widgets;
use crate::ui::dialogs::{Dialog, InputKind, Pending};

/// What opening a workspace does with its folders, worked out before any tab opens.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct OpenPlan {
    /// Folders that exist and are not open yet, in the workspace's order.
    pub open: Vec<PathBuf>,
    /// Folders that are no longer on disk.
    pub missing: Vec<PathBuf>,
    /// Folders the workspace names that are already open as tabs.
    pub already_open: Vec<PathBuf>,
}

/// Splits a workspace's folders into the ones to open, the missing ones, and those already open.
/// A folder listed twice is opened once.
pub fn plan_open(workspace: &Workspace, open_now: &[PathBuf]) -> OpenPlan {
    let mut plan = OpenPlan::default();
    for path in &workspace.repositories {
        if plan.open.contains(path) || plan.missing.contains(path) || plan.already_open.contains(path) {
            continue;
        }
        if open_now.contains(path) {
            plan.already_open.push(path.clone());
        } else if path.exists() {
            plan.open.push(path.clone());
        } else {
            plan.missing.push(path.clone());
        }
    }
    plan
}

/// What the Manage workspaces window asks for once it is drawn.
enum Manage {
    StartRename(String),
    CancelRename,
    Rename { from: String, to: String },
    Delete(String),
}

/// Why `name` cannot name a workspace being renamed, or None when it can. `current` is the name
/// the workspace has now, which the same name may keep.
fn rename_problem(name: &str, current: &str, workspaces: &[Workspace]) -> Option<&'static str> {
    if name.is_empty() {
        Some("Give the workspace a name.")
    } else if name != current && workspaces.iter().any(|workspace| workspace.name == name) {
        Some("Another workspace already has this name.")
    } else {
        None
    }
}

/// "1 repository" or "3 repositories", as the menus and notices say it.
pub(crate) fn repository_count(count: usize) -> String {
    format!("{count} repositor{}", if count == 1 { "y" } else { "ies" })
}

impl NiceGitApp {
    /// The Workspaces menu in the repositories column's header.
    pub fn workspaces_menu(&mut self, ui: &mut Ui) {
        let can_open = self.busy.is_none() && !self.settings.workspaces.is_empty();
        let can_save = !self.repos.is_empty();
        let mut save = false;
        let mut open: Option<String> = None;
        let mut manage = false;
        let menu = ui.menu_button(RichText::new(icon::FOLDERS).size(15.0), |ui| {
            if ui.add_enabled(can_save, egui::Button::new(format!("{}  Save open tabs as workspace…", icon::FLOPPY_DISK))).clicked() {
                save = true;
                ui.close();
            }
            ui.add_enabled_ui(can_open, |ui| {
                ui.menu_button(format!("{}  Open workspace", icon::FOLDER_OPEN), |ui| {
                    for workspace in &self.settings.workspaces {
                        let count = repository_count(workspace.repositories.len());
                        let button = ui.button(format!("{}  {}", icon::FOLDER, workspace.name)).on_hover_text(count);
                        if button.clicked() {
                            open = Some(workspace.name.clone());
                            ui.close();
                        }
                    }
                });
            });
            if ui.button(format!("{}  Manage workspaces…", icon::GEAR_SIX)).clicked() {
                manage = true;
                ui.close();
            }
        });
        menu.response.on_hover_text("Workspaces").widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, "Workspaces"));
        if save {
            self.start_save_workspace();
        }
        if let Some(name) = open {
            self.open_workspace(&name);
        }
        if manage {
            self.show_workspaces = true;
        }
    }

    /// Asks for a name for the open tabs, in tab order. Refused with no tabs open.
    pub fn start_save_workspace(&mut self) {
        let repositories: Vec<PathBuf> = self.repos.iter().map(|repo| repo.path.clone()).collect();
        if repositories.is_empty() {
            return;
        }
        self.dialog = Some(Dialog::input("Save workspace", "Workspace name", "", InputKind::SaveWorkspace { repositories }));
    }

    /// Saves the folders under `name`. A name already in use is replaced only once confirmed.
    pub fn save_workspace(&mut self, name: String, repositories: Vec<PathBuf>) {
        if name.is_empty() {
            self.notify("Give the workspace a name.", true);
            return;
        }
        if self.settings.workspaces.iter().any(|workspace| workspace.name == name) {
            let count = repository_count(repositories.len());
            self.confirm(
                format!("Replace workspace {name}?"),
                format!("It will hold the {count} open now instead of the folders it holds. Folders on disk are not changed."),
                "Replace",
                Pending::ReplaceWorkspace { name, repositories },
            );
            return;
        }
        self.store_workspace(name, repositories);
    }

    /// Stores a workspace, replacing the one with the same name if there is one.
    pub fn store_workspace(&mut self, name: String, repositories: Vec<PathBuf>) {
        let message = format!("Saved workspace {name} with {}.", repository_count(repositories.len()));
        match self.settings.workspaces.iter_mut().find(|workspace| workspace.name == name) {
            Some(workspace) => workspace.repositories = repositories,
            None => self.settings.workspaces.push(Workspace { name, repositories }),
        }
        self.notify(message, false);
    }

    /// Opens a workspace's folders as tabs. Tabs already open are skipped, and folders that are
    /// gone are reported in one notice. Refused while an action runs or the editor has unsaved
    /// text, as opening a repository is.
    pub fn open_workspace(&mut self, name: &str) {
        if self.busy.is_some() {
            return;
        }
        let Some(workspace) = self.settings.workspaces.iter().find(|workspace| workspace.name == name).cloned() else {
            return;
        };
        if self.has_unsaved_editor() {
            self.notify("Save or close the file you are editing before opening another repository.", true);
            return;
        }
        let open_now: Vec<PathBuf> = self.repos.iter().map(|repo| repo.path.clone()).collect();
        let plan = plan_open(&workspace, &open_now);
        let opened = plan.open.len();
        for path in plan.open {
            // Each folder loads in the background, so the tabs open without waiting for Git.
            self.open(path);
        }
        if !plan.missing.is_empty() {
            let folders: Vec<String> = plan.missing.iter().map(|path| path.display().to_string()).collect();
            let plural = if folders.len() == 1 { "folder" } else { "folders" };
            self.notify(
                format!("Workspace {name} has {} missing {plural}, which did not open: {}.", folders.len(), folders.join("; ")),
                true,
            );
        } else if opened == 0 {
            self.notify(format!("Every repository in workspace {name} is already open."), false);
        } else {
            self.notify(format!("Opened workspace {name}."), false);
        }
    }

    /// Removes a saved workspace from the list. The folders stay as they are.
    pub fn delete_workspace(&mut self, name: &str) {
        self.settings.workspaces.retain(|workspace| workspace.name != name);
        if self.workspace_rename.as_ref().is_some_and(|(from, _)| from == name) {
            self.workspace_rename = None;
        }
        self.notify(format!("Deleted workspace {name}. Its folders are unchanged."), false);
    }

    /// Manage workspaces: rename a workspace in place, or delete it after a confirmation.
    pub fn workspaces_window(&mut self, ctx: &egui::Context) {
        if !self.show_workspaces {
            self.workspace_rename = None;
            return;
        }
        let mut open = true;
        let mut requests: Vec<Manage> = Vec::new();
        let workspaces = self.settings.workspaces.clone();
        egui::Window::new("Workspaces")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(460.0)
            .pivot(egui::Align2::CENTER_CENTER)
            .default_pos(ctx.content_rect().center())
            .show(ctx, |ui| {
                let c = theme::of(ui);
                if workspaces.is_empty() {
                    ui.label(
                        RichText::new(
                            "No saved workspaces. Save the open tabs as one from the Workspaces menu in the repositories column.",
                        )
                        .color(c.muted),
                    );
                    return;
                }
                for workspace in &workspaces {
                    let renaming = self.workspace_rename.as_ref().is_some_and(|(from, _)| from == &workspace.name);
                    if renaming {
                        let Some((from, draft)) = self.workspace_rename.as_mut() else { continue };
                        let from = from.clone();
                        let problem = rename_problem(draft.trim(), &from, &workspaces);
                        ui.label(RichText::new("Workspace name").small().color(c.muted));
                        let field = widgets::text_field(ui, draft, "");
                        let label = format!("Name of {from}");
                        field.widget_info(|| egui::WidgetInfo::labeled(egui::Role::TextInput, true, &label));
                        if ui.memory(|m| m.focused().is_none()) {
                            field.request_focus();
                        }
                        if let Some(problem) = problem {
                            ui.label(RichText::new(problem).small().color(c.warning));
                        }
                        let enter = field.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
                        let to = draft.trim().to_string();
                        ui.horizontal(|ui| {
                            let rename = widgets::primary_button(ui, "Rename", problem.is_none());
                            if (rename.clicked() || (enter && problem.is_none())) && problem.is_none() {
                                requests.push(Manage::Rename { from: from.clone(), to: to.clone() });
                            }
                            if ui.button("Cancel").clicked() {
                                requests.push(Manage::CancelRename);
                            }
                        });
                    } else {
                        ui.horizontal(|ui| {
                            let name = RichText::new(workspace.name.as_str()).strong();
                            let row = ui.label(name).on_hover_text(
                                workspace.repositories.iter().map(|path| path.display().to_string()).collect::<Vec<_>>().join("\n"),
                            );
                            row.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, true, &workspace.name));
                            ui.label(RichText::new(repository_count(workspace.repositories.len())).small().color(c.muted));
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                let delete = ui.button(format!("{}  Delete…", icon::TRASH));
                                delete.widget_info(|| {
                                    egui::WidgetInfo::labeled(egui::Role::Button, true, format!("Delete {}", workspace.name))
                                });
                                if delete.clicked() {
                                    requests.push(Manage::Delete(workspace.name.clone()));
                                }
                                let rename = ui.button("Rename…");
                                rename.widget_info(|| {
                                    egui::WidgetInfo::labeled(egui::Role::Button, true, format!("Rename {}", workspace.name))
                                });
                                if rename.clicked() {
                                    requests.push(Manage::StartRename(workspace.name.clone()));
                                }
                            });
                        });
                    }
                    ui.add_space(4.0);
                }
            });
        if !open {
            self.show_workspaces = false;
            self.workspace_rename = None;
        }
        for request in requests {
            match request {
                Manage::StartRename(name) => self.workspace_rename = Some((name.clone(), name)),
                Manage::CancelRename => self.workspace_rename = None,
                Manage::Rename { from, to } => {
                    self.workspace_rename = None;
                    if let Some(workspace) = self.settings.workspaces.iter_mut().find(|workspace| workspace.name == from) {
                        workspace.name = to.clone();
                        self.notify(format!("Renamed workspace {from} to {to}."), false);
                    }
                }
                Manage::Delete(name) => self.confirm(
                    format!("Delete workspace {name}?"),
                    "The saved list is removed. Its folders stay on disk, and any tabs open from it stay open.",
                    "Delete",
                    Pending::DeleteWorkspace { name },
                ),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn workspace(paths: &[&PathBuf]) -> Workspace {
        Workspace { name: "Client work".into(), repositories: paths.iter().map(|path| (*path).clone()).collect() }
    }

    #[test]
    fn opening_skips_open_and_repeated_folders_and_reports_missing_ones() {
        let dir = tempfile::tempdir().expect("temporary folder");
        let (a, b) = (dir.path().join("a"), dir.path().join("b"));
        let gone = dir.path().join("gone");
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        let saved = workspace(&[&a, &b, &gone, &b]);

        let plan = plan_open(&saved, std::slice::from_ref(&a));
        assert_eq!(plan.open, std::slice::from_ref(&b), "the folder that is not open yet opens, once");
        assert_eq!(plan.already_open, std::slice::from_ref(&a), "the open tab is skipped");
        assert_eq!(plan.missing, std::slice::from_ref(&gone), "the folder that is gone is reported");

        let plan = plan_open(&saved, &[a.clone(), b.clone()]);
        assert!(plan.open.is_empty(), "nothing opens when every folder is open");
        assert_eq!(plan.already_open.len(), 2);
    }

    #[test]
    fn a_rename_keeps_its_own_name_and_refuses_another_workspaces() {
        let workspaces = vec![
            Workspace { name: "Client work".into(), repositories: Vec::new() },
            Workspace { name: "Scratch".into(), repositories: Vec::new() },
        ];
        assert_eq!(rename_problem("", "Client work", &workspaces), Some("Give the workspace a name."));
        assert_eq!(rename_problem("Scratch", "Client work", &workspaces), Some("Another workspace already has this name."));
        assert_eq!(rename_problem("Client work", "Client work", &workspaces), None);
        assert_eq!(rename_problem("Clients", "Client work", &workspaces), None);
    }
}
