use egui::{RichText, Ui};
use egui_phosphor::regular as icon;
use nicegit_core::settings::IdentityProfile;
use nicegit_core::PullStrategy;

use crate::app::{HistoryEntry, NiceGitApp};
use crate::i18n::{tr, tr_count, tr_fmt};
use crate::settings::SyncAction;
use crate::theme;
use crate::tools::repository_settings::{StoredProfile, PROFILES_KEY};
use crate::tools::{self};
use crate::ui::dialogs::{Dialog, InputKind};

/// Space either side of a labelled button's label.
const LABEL_PADDING: f32 = 7.0;
const COMPACT_WIDTH: f32 = 34.0;
/// The width of the arrow beside a button that has a menu of choices.
const CARET_WIDTH: f32 = 16.0;
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
    /// A pull run once from the Fetch and Pull menu, whatever the setting says.
    PullWith(PullStrategy),
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
    /// Whether an arrow beside the button opens a menu, for the Fetch and Pull choice.
    menu: bool,
    /// A count drawn on the button's corner, such as commits waiting to be pulled.
    badge: Option<usize>,
    /// The steps the arrow beside the button lists, newest first, for Undo.
    history: Option<Vec<String>>,
}

impl ToolItem {
    fn new(glyph: &'static str, label: &str, tip: String, enabled: bool, action: ToolAction) -> Self {
        Self { glyph, label: label.to_string(), tip, enabled, action, menu: false, badge: None, history: None }
    }

    fn with_menu(mut self) -> Self {
        self.menu = true;
        self
    }

    fn with_badge(mut self, count: usize) -> Self {
        self.badge = (count > 0).then_some(count);
        self
    }

    fn with_history(mut self, titles: Vec<String>) -> Self {
        self.history = Some(titles);
        self
    }

    /// The room the button takes in `style`, including its arrow.
    fn width(&self, ui: &Ui, style: ToolStyle) -> f32 {
        let caret = self.menu || self.history.is_some();
        tool_width(ui, &self.label, style) + 2.0 + if caret { CARET_WIDTH + 2.0 } else { 0.0 }
    }
}

/// The most characters of a step's line in the Undo list before it is cut short.
const HISTORY_LINE_CHARS: usize = 60;

/// A step's name in Undo and Redo wording, such as `commit “Add a fourth line”`.
pub fn step_name(step: &HistoryEntry) -> String {
    match &step.detail {
        Some(detail) => format!("{} “{detail}”", step.title.to_lowercase()),
        None => step.title.to_lowercase(),
    }
}

/// A step's line in the Undo list, such as `Commit: Add a fourth line`, ending in an ellipsis when long.
fn history_line(step: &HistoryEntry) -> String {
    let line = match &step.detail {
        Some(detail) => format!("{}: {detail}", step.title),
        None => step.title.clone(),
    };
    if line.chars().count() <= HISTORY_LINE_CHARS {
        return line;
    }
    let mut cut: String = line.chars().take(HISTORY_LINE_CHARS - 1).collect();
    cut.push('…');
    cut
}

/// The hover text of Undo or Redo: the newest step's name, and how many steps there are.
fn history_tip(redo: bool, stack: &[HistoryEntry]) -> String {
    let (verb, nothing) = if redo { (tr("Redo"), tr("Nothing to redo")) } else { (tr("Undo"), tr("Nothing to undo")) };
    match stack.last() {
        None => nothing.to_string(),
        Some(step) if stack.len() == 1 => tr_fmt!("{verb} {step}", verb = verb, step = step_name(step)),
        Some(step) => tr_fmt!("{verb} {step} ({count} steps)", verb = verb, step = step_name(step), count = stack.len()),
    }
}

/// The steps Undo will step back through, newest first, listed without any way to act on them.
fn history_list(ui: &mut Ui, titles: &[String]) {
    const SHOWN: usize = 10;
    let c = theme::of(ui);
    ui.set_min_width(220.0);
    if titles.is_empty() {
        ui.label(RichText::new(tr("Nothing to undo")).small().color(c.muted));
        return;
    }
    ui.label(RichText::new(tr("Undo steps, newest first")).small().color(c.muted));
    ui.add_space(4.0);
    for title in titles.iter().take(SHOWN) {
        ui.label(title);
    }
    if titles.len() > SHOWN {
        ui.label(RichText::new(tr_fmt!("and {count} more", count = titles.len() - SHOWN)).small().color(c.muted));
    }
}

/// A small count on a toolbar button's icon, at its upper right.
fn badge(ui: &Ui, rect: egui::Rect, count: usize, style: ToolStyle) {
    let c = theme::of(ui);
    let text = if count > 99 { "99+".to_string() } else { count.to_string() };
    let galley = ui.painter().layout_no_wrap(text, egui::FontId::proportional(9.5), c.accent_text);
    // Over the icon: lower in a labelled button, where the label sits above it.
    let icon_top = if style == ToolStyle::Full { rect.bottom() - 26.0 } else { rect.center().y - 12.0 };
    let size = egui::vec2((galley.size().x + 7.0).max(15.0), 15.0);
    let badge = egui::Rect::from_min_size(egui::pos2(rect.center().x + 3.0, icon_top - 3.0), size);
    ui.painter().rect_filled(badge, 7.5, c.accent);
    ui.painter().galley(badge.center() - galley.size() / 2.0, galley, c.accent_text);
}

/// The arrow beside a button that opens its menu of choices.
fn menu_caret(ui: &mut Ui, label: &str, hover: &str) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(CARET_WIDTH, 44.0), egui::Sense::click());
    let response = response.on_hover_text(hover);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, tr_fmt!("{label} options", label = label)));
    if ui.is_rect_visible(rect) {
        if response.hovered() {
            ui.painter().rect_filled(rect, 6.0, ui.visuals().widgets.hovered.weak_bg_fill);
        }
        let color = theme::of(ui).muted;
        ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, icon::CARET_DOWN, egui::FontId::proportional(12.0), color);
    }
    response
}

/// A toolbar button: an icon above a short label, or the icon alone when space is tight.
fn tool(ui: &mut Ui, glyph: &str, label: &str, tooltip: &str, enabled: bool, style: ToolStyle) -> egui::Response {
    let c = theme::of(ui);
    let size = egui::vec2(tool_width(ui, label, style), 44.0);
    let (rect, response) = ui.allocate_exact_size(size, if enabled { egui::Sense::click() } else { egui::Sense::hover() });
    let response = if style == ToolStyle::Compact {
        response.on_hover_text(tr_fmt!("{label}: {tooltip}", label = label, tooltip = tooltip))
    } else {
        response.on_hover_text(tooltip)
    };
    // Screen readers and UI tests find the button by its label.
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, enabled, label));
    if ui.is_rect_visible(rect) {
        if enabled && response.hovered() {
            ui.painter().rect_filled(rect, 6.0, ui.visuals().widgets.hovered.weak_bg_fill);
        }
        // Disabled buttons use the muted token at full strength: it is clearly dimmer than enabled
        // text in both themes and keeps about 3:1 contrast on the toolbar background. Scaling its
        // alpha instead washed it out to near-invisible on light backgrounds.
        let color = if enabled { ui.visuals().text_color() } else { c.muted };
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

/// The choice of what the Fetch and Pull button does, and the pulls that run once. Returns the
/// strategy of a pull chosen from the menu; `pull_ready` says whether a pull can run now.
fn sync_menu(ui: &mut Ui, action: &mut SyncAction, pull_ready: bool) -> Option<PullStrategy> {
    let c = theme::of(ui);
    ui.set_min_width(250.0);
    ui.label(RichText::new(tr("Toolbar button")).small().color(c.muted));
    ui.add_space(4.0);
    // Each description lines up under its choice's name.
    let indent = ui.spacing().icon_width + ui.spacing().icon_spacing;
    for (choice, name, detail) in [
        (SyncAction::Fetch, tr("Fetch"), tr("Download from all remotes")),
        (SyncAction::Pull, tr("Pull"), tr("Fetch, then pull the current branch as Settings chooses")),
    ] {
        ui.scope(|ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            if tools::widgets::radio(ui, *action == choice, name).clicked() {
                *action = choice;
                ui.close();
            }
            ui.horizontal(|ui| {
                ui.add_space(indent);
                ui.label(RichText::new(detail).small().color(c.muted));
            });
        });
        ui.add_space(6.0);
    }
    ui.separator();
    let mut chosen = None;
    for (strategy, name, detail) in [
        (PullStrategy::Merge, tr("Pull (merge)"), tr("Fetch, then merge the upstream's commits into the current branch")),
        (PullStrategy::Rebase, tr("Pull (rebase)"), tr("Fetch, then replay the current branch's commits onto the upstream")),
    ] {
        if ui.add_enabled(pull_ready, egui::Button::new(name)).on_hover_text(detail).clicked() {
            chosen = Some(strategy);
            ui.close();
        }
    }
    chosen
}

impl NiceGitApp {
    pub fn toolbar(&mut self, ui: &mut Ui) {
        let Some(snapshot) = self.snapshot().cloned() else { return };
        let idle = self.idle();
        let clean = snapshot.operation.is_none();
        let has_remote = !snapshot.remotes.is_empty();
        let undo_steps = self.repo().map(|r| r.undo.as_slice()).unwrap_or_default();
        let redo_steps = self.repo().map(|r| r.redo.as_slice()).unwrap_or_default();
        let undo_tip = history_tip(false, undo_steps);
        let redo_tip = history_tip(true, redo_steps);
        let undo_lines: Vec<String> = undo_steps.iter().rev().map(history_line).collect();
        let can_undo = idle && undo_steps.last().is_some_and(|s| s.applies_to(&snapshot.current_branch));
        let can_redo = idle && redo_steps.last().is_some_and(|s| s.applies_to(&snapshot.current_branch));
        let mut sync_action = self.settings.sync_action;
        let pull_ready = idle && clean && snapshot.upstream.is_some();
        let behind = snapshot.behind.unwrap_or(0);
        let ahead = snapshot.ahead.unwrap_or(0);
        let pull_label = if behind > 0 { tr_fmt!("Pull {count}", count = behind) } else { tr("Pull").into() };
        let push_label = if snapshot.upstream.is_none() {
            tr("Publish").to_string()
        } else if ahead > 0 {
            tr_fmt!("Push {count}", count = ahead)
        } else {
            tr("Push").into()
        };
        // Groups of buttons, in order; a gap separates groups.
        let groups: Vec<Vec<ToolItem>> = vec![
            vec![
                ToolItem::new(icon::ARROW_COUNTER_CLOCKWISE, tr("Undo"), undo_tip.clone(), can_undo, ToolAction::Undo)
                    .with_history(undo_lines),
                ToolItem::new(icon::ARROW_CLOCKWISE, tr("Redo"), redo_tip.clone(), can_redo, ToolAction::Redo),
            ],
            vec![
                // Fetch and Pull share one button; its arrow chooses which one it does.
                match sync_action {
                    // Fetching leaves the branch where it is, so commits waiting to be pulled are
                    // counted on the button.
                    SyncAction::Fetch => ToolItem::new(
                        icon::CLOUD_ARROW_DOWN,
                        tr("Fetch"),
                        if behind > 0 {
                            tr_count(
                                "Download from all remotes · {count} commit to pull",
                                "Download from all remotes · {count} commits to pull",
                                behind,
                            )
                        } else {
                            tr("Download from all remotes").into()
                        },
                        idle && has_remote,
                        ToolAction::Fetch,
                    )
                    .with_badge(behind),
                    SyncAction::Pull => ToolItem::new(
                        icon::ARROW_LINE_DOWN,
                        &pull_label,
                        tr("Fetch, then pull the current branch as Settings chooses").into(),
                        pull_ready,
                        ToolAction::Pull,
                    ),
                }
                .with_menu(),
                ToolItem::new(
                    icon::ARROW_LINE_UP,
                    &push_label,
                    tr("Send the current branch to its upstream").into(),
                    idle && snapshot.is_on_branch() && has_remote,
                    ToolAction::Push,
                ),
            ],
            vec![
                ToolItem::new(
                    icon::GIT_BRANCH,
                    tr("Branch"),
                    tr("Create a branch at HEAD and switch to it").into(),
                    idle && clean && snapshot.head_hash.is_some(),
                    ToolAction::Branch,
                ),
                ToolItem::new(
                    icon::ARCHIVE,
                    tr("Stash"),
                    tr("Save all changes in a stash").into(),
                    idle && clean && !snapshot.status.is_empty(),
                    ToolAction::Stash,
                ),
                ToolItem::new(
                    icon::TRAY_ARROW_UP,
                    tr("Pop"),
                    tr("Apply the latest stash and delete it").into(),
                    idle && clean && !snapshot.stashes.is_empty(),
                    ToolAction::Pop,
                ),
            ],
            vec![
                ToolItem::new(icon::TERMINAL_WINDOW, tr("Terminal"), tr("Show the terminal (Ctrl-`)").into(), true, ToolAction::Terminal),
                ToolItem::new(
                    icon::ARROWS_CLOCKWISE,
                    tr("Refresh"),
                    tr("Reload the repository (Ctrl/Cmd-R)").into(),
                    self.busy.is_none(),
                    ToolAction::Refresh,
                ),
                ToolItem::new(icon::COMMAND, tr("Commands"), tr("Command palette (Shift-Ctrl/Cmd-P)").into(), true, ToolAction::Commands),
            ],
        ];

        let mut chosen: Option<ToolAction> = None;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            let name = self.repo().map(|r| r.name()).unwrap_or_default();
            picker(ui, tr("Repository"), &name, |ui| self.repository_menu(ui));
            ui.add_space(8.0);
            picker(ui, tr("Branch"), &snapshot.current_branch, |ui| self.branch_menu_list(ui));
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
                        total += item.width(ui, style);
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
                        let button = tool(ui, item.glyph, &item.label, &item.tip, item.enabled, style);
                        if let Some(count) = item.badge {
                            badge(ui, button.rect, count, style);
                        }
                        if button.clicked() {
                            chosen = Some(item.action);
                        }
                        if item.menu {
                            let caret = menu_caret(ui, tr("Fetch and Pull"), tr("Choose what Fetch and Pull does"));
                            egui::Popup::menu(&caret).show(|ui| {
                                if let Some(strategy) = sync_menu(ui, &mut sync_action, pull_ready) {
                                    chosen = Some(ToolAction::PullWith(strategy));
                                }
                            });
                        }
                        // Only when there is something for Undo to step back through.
                        if let Some(titles) = item.history.as_ref().filter(|titles| !titles.is_empty()) {
                            let caret = menu_caret(ui, tr("Undo history"), tr("Show the steps Undo will step back through"));
                            egui::Popup::menu(&caret).show(|ui| history_list(ui, titles));
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
                    .on_hover_text(tr("More actions"))
                    .widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, tr("More")));
            }
        });

        self.settings.sync_action = sync_action;
        match chosen {
            Some(ToolAction::Undo) => self.confirm_undo(),
            Some(ToolAction::Redo) => self.confirm_redo(),
            Some(ToolAction::Fetch) => self.fetch(),
            Some(ToolAction::Pull) => self.pull(),
            Some(ToolAction::PullWith(strategy)) => self.pull_using(Some(strategy)),
            Some(ToolAction::Push) => self.push(),
            Some(ToolAction::Branch) => {
                self.dialog = Some(Dialog::input(
                    tr("New branch"),
                    tr("Create a branch at HEAD and switch to it"),
                    "",
                    InputKind::CreateBranch { branch: snapshot.current_branch.clone(), head: snapshot.head_hash.clone() },
                ))
            }
            Some(ToolAction::Stash) => {
                self.dialog = Some(Dialog::input(tr("Stash changes"), tr("Message (optional)"), "", InputKind::SaveStash))
            }
            Some(ToolAction::Pop) => {
                if let Some(stash) = snapshot.stashes.first().cloned() {
                    self.confirm(
                        tr_fmt!("Pop {stash}?", stash = stash.reference),
                        tr_fmt!(
                            "“{message}” is applied to your working files and then deleted. If applying fails, the stash is kept.",
                            message = stash.message
                        ),
                        tr("Pop"),
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
        if ui.button(format!("{}  {}", icon::FOLDER_OPEN, tr("Open repository…"))).clicked() {
            ui.close();
            self.choose_folder();
        }
        if ui.button(format!("{}  {}", icon::MAGNIFYING_GLASS, tr("Find repositories in a folder…"))).clicked() {
            ui.close();
            let ctx = ui.ctx().clone();
            self.choose_folder_to_find(&ctx);
        }
        if ui.button(format!("{}  {}", icon::DOWNLOAD_SIMPLE, tr("Clone repository…"))).clicked() {
            ui.close();
            self.dialog = Some(Dialog::clone_repository());
        }
        if ui.button(format!("{}  {}", icon::PLUS, tr("New repository…"))).clicked() {
            ui.close();
            self.create_repository();
        }
        // Recent repositories, reachable here even while the repositories column is hidden.
        let tabs: Vec<_> = self.repos.iter().map(|repo| repo.path.clone()).collect();
        let recent: Vec<_> = self.settings.recent.iter().filter(|path| !tabs.contains(path) && path.is_dir()).take(12).cloned().collect();
        ui.add_enabled_ui(!recent.is_empty(), |ui| {
            ui.menu_button(format!("{}  {}", icon::CLOCK_COUNTER_CLOCKWISE, tr("Recent repositories")), |ui| {
                ui.set_min_width(220.0);
                for path in recent {
                    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    if ui.button(format!("{}  {name}", icon::FOLDER)).on_hover_text(path.display().to_string()).clicked() {
                        ui.close();
                        self.open(path);
                    }
                }
                ui.separator();
                if ui.button(format!("{}  {}", icon::SIDEBAR_SIMPLE, tr("Show all in the sidebar"))).clicked() {
                    ui.close();
                    self.settings.show_repositories = true;
                    self.settings.recent_repositories_open = true;
                    self.settings.forget_missing_recent();
                }
            })
            .response
            .on_disabled_hover_text(tr("Repositories you open appear here"));
        });
        ui.separator();
        let open =
            |ui: &mut Ui, app: &mut NiceGitApp, glyph: &str, text: &str, make: &dyn Fn(&egui::Context) -> Box<dyn tools::ToolWindow>| {
                if ui.add_enabled(has_repo, egui::Button::new(format!("{glyph}  {text}"))).clicked() {
                    ui.close();
                    app.open_tool(make(ui.ctx()));
                }
            };
        if has_repo {
            self.identity_menu(ui);
            ui.separator();
        }
        open(ui, self, icon::GEAR_SIX, tr("Repository settings…"), &|_| {
            Box::new(tools::repository_settings::RepositorySettingsWindow::new())
        });
        open(ui, self, icon::SLIDERS_HORIZONTAL, tr("Git settings…"), &|_| Box::new(tools::git_settings::GitSettingsWindow::new()));
        open(ui, self, icon::ARCHIVE, tr("Stashes…"), &|_| Box::new(tools::stash::StashWindow::new()));
        open(ui, self, icon::MAGNIFYING_GLASS, tr("Search history…"), &|_| Box::new(tools::commit_search::CommitSearchWindow::new()));
        open(ui, self, icon::FILE_MAGNIFYING_GLASS, tr("Search file contents…"), &|_| {
            Box::new(tools::content_search::ContentSearchWindow::new(None))
        });
        open(ui, self, icon::CLOCK_COUNTER_CLOCKWISE, tr("Recover lost work…"), &|_| Box::new(tools::reflog::ReflogWindow::new()));
        open(ui, self, icon::CHART_BAR, tr("Insights…"), &|_| Box::new(tools::insights::InsightsWindow::new()));
        open(ui, self, icon::BROOM, tr("Clean up branches…"), &|_| Box::new(tools::branch_cleanup::BranchCleanupWindow::new()));
        open(ui, self, icon::BUG, tr("Bisect…"), &|_| Box::new(tools::bisect::BisectWindow::new()));
        ui.separator();
        open(ui, self, icon::FOLDERS, tr("Worktrees…"), &|_| Box::new(tools::worktrees::WorktreesWindow::new()));
        open(ui, self, icon::PACKAGE, tr("Submodules…"), &|_| Box::new(tools::submodules::SubmodulesWindow::new()));
        open(ui, self, icon::FLOW_ARROW, "GitFlow…", &|_| Box::new(tools::gitflow::GitFlowWindow::new()));
        open(ui, self, icon::DATABASE, "Git LFS…", &|_| Box::new(tools::lfs::LfsWindow::new()));
        let remote = self.snapshot().and_then(|s| s.remotes.iter().find(|r| *r == "origin").or(s.remotes.first()).cloned());
        if let Some(remote) = remote {
            let hosts = self.settings.github_enterprise_hosts.clone();
            open(ui, self, icon::GITHUB_LOGO, tr("Pull requests and issues…"), &move |_| {
                Box::new(tools::github::GitHubWindow::new(remote.clone(), hosts.clone()))
            });
        }
        if ui.add_enabled(has_repo, egui::Button::new(format!("{}  Launchpad…", icon::GIT_PULL_REQUEST))).clicked() {
            ui.close();
            let window = tools::launchpad::LaunchpadWindow::for_app(self);
            self.open_tool(Box::new(window));
        }
        ui.separator();
        let values = self.repository_values();
        self.custom_actions_submenu(ui, crate::settings::ActionScope::Repository, &values, has_repo && self.idle());
        if ui.button(format!("{}  {}", icon::PENCIL_SIMPLE, tr("Manage custom actions…"))).clicked() {
            ui.close();
            self.show_settings = true;
        }
        ui.separator();
        let can_publish = self.snapshot().is_some_and(|s| s.upstream.is_none() && s.is_on_branch() && !s.remotes.is_empty()) && self.idle();
        if ui
            .add_enabled(can_publish, egui::Button::new(format!("{}  {}", icon::UPLOAD_SIMPLE, tr("Publish branch…"))))
            .on_disabled_hover_text(tr("The branch already has an upstream, or there is no remote"))
            .clicked()
        {
            ui.close();
            self.push();
        }
        // Only a shallow or sparse checkout offers these, and each acts on the repository shown now.
        let shape = self.repo().and_then(|repo| repo.shape).unwrap_or_default();
        let idle = self.idle();
        if shape.shallow
            && ui
                .add_enabled(has_repo && idle, egui::Button::new(format!("{}  {}", icon::DOWNLOAD_SIMPLE, tr("Fetch full history"))))
                .on_disabled_hover_text(tr("Wait for the current action to finish"))
                .clicked()
        {
            ui.close();
            self.act(tr("Fetch full history"), |client, path| {
                client.fetch_full_history(path).map(|_| Some(tr("Fetched the full history.").into()))
            });
        }
        if shape.sparse
            && ui
                .add_enabled(has_repo && idle, egui::Button::new(format!("{}  {}", icon::FOLDERS, tr("Show all folders…"))))
                .on_disabled_hover_text(tr("Wait for the current action to finish"))
                .clicked()
        {
            ui.close();
            self.confirm(
                tr("Show all folders?"),
                tr("Git checks out every folder in this repository, so every file is written to your folder. In a partial clone, the files are downloaded first, which can take a while for a large project. Your commits and branches do not change."),
                tr("Show all folders"),
                crate::ui::dialogs::Pending::ShowAllFolders,
            );
        }
        if ui.add_enabled(has_repo, egui::Button::new(format!("{}  {}", icon::FILE_PLUS, tr("Apply patch…")))).clicked() {
            ui.close();
            self.apply_patch();
        }
    }

    /// The Identity submenu: the saved profiles from Repository settings, with a check mark on the one
    /// the repository's commits already use, then Manage identities, which opens those profiles. A
    /// profile is applied the same way as its Apply button in Repository settings.
    fn identity_menu(&mut self, ui: &mut Ui) {
        let current = self.repo().and_then(|repo| repo.identity.clone());
        let idle = self.idle();
        let profiles: Vec<StoredProfile> = ui.ctx().data_mut(|data| data.get_persisted(egui::Id::unique(PROFILES_KEY))).unwrap_or_default();
        ui.menu_button(format!("{}  {}", icon::USER_CIRCLE, tr("Identity")), |ui| {
            ui.set_min_width(240.0);
            if profiles.is_empty() {
                ui.add_enabled(false, egui::Button::new(tr("No saved profiles")));
            }
            for profile in profiles {
                let core = IdentityProfile::new(&profile.name, &profile.email, profile.signing_key.as_deref());
                let selected = current.as_ref().is_some_and(|(name, email)| *name == core.name && email.eq_ignore_ascii_case(&core.email));
                let mark = if selected { icon::CHECK } else { " " };
                let text = format!("{mark}  {}", core.label);
                if ui.add_enabled(idle, egui::Button::new(text)).clicked() {
                    ui.close();
                    let (label, run) = tools::repository_settings::apply_profile(&profile);
                    self.act(&label, run);
                }
            }
            ui.separator();
            if ui.button(format!("{}  {}", icon::GEAR_SIX, tr("Manage identities…"))).clicked() {
                ui.close();
                self.open_tool(Box::new(tools::repository_settings::RepositorySettingsWindow::on_profiles()));
            }
        });
    }

    /// Asks before undoing the last recorded step, from the toolbar or the command palette.
    pub fn confirm_undo(&mut self) {
        let Some(title) = self.repo().and_then(|r| r.undo.last()).map(step_name) else { return };
        self.confirm(
            tr_fmt!("Undo {step}?", step = title),
            tr("NiceGit moves things back to where they were before. It refuses if the branch has moved since, so newer work is never lost."),
            tr("Undo"),
            crate::ui::dialogs::Pending::Undo,
        );
    }

    /// Asks before redoing the last undone step, from the toolbar or the command palette.
    pub fn confirm_redo(&mut self) {
        let Some(title) = self.repo().and_then(|r| r.redo.last()).map(step_name) else { return };
        self.confirm(
            tr_fmt!("Redo {step}?", step = title),
            tr("NiceGit puts back what the undo reversed. It refuses if the branch has moved since."),
            tr("Redo"),
            crate::ui::dialogs::Pending::Redo,
        );
    }

    /// Opens the Apply patch window, from the toolbar or the command palette.
    pub fn apply_patch(&mut self) {
        if self.snapshot().is_none() {
            return;
        }
        self.open_tool(Box::new(crate::tools::apply_patch::ApplyPatchWindow::new(None)));
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
        if ui
            .add_enabled(self.idle() && snapshot.head_hash.is_some(), egui::Button::new(format!("{}  {}", icon::PLUS, tr("New branch…"))))
            .clicked()
        {
            ui.close();
            self.dialog = Some(Dialog::input(
                tr("New branch"),
                tr("Create a branch at HEAD and switch to it"),
                "",
                InputKind::CreateBranch { branch: snapshot.current_branch.clone(), head: snapshot.head_hash.clone() },
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nicegit_core::undo::{UndoMode, UndoStep};

    fn step(title: &str, detail: Option<&str>) -> HistoryEntry {
        let step = UndoStep::BranchMove { branch: "main".into(), before: "a".into(), after: "b".into(), mode: UndoMode::Soft };
        HistoryEntry { title: title.into(), detail: detail.map(str::to_string), step }
    }

    #[test]
    fn undo_history_line_names_the_step_and_cuts_long_lines() {
        assert_eq!(history_line(&step("Commit", Some("Add a fourth line"))), "Commit: Add a fourth line");
        assert_eq!(history_line(&step("Commit", None)), "Commit");
        assert_eq!(step_name(&step("Commit", Some("Add a fourth line"))), "commit “Add a fourth line”");
        assert_eq!(step_name(&step("Commit", None)), "commit");

        let long = history_line(&step("Commit", Some(&"x".repeat(100))));
        assert_eq!(long.chars().count(), HISTORY_LINE_CHARS);
        assert!(long.ends_with('…'), "a cut line ends in an ellipsis: {long}");
    }
}
