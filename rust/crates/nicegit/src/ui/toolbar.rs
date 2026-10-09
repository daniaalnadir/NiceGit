use egui::{RichText, Ui};
use egui_phosphor::regular as icon;

use crate::app::NiceGitApp;
use crate::theme;
use crate::tools::{self};
use crate::ui::dialogs::{Dialog, InputKind};

/// A toolbar button: an icon above a short label.
fn tool(ui: &mut Ui, glyph: &str, label: &str, tooltip: &str, enabled: bool) -> egui::Response {
    let c = theme::of(ui);
    let size = egui::vec2(50.0, 44.0);
    let (rect, response) = ui.allocate_exact_size(size, if enabled { egui::Sense::click() } else { egui::Sense::hover() });
    let response = response.on_hover_text(tooltip);
    // Screen readers and UI tests find the button by its label.
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, label));
    if ui.is_rect_visible(rect) {
        if enabled && response.hovered() {
            ui.painter().rect_filled(rect, 6.0, ui.visuals().widgets.hovered.weak_bg_fill);
        }
        let color = if enabled { ui.visuals().text_color() } else { c.muted.gamma_multiply(0.6) };
        ui.painter().text(
            rect.center_top() + egui::vec2(0.0, 4.0),
            egui::Align2::CENTER_TOP,
            label,
            egui::FontId::proportional(11.0),
            color,
        );
        ui.painter().text(
            rect.center_bottom() - egui::vec2(0.0, 3.0),
            egui::Align2::CENTER_BOTTOM,
            glyph,
            egui::FontId::proportional(20.0),
            color,
        );
    }
    response
}

/// A labelled picker: a small caption above an accent-coloured value with a chevron.
fn picker(ui: &mut Ui, caption: &str, value: &str, add: impl FnOnce(&mut Ui)) {
    let c = theme::of(ui);
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 1.0;
        ui.label(RichText::new(caption).small().color(c.muted));
        ui.menu_button(RichText::new(format!("{value} {}", icon::CARET_DOWN)).color(c.accent).strong(), add);
    });
}

impl NiceGitApp {
    pub fn toolbar(&mut self, ui: &mut Ui) {
        let Some(snapshot) = self.snapshot().cloned() else { return };
        let idle = self.idle();
        let clean = snapshot.operation.is_none();
        egui::ScrollArea::horizontal().id_salt("toolbar").scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden).show(
            ui,
            |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    let name = self.repo().map(|r| r.name()).unwrap_or_default();
                    picker(ui, "Repository", &name, |ui| self.repository_menu(ui));
                    ui.add_space(8.0);
                    let branch = snapshot.current_branch.clone();
                    picker(ui, "Branch", &branch, |ui| self.branch_menu_list(ui));
                    ui.add_space(10.0);

                    let can_undo =
                        idle && self.repo().is_some_and(|r| r.undo.as_ref().is_some_and(|s| s.applies_to(&snapshot.current_branch)));
                    let undo_tip = self
                        .repo()
                        .and_then(|r| r.undo.as_ref())
                        .map(|s| format!("Undo {}", s.title.to_lowercase()))
                        .unwrap_or_else(|| "Nothing to undo".into());
                    if tool(ui, icon::ARROW_COUNTER_CLOCKWISE, "Undo", &undo_tip, can_undo).clicked() {
                        self.undo();
                    }
                    let can_redo =
                        idle && self.repo().is_some_and(|r| r.redo.as_ref().is_some_and(|s| s.applies_to(&snapshot.current_branch)));
                    let redo_tip = self
                        .repo()
                        .and_then(|r| r.redo.as_ref())
                        .map(|s| format!("Redo {}", s.title.to_lowercase()))
                        .unwrap_or_else(|| "Nothing to redo".into());
                    if tool(ui, icon::ARROW_CLOCKWISE, "Redo", &redo_tip, can_redo).clicked() {
                        self.redo();
                    }
                    ui.add_space(8.0);
                    let has_remote = !snapshot.remotes.is_empty();
                    if tool(ui, icon::CLOUD_ARROW_DOWN, "Fetch", "Download from all remotes", idle && has_remote).clicked() {
                        self.fetch();
                    }
                    let behind = snapshot.behind.unwrap_or(0);
                    let pull_label = if behind > 0 { format!("Pull {behind}") } else { "Pull".into() };
                    if tool(
                        ui,
                        icon::ARROW_LINE_DOWN,
                        &pull_label,
                        "Fetch and fast-forward the current branch",
                        idle && clean && snapshot.upstream.is_some(),
                    )
                    .clicked()
                    {
                        self.pull();
                    }
                    let ahead = snapshot.ahead.unwrap_or(0);
                    let push_label = if snapshot.upstream.is_none() {
                        "Publish".to_string()
                    } else if ahead > 0 {
                        format!("Push {ahead}")
                    } else {
                        "Push".into()
                    };
                    if tool(
                        ui,
                        icon::ARROW_LINE_UP,
                        &push_label,
                        "Send the current branch to its upstream",
                        idle && snapshot.is_on_branch() && has_remote,
                    )
                    .clicked()
                    {
                        self.push();
                    }
                    ui.add_space(8.0);
                    if tool(
                        ui,
                        icon::GIT_BRANCH,
                        "Branch",
                        "Create a branch at HEAD and switch to it",
                        idle && clean && snapshot.head_hash.is_some(),
                    )
                    .clicked()
                    {
                        self.dialog = Some(Dialog::input(
                            "New branch",
                            "Create a branch at HEAD and switch to it",
                            "",
                            InputKind::CreateBranch { branch: snapshot.current_branch.clone(), head: snapshot.head_hash.clone() },
                        ));
                    }
                    if tool(ui, icon::ARCHIVE, "Stash", "Save all changes in a stash", idle && clean && !snapshot.status.is_empty())
                        .clicked()
                    {
                        self.dialog = Some(Dialog::input("Stash changes", "Message (optional)", "", InputKind::SaveStash));
                    }
                    let latest = snapshot.stashes.first().cloned();
                    if tool(ui, icon::TRAY_ARROW_UP, "Pop", "Apply the latest stash and delete it", idle && clean && latest.is_some())
                        .clicked()
                    {
                        if let Some(stash) = latest {
                            self.act("Pop stash", move |client, path| {
                                client.pop_stash(&stash, path).map(|_| Some("Applied and deleted the stash.".into()))
                            });
                        }
                    }
                    ui.add_space(8.0);
                    if tool(ui, icon::TERMINAL_WINDOW, "Terminal", "Show the terminal (Ctrl-`)", true).clicked() {
                        self.toggle_terminal(ui.ctx());
                    }
                    if tool(ui, icon::ARROWS_CLOCKWISE, "Refresh", "Reload the repository (Ctrl/Cmd-R)", self.busy.is_none()).clicked() {
                        self.load(true);
                    }
                    ui.add_space(8.0);
                    if tool(ui, icon::COMMAND, "Commands", "Command palette (Shift-Ctrl/Cmd-P)", true).clicked() {
                        self.toggle_palette();
                    }
                })
            },
        );
    }

    /// The Repository menu: opening repositories and every repository tool.
    pub fn repository_menu(&mut self, ui: &mut Ui) {
        ui.set_min_width(240.0);
        let has_repo = self.snapshot().is_some();
        if ui.button(format!("{}  Open repository…", icon::FOLDER_OPEN)).clicked() {
            ui.close();
            self.choose_folder();
        }
        if ui.button(format!("{}  Clone repository…", icon::DOWNLOAD_SIMPLE)).clicked() {
            ui.close();
            self.dialog = Some(Dialog::clone_repository());
        }
        if ui.button(format!("{}  New repository…", icon::PLUS)).clicked() {
            ui.close();
            self.create_repository();
        }
        ui.separator();
        let open =
            |ui: &mut Ui, app: &mut NiceGitApp, glyph: &str, text: &str, make: &dyn Fn(&egui::Context) -> Box<dyn tools::ToolWindow>| {
                if ui.add_enabled(has_repo, egui::Button::new(format!("{glyph}  {text}"))).clicked() {
                    ui.close();
                    app.open_tool(make(ui.ctx()));
                }
            };
        open(ui, self, icon::GEAR_SIX, "Repository settings…", &|_| Box::new(tools::repository_settings::RepositorySettingsWindow::new()));
        open(ui, self, icon::MAGNIFYING_GLASS, "Search history…", &|_| Box::new(tools::commit_search::CommitSearchWindow::new()));
        open(ui, self, icon::FILE_MAGNIFYING_GLASS, "Search file contents…", &|_| {
            Box::new(tools::content_search::ContentSearchWindow::new(None))
        });
        open(ui, self, icon::CLOCK_COUNTER_CLOCKWISE, "Recover lost work…", &|_| Box::new(tools::reflog::ReflogWindow::new()));
        open(ui, self, icon::BROOM, "Clean up branches…", &|_| Box::new(tools::branch_cleanup::BranchCleanupWindow::new()));
        open(ui, self, icon::BUG, "Bisect…", &|_| Box::new(tools::bisect::BisectWindow::new()));
        ui.separator();
        open(ui, self, icon::FOLDERS, "Worktrees…", &|_| Box::new(tools::worktrees::WorktreesWindow::new()));
        open(ui, self, icon::PACKAGE, "Submodules…", &|_| Box::new(tools::submodules::SubmodulesWindow::new()));
        open(ui, self, icon::FLOW_ARROW, "GitFlow…", &|_| Box::new(tools::gitflow::GitFlowWindow::new()));
        open(ui, self, icon::DATABASE, "Git LFS…", &|_| Box::new(tools::lfs::LfsWindow::new()));
        let remote = self.snapshot().and_then(|s| s.remotes.iter().find(|r| *r == "origin").or(s.remotes.first()).cloned());
        if let Some(remote) = remote {
            open(ui, self, icon::GITHUB_LOGO, "Pull requests and issues…", &move |_| {
                Box::new(tools::github::GitHubWindow::new(remote.clone()))
            });
        }
        ui.separator();
        if ui.add_enabled(has_repo, egui::Button::new(format!("{}  Apply patch…", icon::FILE_PLUS))).clicked() {
            ui.close();
            self.apply_patch();
        }
    }

    pub fn apply_patch(&mut self) {
        let Some(snapshot) = self.snapshot().cloned() else { return };
        if let Some(file) = rfd::FileDialog::new().set_title("Apply a patch").add_filter("Patch", &["patch", "diff"]).pick_file() {
            let name = file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            self.confirm(
                format!("Apply {name}?"),
                format!(
                    "Git checks the patch, then applies it to the working files of {}. The changes stay unstaged.",
                    snapshot.current_branch
                ),
                "Apply",
                crate::ui::dialogs::Pending::ApplyPatch { file, branch: snapshot.current_branch.clone(), head: snapshot.head_hash.clone() },
            );
        }
    }

    /// The Branch menu: switch to another local branch, with a filter.
    fn branch_menu_list(&mut self, ui: &mut Ui) {
        let Some(snapshot) = self.snapshot().cloned() else { return };
        ui.set_min_width(260.0);
        let can_switch = self.idle() && snapshot.operation.is_none();
        let filter = self.repo().map(|r| r.branch_filter.to_lowercase()).unwrap_or_default();
        egui::ScrollArea::vertical().max_height(360.0).show(ui, |ui| {
            for branch in snapshot.branches.iter().filter(|b| !b.is_remote && !b.is_detached()) {
                if !filter.is_empty() && !branch.name.to_lowercase().contains(&filter) {
                    continue;
                }
                let text = if branch.is_current { format!("{}  {}", icon::CHECK, branch.name) } else { format!("      {}", branch.name) };
                if ui.add_enabled(can_switch && !branch.is_current, egui::Button::new(text).frame(false)).clicked() {
                    ui.close();
                    self.checkout(branch.clone());
                }
            }
        });
        ui.separator();
        if ui.add_enabled(self.idle() && snapshot.head_hash.is_some(), egui::Button::new(format!("{}  New branch…", icon::PLUS))).clicked()
        {
            ui.close();
            self.dialog = Some(Dialog::input(
                "New branch",
                "Create a branch at HEAD and switch to it",
                "",
                InputKind::CreateBranch { branch: snapshot.current_branch.clone(), head: snapshot.head_hash.clone() },
            ));
        }
    }
}
