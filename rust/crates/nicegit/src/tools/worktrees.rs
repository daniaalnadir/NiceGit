#![allow(dead_code)]
//! Linked worktrees: listing them, opening and removing them, forgetting missing ones, and
//! creating a new one for a local branch.

use crate::i18n::{tr, tr_fmt};
use std::path::PathBuf;

use egui::{RichText, Ui};
use egui_phosphor::regular as icon;
use nicegit_core::{GitClient, Snapshot, Worktree};

use crate::theme;
use crate::tools::{query, widgets, Ctx, Task, ToolWindow};

/// A local branch picked for a new worktree, with the tip that was shown when it was picked.
#[derive(Clone, Debug, PartialEq, Eq)]
struct BranchChoice {
    name: String,
    tip: String,
}

/// A button press on one worktree card, handled once the card is drawn.
enum WorktreeAction {
    Open(String),
    Copy(String),
    AskRemove(String),
    CancelRemove,
    ConfirmRemove(String),
}

/// Lists the repository's linked worktrees and creates new ones. Every change is confirmed
/// against the state shown when it was chosen.
pub struct WorktreesWindow {
    confirming_removal: Option<String>,
    /// Set when a removal starts; the list confirms it once the worktree is gone.
    removing: Option<String>,
    choice: Option<BranchChoice>,
    /// The folder that will contain the new worktree, typed or chosen with the folder dialog.
    parent: String,
    folder_name: String,
    /// The branch tip read right after the folder dialog closed, and the choice it was read for.
    tip_check: Option<(BranchChoice, Task<nicegit_core::Result<String>>)>,
}

impl WorktreesWindow {
    pub fn new() -> Self {
        Self { confirming_removal: None, removing: None, choice: None, parent: String::new(), folder_name: String::new(), tip_check: None }
    }

    /// Opens with `branch` already chosen for a new worktree, as its context menu does.
    pub fn for_branch(name: String, tip: String) -> Self {
        let folder_name = name.replace('/', "-");
        Self { choice: Some(BranchChoice { name, tip }), folder_name, ..Self::new() }
    }

    fn parent_folder(&self) -> Option<PathBuf> {
        let text = self.parent.trim();
        (!text.is_empty()).then(|| PathBuf::from(text))
    }

    fn worktree_list(&mut self, ui: &mut Ui, cx: &mut Ctx) {
        let snapshot: &Snapshot = cx.snapshot;
        let idle = cx.idle;
        let missing = snapshot.worktrees.iter().filter(|worktree| worktree.is_prunable).count();

        widgets::section(ui, tr("Worktrees"));
        if snapshot.worktrees.is_empty() {
            widgets::empty_state(ui, icon::FOLDER, tr("No worktrees were listed for this repository."));
        }
        let mut actions = Vec::new();
        for (index, worktree) in snapshot.worktrees.iter().enumerate() {
            if let Some(action) = worktree_card(ui, idle, snapshot, index == 0, worktree, self.confirming_removal.as_deref()) {
                actions.push(action);
            }
            ui.add_space(6.0);
        }
        for action in actions {
            match action {
                WorktreeAction::Open(path) => cx.open_repository(PathBuf::from(path)),
                WorktreeAction::Copy(path) => ui.ctx().copy_text(path),
                WorktreeAction::AskRemove(path) => self.confirming_removal = Some(path),
                WorktreeAction::CancelRemove => self.confirming_removal = None,
                WorktreeAction::ConfirmRemove(path) => {
                    self.removing = Some(path.clone());
                    cx.act(tr_fmt!("Remove worktree {path}", path = path), move |client, dir| {
                        client.remove_worktree(&path, dir)?;
                        Ok(Some(tr("Removed the worktree. Its branch was kept.").to_string()))
                    });
                }
            }
        }

        if missing > 0 {
            ui.add_space(6.0);
            let text = if missing == 1 {
                tr("A worktree folder no longer exists on disk. Forget it to remove it from Git's list.").to_string()
            } else {
                tr_fmt!("{count} worktree folders no longer exist on disk. Forget them to remove them from Git's list.", count = missing)
            };
            widgets::callout(ui, &text, true);
            ui.add_space(4.0);
            if ui.add_enabled(idle, egui::Button::new(format!("{}  {}", icon::BROOM, tr("Forget missing worktrees")))).clicked() {
                cx.act(tr("Forget missing worktrees"), |client, dir| {
                    client.prune_worktrees(dir)?;
                    Ok(Some(tr("Forgot the missing worktrees.").to_string()))
                });
            }
        }
    }

    fn create_section(&mut self, ui: &mut Ui, cx: &mut Ctx) {
        let snapshot: &Snapshot = cx.snapshot;
        let idle = cx.idle;
        let c = theme::of(ui);

        // A branch already checked out elsewhere cannot back another worktree.
        let in_use = GitClient::branches_in_other_worktrees(snapshot);
        let candidates: Vec<_> = snapshot
            .branches
            .iter()
            .filter(|branch| !branch.is_remote && !branch.is_current && !branch.is_detached() && !in_use.contains(&branch.name))
            .collect();
        // A branch that stopped being a candidate is dropped; one that only moved is kept, and the
        // action refuses it when its shown tip no longer matches.
        if let Some(choice) = &self.choice {
            if !candidates.iter().any(|branch| branch.name == choice.name) {
                self.choice = None;
            }
        }

        widgets::section(ui, tr("Create a worktree"));
        ui.label(
            RichText::new(tr("Check a local branch out in its own folder, so you can work on it without switching this checkout."))
                .color(c.muted),
        );
        ui.add_space(8.0);

        if candidates.is_empty() {
            ui.label(RichText::new(tr("Every local branch is already checked out. Create a branch first.")).color(c.muted));
            return;
        }

        let mut picked: Option<BranchChoice> = None;
        egui::Grid::new("worktree_create_form").num_columns(2).spacing([16.0, 10.0]).show(ui, |ui| {
            ui.label(tr("Branch"));
            ui.add_enabled_ui(idle, |ui| {
                egui::ComboBox::from_id_salt("worktree_branch")
                    .selected_text(self.choice.as_ref().map(|choice| choice.name.as_str()).unwrap_or(tr("Choose a branch")))
                    .width(320.0)
                    .show_ui(ui, |ui| {
                        for branch in &candidates {
                            let selected = self.choice.as_ref().is_some_and(|choice| choice.name == branch.name);
                            if ui.selectable_label(selected, &branch.name).clicked() {
                                picked = Some(BranchChoice { name: branch.name.clone(), tip: branch.tip.clone() });
                            }
                        }
                    });
            });
            ui.end_row();

            ui.label(tr("Parent folder"));
            ui.horizontal(|ui| {
                if ui.add_enabled(idle, egui::Button::new(format!("{}  {}", icon::FOLDER_OPEN, tr("Choose folder")))).clicked() {
                    let folder = crate::file_dialog::pick_folder(tr("Choose where the new worktree folder goes"));
                    if let Some(folder) = folder {
                        self.parent = folder.display().to_string();
                        // The branch may have moved while the dialog was open; read it again.
                        if let Some(choice) = self.choice.clone() {
                            let reference = format!("refs/heads/{}", choice.name);
                            let task = query(ui.ctx(), cx.repo, move |client, dir| {
                                client
                                    .run(&["rev-parse", "--verify", "--end-of-options", &reference], dir)
                                    .map(|tip| tip.trim().to_string())
                            });
                            self.tip_check = Some((choice, task));
                        }
                    }
                }
                ui.add_enabled(
                    idle,
                    egui::TextEdit::singleline(&mut self.parent).hint_text(tr("Type a path or choose a folder")).desired_width(260.0),
                )
                .widget_info(|| egui::WidgetInfo::labeled(egui::Role::TextInput, true, tr("Parent folder")));
            });
            ui.end_row();

            ui.label(tr("Folder name"));
            let hint = self.choice.as_ref().map(|choice| folder_for(&choice.name)).unwrap_or_else(|| tr("feature-branch").to_string());
            ui.add_enabled(idle, egui::TextEdit::singleline(&mut self.folder_name).hint_text(hint).desired_width(320.0));
            ui.end_row();
        });
        if let Some(choice) = picked {
            self.choice = Some(choice);
            self.tip_check = None;
        }

        let folder = self.effective_folder();
        if let (Some(parent), Some(folder)) = (&self.parent_folder(), &folder) {
            ui.add_space(4.0);
            ui.label(RichText::new(tr_fmt!("Creates {path}", path = parent.join(folder).display())).small().color(c.muted));
        }

        ui.add_space(8.0);
        let checking = self.tip_check.is_some();
        let ready = idle && !checking && self.choice.is_some() && self.parent_folder().is_some() && folder.is_some();
        let create = ui.horizontal(|ui| {
            if checking {
                widgets::loading(ui, tr("Checking the branch"));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                widgets::primary_button(ui, &format!("{}  {}", icon::PLUS, tr("Create worktree")), ready).clicked()
            })
            .inner
        });
        if create.inner {
            if let (Some(choice), Some(parent), Some(folder)) = (self.choice.clone(), self.parent_folder(), folder) {
                let destination = parent.join(folder);
                let BranchChoice { name, tip } = choice;
                cx.act(tr_fmt!("Create worktree for {name}", name = name), move |client, dir| {
                    client.create_worktree(&name, &tip, &destination, dir)?;
                    Ok(Some(tr_fmt!("Created a worktree for {name} at {path}.", name = name, path = destination.display())))
                });
            }
        }
    }

    /// The folder name to create: the typed one, or one derived from the branch.
    fn effective_folder(&self) -> Option<String> {
        let typed = self.folder_name.trim();
        let folder = if typed.is_empty() { self.choice.as_ref().map(|choice| folder_for(&choice.name))? } else { typed.to_string() };
        valid_folder(&folder).then_some(folder)
    }

    /// Compares the branch tip read after the folder dialog with the tip that was shown.
    fn check_tip(&mut self, cx: &mut Ctx) {
        let Some((choice, mut task)) = self.tip_check.take() else { return };
        let Some(result) = task.get().cloned() else {
            self.tip_check = Some((choice, task));
            return;
        };
        match result {
            Ok(tip) if tip == choice.tip => {}
            Ok(_) => {
                self.choice = None;
                cx.notice(tr_fmt!("{name} moved after it was selected. Choose it again and review it.", name = choice.name), true);
            }
            Err(error) => {
                self.choice = None;
                cx.notice(tr_fmt!("Could not read {name}: {error}", name = choice.name, error = error), true);
            }
        }
    }
}

impl Default for WorktreesWindow {
    fn default() -> Self {
        Self::new()
    }
}

impl ToolWindow for WorktreesWindow {
    fn id(&self) -> String {
        "worktrees".to_string()
    }

    fn title(&self) -> String {
        tr("Worktrees").to_string()
    }

    fn default_size(&self) -> egui::Vec2 {
        egui::vec2(680.0, 600.0)
    }

    fn ui(&mut self, ui: &mut Ui, cx: &mut Ctx) {
        self.check_tip(cx);
        let c = theme::of(ui);
        ui.label(
            RichText::new(tr("A linked worktree checks out another branch in its own folder and shares this repository's history."))
                .color(c.muted),
        );
        if !cx.idle {
            ui.add_space(4.0);
            widgets::loading(ui, tr("An action is running. Controls return when it finishes."));
        }
        ui.add_space(6.0);
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.set_width(ui.available_width());
            self.worktree_list(ui, cx);
            ui.add_space(14.0);
            self.create_section(ui, cx);
        });
    }

    fn repository_changed(&mut self, snapshot: &Snapshot) {
        if let Some(path) = self.removing.take() {
            if !snapshot.worktrees.iter().any(|worktree| worktree.path == path) {
                self.confirming_removal = None;
            }
        }
    }
}

/// One worktree's card: its folder, branch, state badges, and actions.
fn worktree_card(
    ui: &mut Ui,
    idle: bool,
    snapshot: &Snapshot,
    is_main: bool,
    worktree: &Worktree,
    confirming: Option<&str>,
) -> Option<WorktreeAction> {
    let c = theme::of(ui);
    let is_current = worktree.path == snapshot.root_path;
    let name = std::path::Path::new(&worktree.path)
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| worktree.path.clone());
    let mut action = None;
    egui::Frame::new()
        .fill(c.card_bg)
        .stroke(egui::Stroke::new(1.0, c.border))
        .corner_radius(8.0)
        .inner_margin(egui::Margin::same(12))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(icon::FOLDER).color(c.muted));
                ui.label(RichText::new(name).strong());
                if is_main {
                    widgets::pill(ui, tr("Main"), c.local_branch);
                }
                if is_current {
                    widgets::pill(ui, tr("Open here"), c.accent);
                }
                if worktree.is_locked {
                    widgets::pill(ui, tr("Locked"), c.warning);
                }
                if worktree.is_prunable {
                    widgets::pill(ui, tr("Missing"), c.danger);
                }
            });
            ui.add(egui::Label::new(RichText::new(&worktree.path).monospace().small().color(c.muted)).wrap());
            let branch = match (&worktree.branch, worktree.is_bare) {
                (Some(branch), _) => format!("{}  {branch}", icon::GIT_BRANCH),
                (None, true) => tr("Bare repository").to_string(),
                (None, false) => tr("Detached HEAD").to_string(),
            };
            ui.label(RichText::new(branch).small());

            ui.add_space(6.0);
            let confirming_this = confirming == Some(worktree.path.as_str());
            ui.horizontal(|ui| {
                if ui.add_enabled(!is_current && !worktree.is_prunable, egui::Button::new(format!("{}  {}", icon::ARROW_SQUARE_OUT, tr("Open")))).clicked() {
                    action = Some(WorktreeAction::Open(worktree.path.clone()));
                }
                if ui.button(format!("{}  {}", icon::COPY, tr("Copy path"))).clicked() {
                    action = Some(WorktreeAction::Copy(worktree.path.clone()));
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let removable = !is_main && !is_current && !worktree.is_prunable && !worktree.is_locked;
                    let hint = if worktree.is_locked {
                        tr("A locked worktree must be unlocked in a terminal before it can be removed")
                    } else if is_main || is_current {
                        tr("The main worktree and the checkout open here cannot be removed")
                    } else {
                        tr("Remove this worktree and its folder")
                    };
                    let remove = ui.add_enabled(idle && removable, egui::Button::new(format!("{}  {}", icon::TRASH, tr("Remove"))));
                    if remove.on_hover_text(hint).on_disabled_hover_text(hint).clicked() {
                        action = Some(WorktreeAction::AskRemove(worktree.path.clone()));
                    }
                });
            });

            if confirming_this {
                ui.add_space(6.0);
                widgets::callout(
                    ui,
                    tr("Remove this worktree and its folder? Git refuses if the folder has uncommitted or untracked files. The branch is kept."),
                    true,
                );
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    if widgets::danger_button(ui, &format!("{}  {}", icon::TRASH, tr("Remove worktree")), idle).clicked() {
                        action = Some(WorktreeAction::ConfirmRemove(worktree.path.clone()));
                    }
                    if ui.button(tr("Cancel")).clicked() {
                        action = Some(WorktreeAction::CancelRemove);
                    }
                });
            }
        });
    action
}

/// A folder name derived from a branch, with characters that do not suit folder names replaced.
fn folder_for(branch: &str) -> String {
    branch
        .chars()
        .map(|character| if character.is_alphanumeric() || matches!(character, '-' | '_' | '.') { character } else { '-' })
        .collect()
}

fn valid_folder(name: &str) -> bool {
    !name.is_empty() && name != "." && name != ".." && !name.contains(['/', '\\'])
}
