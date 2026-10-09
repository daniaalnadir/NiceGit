use egui::{RichText, Ui};
use egui_phosphor::regular as icon;

use crate::app::NiceGitApp;
use crate::theme;
use crate::tools::{self};
use crate::ui::dialogs::{Dialog, InputKind};

/// Space either side of a labelled button's label.
const LABEL_PADDING: f32 = 7.0;
const COMPACT_WIDTH: f32 = 34.0;
/// The room between two groups of buttons: space, a thin rule, and space.
const GROUP_GAP: f32 = 15.0;

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
    let size = egui::vec2(tool_width(ui, label, style), 44.0);
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
        if style == ToolStyle::Full {
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
        } else {
            // A compact cell is narrower than most labels, so it shows the icon alone; the label
            // is in the tooltip and the accessible name.
            ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, glyph, egui::FontId::proportional(20.0), color);
        }
    }
    response
}

/// How wide a toolbar button is: as wide as its label needs, or a compact icon cell.
fn tool_width(ui: &Ui, label: &str, style: ToolStyle) -> f32 {
    match style {
        ToolStyle::Full => {
            let text =
                ui.ctx().fonts_mut(|fonts| fonts.layout_no_wrap(label.to_string(), egui::FontId::proportional(11.0), egui::Color32::WHITE));
            (text.size().x + 2.0 * LABEL_PADDING).max(COMPACT_WIDTH)
        }
        ToolStyle::Compact => COMPACT_WIDTH,
    }
}

/// A thin vertical rule between two groups of toolbar buttons.
fn group_rule(ui: &mut Ui) {
    let c = theme::of(ui);
    let (rect, _) = ui.allocate_exact_size(egui::vec2(1.0, 44.0), egui::Sense::hover());
    // Stronger than a panel border, so the groups read apart at a glance in either theme.
    let color = if ui.visuals().dark_mode { c.muted.gamma_multiply(0.7) } else { c.muted };
    ui.painter().vline(rect.center().x, rect.shrink2(egui::vec2(0.0, 6.0)).y_range(), egui::Stroke::new(1.0, color));
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
            let width_for = |style: ToolStyle, shown: usize| -> f32 {
                let mut total = 0.0;
                let mut seen = 0;
                for group in &groups {
                    if seen >= shown {
                        break;
                    }
                    if seen > 0 {
                        total += GROUP_GAP;
                    }
                    for item in group {
                        if seen >= shown {
                            break;
                        }
                        total += tool_width(ui, &item.label, style) + 2.0;
                        seen += 1;
                    }
                }
                total
            };
            let (style, shown) = if width_for(ToolStyle::Full, count) <= available {
                (ToolStyle::Full, count)
            } else if width_for(ToolStyle::Compact, count) <= available {
                (ToolStyle::Compact, count)
            } else {
                // Room for the More menu and the rule before it.
                let room = available - COMPACT_WIDTH - GROUP_GAP;
                let fits = (0..=count).rev().find(|&n| width_for(ToolStyle::Compact, n) <= room).unwrap_or(0);
                (ToolStyle::Compact, fits)
            };
            let mut index = 0;
            let mut overflow: Vec<&ToolItem> = Vec::new();
            for group in &groups {
                if index > 0 && index < shown {
                    ui.add_space((GROUP_GAP - 1.0) / 2.0);
                    group_rule(ui);
                    ui.add_space((GROUP_GAP - 1.0) / 2.0);
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
                if shown > 0 {
                    ui.add_space((GROUP_GAP - 1.0) / 2.0);
                    group_rule(ui);
                    ui.add_space((GROUP_GAP - 1.0) / 2.0);
                }
                // Drawn without a frame at rest, like the other toolbar buttons.
                let visuals = &mut ui.visuals_mut().widgets.inactive;
                visuals.weak_bg_fill = egui::Color32::TRANSPARENT;
                visuals.bg_stroke = egui::Stroke::NONE;
                let more = ui.menu_button(RichText::new(icon::DOTS_THREE).size(20.0), |ui| {
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
            Some(ToolAction::Undo) => self.confirm_undo(),
            Some(ToolAction::Redo) => self.confirm_redo(),
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
                    self.confirm(
                        format!("Pop {}?", stash.reference),
                        format!(
                            "“{}” is applied to your working files and then deleted. If applying fails, the stash is kept.",
                            stash.message
                        ),
                        "Pop",
                        crate::ui::dialogs::Pending::PopStash(stash),
                    );
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
        let can_publish = self.snapshot().is_some_and(|s| s.upstream.is_none() && s.is_on_branch() && !s.remotes.is_empty()) && self.idle();
        if ui
            .add_enabled(can_publish, egui::Button::new(format!("{}  Publish branch…", icon::UPLOAD_SIMPLE)))
            .on_disabled_hover_text("The branch already has an upstream, or there is no remote")
            .clicked()
        {
            ui.close();
            self.push();
        }
        if ui.add_enabled(has_repo, egui::Button::new(format!("{}  Apply patch…", icon::FILE_PLUS))).clicked() {
            ui.close();
            self.apply_patch();
        }
    }

    /// Asks before undoing the last recorded step, from the toolbar or the command palette.
    pub fn confirm_undo(&mut self) {
        let Some(title) = self.repo().and_then(|r| r.undo.as_ref()).map(|s| s.title.to_lowercase()) else { return };
        self.confirm(
            format!("Undo {title}?"),
            "NiceGit moves things back to where they were before. It refuses if the branch has moved since, so newer work is never lost.",
            "Undo",
            crate::ui::dialogs::Pending::Undo,
        );
    }

    /// Asks before redoing the last undone step, from the toolbar or the command palette.
    pub fn confirm_redo(&mut self) {
        let Some(title) = self.repo().and_then(|r| r.redo.as_ref()).map(|s| s.title.to_lowercase()) else { return };
        self.confirm(
            format!("Redo {title}?"),
            "NiceGit puts back what the undo reversed. It refuses if the branch has moved since.",
            "Redo",
            crate::ui::dialogs::Pending::Redo,
        );
    }

    pub fn apply_patch(&mut self) {
        let Some(snapshot) = self.snapshot().cloned() else { return };
        if let Some(file) = crate::file_dialog::pick_file("Apply a patch", "Patch", &["patch", "diff"]) {
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
