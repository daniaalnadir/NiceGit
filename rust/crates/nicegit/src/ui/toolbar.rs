use egui::{RichText, Ui};
use egui_phosphor::regular as icon;

use crate::app::NiceGitApp;
use crate::theme;
use crate::tools::{self};
use crate::ui::dialogs::{Dialog, InputKind};

const FULL_WIDTH: f32 = 54.0;
const COMPACT_WIDTH: f32 = 34.0;

#[derive(Clone, Copy, PartialEq)]
enum ToolStyle {
    /// An icon above a short label.
    Full,
    /// The icon alone, with the label in its tooltip.
    Compact,
}

#[derive(Clone, Copy)]
enum ToolAction {
    Undo,
    Redo,
    Fetch,
    Pull,
    Push,
    Branch,
    Stash,
    Pop,
    Terminal,
    Refresh,
    Commands,
}

struct ToolItem {
    glyph: &'static str,
    label: String,
    tip: String,
    enabled: bool,
    action: ToolAction,
}

impl ToolItem {
    fn new(glyph: &'static str, label: &str, tip: String, enabled: bool, action: ToolAction) -> Self {
        Self { glyph, label: label.to_string(), tip, enabled, action }
    }
}

/// A toolbar button: an icon above a short label, or the icon alone when space is tight.
fn tool(ui: &mut Ui, glyph: &str, label: &str, tooltip: &str, enabled: bool, style: ToolStyle) -> egui::Response {
    let c = theme::of(ui);
    let size = egui::vec2(if style == ToolStyle::Full { FULL_WIDTH } else { COMPACT_WIDTH }, 44.0);
    let (rect, response) = ui.allocate_exact_size(size, if enabled { egui::Sense::click() } else { egui::Sense::hover() });
    let response =
        if style == ToolStyle::Compact { response.on_hover_text(format!("{label}: {tooltip}")) } else { response.on_hover_text(tooltip) };
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
        let has_remote = !snapshot.remotes.is_empty();
        let undo_tip = self
            .repo()
            .and_then(|r| r.undo.as_ref())
            .map(|s| format!("Undo {}", s.title.to_lowercase()))
            .unwrap_or_else(|| "Nothing to undo".into());
        let redo_tip = self
            .repo()
            .and_then(|r| r.redo.as_ref())
            .map(|s| format!("Redo {}", s.title.to_lowercase()))
            .unwrap_or_else(|| "Nothing to redo".into());
        let can_undo = idle && self.repo().is_some_and(|r| r.undo.as_ref().is_some_and(|s| s.applies_to(&snapshot.current_branch)));
        let can_redo = idle && self.repo().is_some_and(|r| r.redo.as_ref().is_some_and(|s| s.applies_to(&snapshot.current_branch)));
        let behind = snapshot.behind.unwrap_or(0);
        let ahead = snapshot.ahead.unwrap_or(0);
        let pull_label = if behind > 0 { format!("Pull {behind}") } else { "Pull".into() };
        let push_label = if snapshot.upstream.is_none() {
            "Publish".to_string()
        } else if ahead > 0 {
            format!("Push {ahead}")
        } else {
            "Push".into()
        };
        // Groups of buttons, in order; a gap separates groups.
        let groups: Vec<Vec<ToolItem>> = vec![
            vec![
                ToolItem::new(icon::ARROW_COUNTER_CLOCKWISE, "Undo", undo_tip.clone(), can_undo, ToolAction::Undo),
                ToolItem::new(icon::ARROW_CLOCKWISE, "Redo", redo_tip.clone(), can_redo, ToolAction::Redo),
            ],
            vec![
                ToolItem::new(icon::CLOUD_ARROW_DOWN, "Fetch", "Download from all remotes".into(), idle && has_remote, ToolAction::Fetch),
                ToolItem::new(
                    icon::ARROW_LINE_DOWN,
                    &pull_label,
                    "Fetch and fast-forward the current branch".into(),
                    idle && clean && snapshot.upstream.is_some(),
                    ToolAction::Pull,
                ),
                ToolItem::new(
                    icon::ARROW_LINE_UP,
                    &push_label,
                    "Send the current branch to its upstream".into(),
                    idle && snapshot.is_on_branch() && has_remote,
                    ToolAction::Push,
                ),
            ],
            vec![
                ToolItem::new(
                    icon::GIT_BRANCH,
                    "Branch",
                    "Create a branch at HEAD and switch to it".into(),
                    idle && clean && snapshot.head_hash.is_some(),
                    ToolAction::Branch,
                ),
                ToolItem::new(
                    icon::ARCHIVE,
                    "Stash",
                    "Save all changes in a stash".into(),
                    idle && clean && !snapshot.status.is_empty(),
                    ToolAction::Stash,
                ),
                ToolItem::new(
                    icon::TRAY_ARROW_UP,
                    "Pop",
                    "Apply the latest stash and delete it".into(),
                    idle && clean && !snapshot.stashes.is_empty(),
                    ToolAction::Pop,
                ),
            ],
            vec![
                ToolItem::new(icon::TERMINAL_WINDOW, "Terminal", "Show the terminal (Ctrl-`)".into(), true, ToolAction::Terminal),
                ToolItem::new(
                    icon::ARROWS_CLOCKWISE,
                    "Refresh",
                    "Reload the repository (Ctrl/Cmd-R)".into(),
                    self.busy.is_none(),
                    ToolAction::Refresh,
                ),
                ToolItem::new(icon::COMMAND, "Commands", "Command palette (Shift-Ctrl/Cmd-P)".into(), true, ToolAction::Commands),
            ],
        ];

        let mut chosen: Option<ToolAction> = None;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            let name = self.repo().map(|r| r.name()).unwrap_or_default();
            picker(ui, "Repository", &name, |ui| self.repository_menu(ui));
            ui.add_space(8.0);
            picker(ui, "Branch", &snapshot.current_branch, |ui| self.branch_menu_list(ui));
            ui.add_space(10.0);

            // Labelled buttons when they all fit, icons alone when not, and a More menu for
            // whatever still does not fit, so the toolbar stays on one row.
            let available = ui.available_width();
            let count: usize = groups.iter().map(Vec::len).sum();
            let width_for = |button: f32, shown: usize| -> f32 {
                let mut total = 0.0;
                let mut seen = 0;
                for group in &groups {
                    if seen >= shown {
                        break;
                    }
                    total += 8.0;
                    for _ in group {
                        if seen >= shown {
                            break;
                        }
                        total += button + 2.0;
                        seen += 1;
                    }
                }
                total
            };
            let (style, shown) = if width_for(FULL_WIDTH, count) <= available {
                (ToolStyle::Full, count)
            } else if width_for(COMPACT_WIDTH, count) <= available {
                (ToolStyle::Compact, count)
            } else {
                let room = available - COMPACT_WIDTH - 10.0;
                let fits = (0..=count).rev().find(|&n| width_for(COMPACT_WIDTH, n) <= room).unwrap_or(0);
                (ToolStyle::Compact, fits)
            };
            let mut index = 0;
            let mut overflow: Vec<&ToolItem> = Vec::new();
            for group in &groups {
                if index < shown {
                    ui.add_space(6.0);
                }
                for item in group {
                    if index < shown {
                        if tool(ui, item.glyph, &item.label, &item.tip, item.enabled, style).clicked() {
                            chosen = Some(item.action);
                        }
                    } else {
                        overflow.push(item);
                    }
                    index += 1;
                }
            }
            if !overflow.is_empty() {
                ui.add_space(6.0);
                let more = ui.menu_button(RichText::new(format!("{}", icon::DOTS_THREE)).size(20.0), |ui| {
                    for item in &overflow {
                        if ui
                            .add_enabled(item.enabled, egui::Button::new(format!("{}  {}", item.glyph, item.label)))
                            .on_hover_text(&item.tip)
                            .clicked()
                        {
                            ui.close();
                            chosen = Some(item.action);
                        }
                    }
                });
                more.response
                    .on_hover_text("More actions")
                    .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "More"));
            }
        });

        match chosen {
            Some(ToolAction::Undo) => self.confirm(
                format!("{undo_tip}?"),
                "NiceGit moves things back to where they were before. It refuses if the branch has moved since, so newer work is never lost.",
                "Undo",
                crate::ui::dialogs::Pending::Undo,
            ),
            Some(ToolAction::Redo) => self.confirm(
                format!("{redo_tip}?"),
                "NiceGit puts back what the undo reversed. It refuses if the branch has moved since.",
                "Redo",
                crate::ui::dialogs::Pending::Redo,
            ),
            Some(ToolAction::Fetch) => self.fetch(),
            Some(ToolAction::Pull) => self.pull(),
            Some(ToolAction::Push) => self.push(),
            Some(ToolAction::Branch) => {
                self.dialog = Some(Dialog::input(
                    "New branch",
                    "Create a branch at HEAD and switch to it",
                    "",
                    InputKind::CreateBranch { branch: snapshot.current_branch.clone(), head: snapshot.head_hash.clone() },
                ))
            }
            Some(ToolAction::Stash) => self.dialog = Some(Dialog::input("Stash changes", "Message (optional)", "", InputKind::SaveStash)),
            Some(ToolAction::Pop) => {
                if let Some(stash) = snapshot.stashes.first().cloned() {
                    self.act("Pop stash", move |client, path| client.pop_stash(&stash, path).map(|_| Some("Applied and deleted the stash.".into())));
                }
            }
            Some(ToolAction::Terminal) => self.toggle_terminal(ui.ctx()),
            Some(ToolAction::Refresh) => self.load(true),
            Some(ToolAction::Commands) => self.toggle_palette(),
            None => {}
        }
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
        open(ui, self, icon::ARCHIVE, "Stashes…", &|_| Box::new(tools::stash::StashWindow::new()));
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
