//! The main window: repositories column, branches sidebar, toolbar and history in the middle,
//! and the changes panel or commit inspector on the right.

pub mod ai_draft;
pub mod branch_checks;
pub mod changes;
pub mod clone_picker;
pub mod commit_tree;
pub mod dialogs;
pub mod find_repositories;
pub mod github_section;
pub mod go_to_file;
pub mod history;
pub mod history_columns;
pub mod inspector;
pub mod palette;
pub mod pull_request;
pub mod repositories;
pub mod settings_window;
pub mod shortcuts_window;
pub mod sidebar;
pub mod tips;
pub mod toolbar;
pub mod updates;
pub mod workspaces;

use egui::{Key, KeyboardShortcut, Modifiers, RichText};
use egui_phosphor::regular as icon;

use crate::app::{NiceGitApp, Selection};
use crate::i18n::{tr, tr_count, tr_fmt};
use crate::settings::DiffContextSetting;
use crate::theme;
use crate::tools::widgets;

/// What a click on the welcome screen asks for, applied once the screen is drawn.
enum WelcomeAction {
    Open,
    Clone,
    Create,
    Recent(std::path::PathBuf),
    NextTip,
}

/// Room kept at the right of the status bar for the head commit and when the remotes were fetched,
/// so a long checkout name is shortened before they are pushed out of view.
const CHECKOUT_RESERVE: f32 = 180.0;

impl NiceGitApp {
    pub fn shortcuts(&mut self, ctx: &egui::Context) {
        let command = |key| KeyboardShortcut::new(Modifiers::COMMAND, key);
        let command_shift = |key| KeyboardShortcut::new(Modifiers::COMMAND | Modifiers::SHIFT, key);
        let command_alt = |key| KeyboardShortcut::new(Modifiers::COMMAND | Modifiers::ALT, key);
        if ctx.input_mut(|i| i.consume_shortcut(&command_shift(Key::P))) {
            self.toggle_palette();
        }
        if self.palette.is_none() && self.dialog.is_none() && ctx.input_mut(|i| i.consume_shortcut(&command(Key::P))) {
            self.toggle_go_to_file(ctx);
        }
        if ctx.input_mut(|i| i.consume_shortcut(&command_shift(Key::F))) && self.snapshot().is_some() {
            self.open_tool(Box::new(crate::tools::commit_search::CommitSearchWindow::new()));
        }
        if ctx.input_mut(|i| i.consume_shortcut(&command_shift(Key::H))) && self.snapshot().is_some() {
            self.go_to_head();
        }
        if ctx.input_mut(|i| i.consume_shortcut(&command_alt(Key::F))) && self.snapshot().is_some() {
            self.open_tool(Box::new(crate::tools::content_search::ContentSearchWindow::new(None)));
        }
        if ctx.input_mut(|i| i.consume_shortcut(&command(Key::R))) {
            self.load(true);
        }
        if ctx.input_mut(|i| i.consume_shortcut(&command(Key::O))) {
            self.choose_folder();
        }
        if ctx.input_mut(|i| i.consume_shortcut(&command(Key::Slash))) {
            self.show_shortcuts = !self.show_shortcuts;
        }
        if ctx.input_mut(|i| i.consume_shortcut(&command(Key::Comma))) {
            self.show_settings = !self.show_settings;
        }
        // Command-1 to Command-8 open the repository tabs in order; Command-9 the last, as in browsers.
        let tabs = [Key::Num1, Key::Num2, Key::Num3, Key::Num4, Key::Num5, Key::Num6, Key::Num7, Key::Num8, Key::Num9];
        for (position, key) in tabs.into_iter().enumerate() {
            if ctx.input_mut(|i| i.consume_shortcut(&command(key))) {
                let index = if key == Key::Num9 { self.repos.len().saturating_sub(1) } else { position };
                if index < self.repos.len() && index != self.active {
                    self.switch_to(index);
                }
            }
        }
        // Command-W closes the newest open window first, then the active repository tab.
        if ctx.input_mut(|i| i.consume_shortcut(&command(Key::W)))
            && self.dialog.is_none()
            && self.palette.is_none()
            && self.go_to_file.is_none()
        {
            if self.show_settings {
                self.show_settings = false;
            } else if self.tools.last().is_some_and(|tool| tool.has_unsaved_changes()) {
                self.notify(tr("Save or close the file you are editing first."), true);
            } else if !self.tools.is_empty() {
                self.tools.pop();
            } else if !self.repos.is_empty() {
                self.close_tab(self.active);
            }
        }
        if ctx.input_mut(|i| i.consume_shortcut(&command_shift(Key::S))) && self.showing_diff() {
            self.toggle_stage_open_file();
        }
        // Checked first: Command-Enter also matches while Shift is held. Both stay out of the way
        // of the Go to file finder, where Command-Enter opens blame.
        if self.go_to_file.is_none() && ctx.input_mut(|i| i.consume_shortcut(&command_shift(Key::Enter))) {
            self.commit_and_push();
        }
        if self.go_to_file.is_none() && ctx.input_mut(|i| i.consume_shortcut(&command(Key::Enter))) && self.can_commit() {
            self.commit();
        }
        if ctx.input_mut(|i| i.consume_shortcut(&KeyboardShortcut::new(Modifiers::CTRL, Key::Backtick))) {
            self.toggle_terminal(ctx);
        }
        if ctx.input_mut(|i| i.consume_shortcut(&command_shift(Key::L))) && self.snapshot().is_some() {
            changes::request_filter_focus(ctx);
        }
        // Escape in the Changes panel's filter clears it, and goes no further: it must not close an
        // open diff or clear the selection. Egui has already dropped the field's focus, so this
        // checks whether the field had focus when the panel was drawn in the previous pass.
        if changes::filter_had_focus(ctx) && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
            if let Some(repo) = self.repo_mut() {
                repo.change_filter.clear();
            }
        }
        // The row a keyboard move selected takes focus in the frame after the move (see history).
        if let Some(repo) = self.repo_mut() {
            repo.focus_row_in = repo.focus_row_in.saturating_sub(1);
        }
        // Arrow keys, Home and End move through the graph when no text field has focus. A graph row
        // that a keyboard move focused still takes them.
        let focused = ctx.memory(|m| m.focused());
        let typing = focused.is_some_and(|id| ctx.data(|d| d.get_temp::<egui::Id>(history::focused_row_key())) != Some(id));
        if !typing && self.dialog.is_none() && self.palette.is_none() {
            let (up, down, home, end, escape) = ctx.input(|i| {
                (
                    i.key_pressed(Key::ArrowUp),
                    i.key_pressed(Key::ArrowDown),
                    i.key_pressed(Key::Home),
                    i.key_pressed(Key::End),
                    i.key_pressed(Key::Escape),
                )
            });
            let diff = self.showing_diff();
            let alt = ctx.input(|i| i.modifiers.alt);
            // While a diff is open, Option (Alt) with an arrow opens the previous or next file;
            // the graph is hidden, so the arrows alone do not move through it.
            if (up || down) && diff && alt {
                self.step_diff_file(if up { -1 } else { 1 });
            } else if (up || down) && !diff {
                self.move_selection(if up { -1 } else { 1 });
            }
            if (home || end) && !diff {
                self.select_end_of_history(end);
            }
            if escape && diff {
                self.close_diff();
            } else if escape {
                self.clear_selection();
                if let Some(repo) = self.repo_mut() {
                    repo.marked.clear();
                }
            }
        }
    }

    /// Selects the graph's first row (the working tree, when it has changes, as Up reaches it) or
    /// its last loaded row, as the Home and End keys do.
    fn select_end_of_history(&mut self, oldest: bool) {
        let Some(count) = self.repo().map(|repo| repo.rows.len()).filter(|count| *count > 0) else { return };
        self.select_row(if oldest { count - 1 } else { 0 });
    }

    fn move_selection(&mut self, delta: isize) {
        let Some(repo) = self.repo() else { return };
        let count = repo.rows.len();
        if count == 0 {
            return;
        }
        let current = match &repo.selection {
            Selection::Commit { hash, .. } | Selection::CommitFile { hash, .. } => {
                (0..count).find(|&i| repo.commit_at(i).is_some_and(|c| &c.hash == hash))
            }
            Selection::WorkingTree | Selection::Change { .. } if repo.dirty() => Some(0),
            _ => None,
        };
        let next = match current {
            Some(index) => (index as isize + delta).clamp(0, count as isize - 1) as usize,
            None => 0,
        };
        if Some(next) == current {
            return;
        }
        self.select_row(next);
    }

    /// Selects the graph row at `index`: its commit, or the working tree for the row above them.
    fn select_row(&mut self, index: usize) {
        let Some(repo) = self.repo() else { return };
        match repo.commit_at(index).map(|c| c.hash.clone()) {
            Some(hash) => self.select_commit(hash),
            None => self.select_working_tree(),
        }
        if let Some(repo) = self.repo_mut() {
            repo.scroll_to_selection = true;
            repo.focus_row_in = 2;
        }
    }

    /// Shows or hides the terminal panel. Its shells keep running while hidden, and hiding it
    /// refreshes the repository so changes made in the shell appear.
    pub fn toggle_terminal(&mut self, ctx: &egui::Context) {
        if self.terminal.take().is_some() {
            self.load(false);
        } else if let Some(repo) = self.repo() {
            self.terminal = Some(crate::tools::terminal::TerminalWindow::new(ctx, repo.path.clone()));
        }
    }

    fn terminal_panel(&mut self, ui: &mut egui::Ui) {
        let (Some(repo), Some(_)) = (self.repo(), self.terminal.as_ref()) else { return };
        let (Some(snapshot), path) = (repo.snapshot.clone(), repo.path.clone()) else { return };
        let idle = self.busy.is_none();
        let mut requests = Vec::new();
        let mut hide = false;
        egui::Panel::bottom("terminal")
            .resizable(true)
            .default_size(280.0)
            .size_range(140.0..=800.0)
            .frame(egui::Frame::new().fill(ui.visuals().extreme_bg_color).inner_margin(egui::Margin::symmetric(10, 6)))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(format!("{}  {}", icon::TERMINAL_WINDOW, tr("Terminal"))).strong());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        hide = widgets::icon_button(ui, icon::CARET_DOWN, tr("Hide the terminal (Ctrl-`); its shells keep running"), true)
                            .clicked();
                    });
                });
                if let Some(terminal) = self.terminal.as_mut() {
                    let mut cx = crate::tools::Ctx::new(&path, &snapshot, idle, &mut requests);
                    crate::tools::ToolWindow::ui(terminal, ui, &mut cx);
                    hide |= terminal.take_close_request();
                }
            });
        for request in requests {
            self.handle(request);
        }
        if hide {
            self.terminal = None;
            self.load(false);
        }
    }

    pub fn layout(&mut self, ui: &mut egui::Ui) {
        let c = theme::of(ui);
        let panel = egui::Frame::new().fill(ui.visuals().panel_fill).inner_margin(egui::Margin::ZERO);
        // The update notice spans the bottom of the window, so it is declared before the side panels.
        self.update_notice(ui);
        // Side panels scale with the window so the history keeps a usable width on small screens.
        let width = ui.ctx().content_rect().width();
        // Each panel's draggable edge; double-clicking one restores that panel's default size.
        let mut edges: Vec<(&str, PanelEdge)> = Vec::new();
        if self.settings.show_repositories {
            let shown = egui::Panel::left("repositories")
                .resizable(true)
                .default_size(220.0_f32.min(width * 0.16))
                .size_range(150.0..=320.0_f32.min(width * 0.22).max(150.0))
                .frame(panel.fill(c.subtle_bg))
                .show(ui, |ui| self.repositories_column(ui));
            edges.push(("repositories", PanelEdge::Right(shown.response.rect)));
        }
        if self.snapshot().is_some() {
            let shown = egui::Panel::left("sidebar")
                .resizable(true)
                .default_size(260.0_f32.min(width * 0.2))
                .size_range(180.0..=440.0_f32.min(width * 0.3).max(180.0))
                .frame(panel)
                .show(ui, |ui| self.sidebar(ui));
            edges.push(("sidebar", PanelEdge::Right(shown.response.rect)));
            let shown = egui::Panel::right("changes")
                .resizable(true)
                .default_size(400.0_f32.min(width * 0.28))
                .size_range(260.0..=640.0_f32.min(width * 0.4).max(260.0))
                .frame(panel)
                .show(ui, |ui| {
                    if matches!(
                        self.repo().map(|r| &r.selection),
                        Some(Selection::Commit { .. } | Selection::CommitFile { .. } | Selection::Stash { .. })
                    ) {
                        self.inspector(ui);
                    } else {
                        self.changes_panel(ui);
                    }
                });
            edges.push(("changes", PanelEdge::Left(shown.response.rect)));
        }
        egui::CentralPanel::default().frame(egui::Frame::new().fill(ui.visuals().window_fill)).show(ui, |ui| {
            if self.snapshot().is_none() {
                self.empty_window(ui);
                return;
            }
            egui::Panel::top("toolbar")
                .frame(egui::Frame::new().inner_margin(egui::Margin::symmetric(14, 8)))
                .show(ui, |ui| self.toolbar(ui));
            egui::Panel::bottom("statusbar")
                .frame(egui::Frame::new().inner_margin(egui::Margin::symmetric(14, 5)))
                .show(ui, |ui| self.status_bar(ui));
            self.terminal_panel(ui);
            self.operation_banner(ui);
            self.bisect_banner(ui);
            // A file's diff takes the whole main area in place of the history, so long lines
            // and side-by-side views have room; History or Escape goes back.
            if self.showing_diff() {
                egui::Frame::new().fill(ui.visuals().extreme_bg_color).show(ui, |ui| {
                    ui.set_min_size(ui.available_size());
                    if matches!(self.repo().map(|r| &r.selection), Some(Selection::CommitFile { .. })) {
                        self.contents_panel(ui);
                    } else {
                        self.diff_panel(ui);
                    }
                });
            } else {
                self.history(ui);
            }
        });
        restore_on_double_click(ui.ctx(), &edges);
    }

    /// The window shown while no repository is open: the three ways to start one, the recent
    /// repositories that still exist, and a tip. The buttons call what the repository menu's
    /// items call. The content scrolls, so a small window never clips it.
    fn welcome_screen(&mut self, ui: &mut egui::Ui) {
        let c = theme::of(ui);
        let recent: Vec<std::path::PathBuf> =
            self.settings.recent.iter().filter(|path| repositories::is_repository_folder(path)).take(8).cloned().collect();
        let tip = tips::text(self.welcome_tip);
        let mut action = None;
        egui::ScrollArea::vertical().id_salt("welcome").auto_shrink([false, false]).show(ui, |ui| {
            let width = (ui.available_width() - 32.0).clamp(200.0, 560.0);
            ui.horizontal(|ui| {
                ui.add_space(((ui.available_width() - width) / 2.0).max(0.0));
                ui.vertical(|ui| {
                    ui.set_width(width);
                    ui.add_space(28.0);
                    ui.vertical_centered(|ui| {
                        ui.label(RichText::new(icon::GIT_BRANCH).size(48.0).color(c.accent));
                        ui.add_space(6.0);
                        ui.label(RichText::new(tr("Welcome to NiceGit")).size(26.0).strong());
                        ui.label(RichText::new(tr("A fast Git client that keeps your work safe.")).color(c.muted));
                    });
                    ui.add_space(18.0);
                    let gap = 10.0;
                    let narrow = width < 3.0 * 150.0 + 2.0 * gap;
                    let button_width = if narrow { width } else { (width - 2.0 * gap) / 3.0 };
                    // The first button is the primary one; the others are ordinary buttons.
                    let start_button = |ui: &mut egui::Ui, label: String, primary: bool| -> bool {
                        let text = RichText::new(label).size(15.0);
                        let widget =
                            if primary { egui::Button::new(text.color(c.accent_text)).fill(c.accent) } else { egui::Button::new(text) };
                        ui.add(widget.min_size(egui::vec2(button_width, 44.0))).clicked()
                    };
                    let starts = [
                        (format!("{}  {}", icon::FOLDER_OPEN, tr("Open repository…")), true, WelcomeAction::Open),
                        (format!("{}  {}", icon::DOWNLOAD_SIMPLE, tr("Clone repository…")), false, WelcomeAction::Clone),
                        (format!("{}  {}", icon::PLUS, tr("Create repository…")), false, WelcomeAction::Create),
                    ];
                    if narrow {
                        // Too narrow for a row: the three buttons stack as a column.
                        ui.vertical_centered(|ui| {
                            for (label, primary, to) in starts {
                                if start_button(ui, label, primary) {
                                    action = Some(to);
                                }
                                ui.add_space(gap);
                            }
                        });
                    } else {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = gap;
                            for (label, primary, to) in starts {
                                if start_button(ui, label, primary) {
                                    action = Some(to);
                                }
                            }
                        });
                    }
                    if !recent.is_empty() {
                        ui.add_space(24.0);
                        widgets::section(ui, tr("Recent repositories"));
                        ui.add_space(4.0);
                        for path in &recent {
                            if repositories::recent_start_row(ui, path, c) {
                                action = Some(WelcomeAction::Recent(path.clone()));
                            }
                        }
                    }
                    ui.add_space(18.0);
                    ui.vertical_centered(|ui| {
                        ui.label(RichText::new(tr("Or drag a folder onto this window.")).color(c.muted));
                    });
                    ui.add_space(14.0);
                    egui::Frame::new().fill(c.card_bg).corner_radius(10.0).inner_margin(egui::Margin::symmetric(14, 12)).show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(icon::LIGHTBULB).size(22.0).color(c.accent));
                            ui.vertical(|ui| {
                                ui.add(egui::Label::new(RichText::new(&tip)).wrap());
                                ui.add_space(6.0);
                                if ui.button(tr("Next tip")).clicked() {
                                    action = Some(WelcomeAction::NextTip);
                                }
                            });
                        });
                    });
                    ui.add_space(24.0);
                });
            });
        });
        match action {
            Some(WelcomeAction::Open) => self.choose_folder(),
            Some(WelcomeAction::Clone) => self.dialog = Some(dialogs::Dialog::clone_repository()),
            Some(WelcomeAction::Create) => self.create_repository(),
            Some(WelcomeAction::Recent(path)) => self.open(path),
            Some(WelcomeAction::NextTip) => self.next_tip(),
            None => {}
        }
    }

    fn empty_window(&mut self, ui: &mut egui::Ui) {
        if self.repos.is_empty() && !self.git_missing {
            self.welcome_screen(ui);
            return;
        }
        let c = theme::of(ui);
        let error = self.repo().and_then(|r| r.error.clone());
        let loading = self.repo().is_some_and(|r| r.loading);
        ui.vertical_centered(|ui| {
            ui.add_space(ui.available_height() * 0.28);
            ui.label(RichText::new(icon::GIT_BRANCH).size(54.0).color(c.accent));
            ui.add_space(6.0);
            ui.label(RichText::new("NiceGit").size(28.0).strong());
            ui.add_space(2.0);
            if self.git_missing {
                ui.label(RichText::new(tr("Git was not found. Install Git, then restart NiceGit.")).color(c.danger));
                return;
            }
            if loading {
                widgets::loading(ui, tr("Opening repository…"));
                return;
            }
            if let Some(error) = error {
                ui.label(RichText::new(error).color(c.danger));
                ui.add_space(8.0);
            } else {
                ui.label(RichText::new(tr("A fast Git client for macOS, Windows, and Linux.")).color(c.muted));
                ui.add_space(14.0);
            }
            ui.horizontal(|ui| {
                let width = 420.0_f32.min(ui.available_width());
                ui.add_space((ui.available_width() - width).max(0.0) / 2.0);
                if widgets::primary_button(ui, &format!("{}  {}", icon::FOLDER_OPEN, tr("Open repository…")), true).clicked() {
                    self.choose_folder();
                }
                if ui.button(format!("{}  {}", icon::DOWNLOAD_SIMPLE, tr("Clone…"))).clicked() {
                    self.dialog = Some(dialogs::Dialog::clone_repository());
                }
                if ui.button(format!("{}  {}", icon::PLUS, tr("New repository…"))).clicked() {
                    self.create_repository();
                }
            });
            let recent: Vec<_> = self.settings.recent.iter().filter(|p| p.exists()).take(6).cloned().collect();
            if !recent.is_empty() {
                ui.add_space(24.0);
                widgets::section(ui, tr("Recent"));
                for path in recent {
                    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    if ui
                        .add(egui::Button::new(RichText::new(format!("{}  {name}", icon::FOLDER)).size(14.0)).frame(false))
                        .on_hover_text(path.display().to_string())
                        .clicked()
                    {
                        self.open(path);
                    }
                }
            }
            ui.add_space(18.0);
            ui.label(RichText::new(tr("Ctrl/Cmd-O to open · Shift-Ctrl/Cmd-P for commands")).small().color(c.muted));
        });
    }

    /// Opens the New repository dialog, with Git's default first branch name.
    pub fn create_repository(&mut self) {
        let initial_branch = nicegit_core::GitClient::new().default_initial_branch();
        self.dialog = Some(dialogs::Dialog::new_repository(initial_branch));
    }

    fn status_bar(&mut self, ui: &mut egui::Ui) {
        let c = theme::of(ui);
        let Some(snapshot) = self.snapshot().cloned() else { return };
        ui.horizontal(|ui| {
            if let Some(notice) = &self.notice {
                let action = notice.action.clone();
                let detail = notice.detail.clone();
                let color = if notice.is_error { c.danger } else { c.accent };
                let glyph = if notice.is_error { icon::WARNING_CIRCLE } else { icon::CHECK_CIRCLE };
                ui.label(RichText::new(glyph).color(color));
                let text = RichText::new(notice.text.clone()).color(if notice.is_error { c.danger } else { ui.visuals().text_color() });
                // The buttons are laid out from the right first, so a long message is shortened
                // rather than pushing them out of view; the whole message is in its tooltip.
                let (mut dismiss, mut pull, mut identity, mut stash) = (false, false, false, false);
                let mut open_link: Option<String> = None;
                let mut retry_commit: Option<crate::app::HookRetry> = None;
                let mut apply_into: Option<(nicegit_core::Stash, bool)> = None;
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    dismiss = ui.add(egui::Button::new(RichText::new(icon::X).small()).frame(false)).on_hover_text(tr("Dismiss")).clicked();
                    if action == Some(crate::app::NoticeAction::Stash) {
                        stash = ui.add_enabled(self.idle(), egui::Button::new(tr("Stash changes…"))).clicked();
                    }
                    if action == Some(crate::app::NoticeAction::SetIdentity) {
                        identity = ui.button(tr("Set your identity…")).clicked();
                    }
                    if action == Some(crate::app::NoticeAction::Integrate) {
                        pull = ui
                            .add_enabled(self.idle(), egui::Button::new(tr("Merge or rebase…")))
                            .on_hover_text(tr("Fetch, then choose whether to merge the remote's commits or rebase onto them"))
                            .clicked();
                    }
                    if let Some(crate::app::NoticeAction::OpenUrl(url)) = &action {
                        if ui.button(tr("Open")).on_hover_text(url.as_str()).clicked() {
                            open_link = Some(url.clone());
                        }
                    }
                    if let Some(crate::app::NoticeAction::ApplyIntoChanges { stash, drop_after }) = &action {
                        let button = ui
                            .add_enabled(self.idle(), egui::Button::new(tr("Apply into my changes")))
                            .on_hover_text(tr("Apply the stash's changes on top of your changes, and keep your changes"));
                        if button.clicked() {
                            apply_into = Some((stash.clone(), *drop_after));
                        }
                    }
                    if let Some(crate::app::NoticeAction::CommitWithoutHooks(retry)) = &action {
                        let button = ui
                            .add_enabled(self.idle(), egui::Button::new(tr("Commit without hooks")))
                            .on_hover_text(tr("Make the same commit again with Git hooks skipped"));
                        if button.clicked() {
                            retry_commit = Some(retry.clone());
                        }
                    }
                    ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                        // The full message, and Git's own words when the message explains them.
                        let full = match &detail {
                            Some(detail) => format!("{}\n\n{}\n{detail}", text.text(), tr("Git said:")),
                            None => text.text().to_string(),
                        };
                        ui.add(egui::Label::new(text).truncate()).on_hover_text(full);
                    });
                });
                let opened = open_link.is_some();
                if let Some(url) = &open_link {
                    crate::ui::changes::open_url(url);
                }
                if dismiss || pull || identity || stash || opened {
                    self.notice = None;
                }
                if stash {
                    self.dialog = Some(crate::ui::dialogs::Dialog::input(
                        tr("Stash changes"),
                        tr("Message (optional)"),
                        "",
                        crate::ui::dialogs::InputKind::SaveStash,
                    ));
                }
                if identity {
                    self.open_tool(Box::new(crate::tools::repository_settings::RepositorySettingsWindow::new()));
                }
                if pull {
                    self.integrate_upstream();
                }
                if let Some(retry) = retry_commit {
                    self.notice = None;
                    self.commit_without_hooks(retry);
                }
                if let Some((stash, drop_after)) = apply_into {
                    self.notice = None;
                    self.act(tr("Apply into changes"), self.guard_rewrite(move |client, path| {
                        client.apply_stash_into_changes(&stash, drop_after, path).map(|outcome| {
                            Some(match outcome {
                                nicegit_core::staging::StashIntoChanges::Removed => {
                                    tr("Applied the stash on top of your changes and removed it.").to_string()
                                }
                                nicegit_core::staging::StashIntoChanges::Kept => tr("Applied the stash on top of your changes.").to_string(),
                                nicegit_core::staging::StashIntoChanges::KeptForStagedChanges => {
                                    tr("Applied the stash's unstaged changes on top of yours. It also saved staged changes, which were not applied, so the stash is kept.")
                                        .to_string()
                                }
                            })
                        })
                    }));
                }
                return;
            }
            if let Some(label) = self.busy.clone() {
                ui.spinner();
                ui.label(RichText::new(format!("{label}…")).color(c.muted));
                return;
            }
            ui.label(RichText::new(icon::GIT_BRANCH).color(c.muted));
            // The pull request keeps its width; the checkout's name is shortened to fit beside it.
            let badge = self.pull_request_status().map(|status| crate::ui::pull_request::PrBadge::new(&status, &c));
            let reserved = badge.as_ref().map_or(0.0, |badge| badge.width(ui) + ui.spacing().item_spacing.x);
            let checkout_width = (ui.available_width() - reserved - CHECKOUT_RESERVE).max(0.0);
            let checkout = tr_fmt!("Current checkout: {branch}", branch = snapshot.current_branch);
            ui.scope(|ui| {
                ui.set_max_width(checkout_width);
                ui.add(egui::Label::new(RichText::new(&checkout).monospace().small().color(c.muted)).truncate())
                    .on_hover_text(checkout.as_str());
            });
            if let Some(badge) = &badge {
                match crate::ui::pull_request::show_pull_request_badge(ui, badge) {
                    crate::ui::pull_request::BadgeAction::Open => crate::ui::changes::open_url(&badge.url),
                    crate::ui::pull_request::BadgeAction::Details => self.open_current_pull_request_details(badge.number),
                    crate::ui::pull_request::BadgeAction::None => {}
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if let Some(head) = &snapshot.head_hash {
                    ui.label(RichText::new(nicegit_core::models::short(head)).monospace().small().color(c.muted));
                }
                // When the remotes were last fetched, or why the background fetch failed.
                if let Some(repo) = self.repo().filter(|_| !snapshot.remotes.is_empty()) {
                    if let Some(problem) = &repo.fetch_problem {
                        let plain = crate::app::remote_problem(problem).map(|plain| format!("{plain}\n\n")).unwrap_or_default();
                        ui.label(RichText::new(format!("{}  {}", icon::CLOUD_SLASH, tr("Couldn't fetch"))).small().color(c.warning))
                            .on_hover_text(format!("{}\n{problem}", tr_fmt!("The last automatic fetch failed. {plain}{said}", plain = plain, said = tr("Git said:"))));
                    } else if let Some(when) = repo.last_fetched {
                        ui.label(RichText::new(fetched_ago(when.elapsed())).small().color(c.muted))
                            .on_hover_text(tr("Remotes are fetched automatically; change how often in Settings."));
                        // The age is in minutes, so it is redrawn once a minute.
                        ui.ctx().request_repaint_after(std::time::Duration::from_secs(60));
                    }
                }
                if self.repo().is_some_and(|r| r.loading) {
                    ui.spinner();
                }
            });
        });
    }

    fn operation_banner(&mut self, ui: &mut egui::Ui) {
        let Some(snapshot) = self.snapshot().cloned() else { return };
        let Some(operation) = snapshot.operation else { return };
        let c = theme::of(ui);
        let conflicts: Vec<String> =
            snapshot.status.iter().filter(|e| e.kind == nicegit_core::StatusKind::Conflicted).map(|e| e.path.clone()).collect();
        egui::Frame::new().fill(c.banner_bg).inner_margin(egui::Margin::symmetric(14, 10)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(icon::WARNING).size(16.0).color(c.warning));
                let text = if snapshot.rebase_stopped_for_edit {
                    tr("Rebase stopped to edit a commit: stage your changes, then Continue.").to_string()
                } else if conflicts.is_empty() {
                    tr_fmt!(
                        "A {operation} is in progress. Continue when ready, or abort to return to where you started.",
                        operation = operation.name()
                    )
                } else {
                    tr_fmt!(
                        "A {operation} stopped with {count} conflicted file(s). Resolve each one, then continue.",
                        operation = operation.name(),
                        count = conflicts.len()
                    )
                };
                ui.label(RichText::new(text).color(c.warning).strong());
            });
            if !conflicts.is_empty() {
                ui.horizontal_wrapped(|ui| {
                    for path in &conflicts {
                        if ui.button(format!("{}  {path}", icon::GIT_DIFF)).on_hover_text(tr("Open the conflict editor")).clicked() {
                            self.open_tool(Box::new(crate::tools::conflict::ConflictWindow::new(path.clone())));
                        }
                    }
                });
            }
            ui.horizontal(|ui| {
                let idle = self.idle();
                if widgets::primary_button(ui, tr("Continue"), idle && conflicts.is_empty()).clicked() {
                    self.act_recording(tr("Continue"), operation.name(), move |client, path| {
                        client.continue_operation(operation, path).map(|_| None)
                    });
                }
                if ui.add_enabled(idle, egui::Button::new(tr("Abort…"))).clicked() {
                    self.confirm(
                        tr_fmt!("Abort the {operation}?", operation = operation.name()),
                        tr("Git returns the branch and files to the state before it started. Resolutions you made are lost."),
                        tr("Abort"),
                        dialogs::Pending::Abort(operation),
                    );
                }
                if !conflicts.is_empty() {
                    let (current, incoming) = crate::tools::conflict::side_names(Some(operation));
                    // An open conflict editor may hold an unsaved result that would be discarded.
                    let editing = self.tools.iter().any(|tool| tool.id().starts_with("conflict:"));
                    let enabled = idle && !self.has_unsaved_editor() && !editing;
                    ui.add_enabled_ui(enabled, |ui| {
                        ui.menu_button(tr("Resolve all…"), |ui| {
                            for (side, from_incoming) in [(current, false), (incoming, true)] {
                                let side = side.to_lowercase();
                                if ui
                                    .button(tr_fmt!("Take {side} for all", side = side))
                                    .on_hover_text(tr("Take one side of every conflicted file"))
                                    .clicked()
                                {
                                    ui.close();
                                    let (title, message, button) = dialogs::resolve_conflicts_confirmation(&conflicts, &side);
                                    self.confirm(
                                        title,
                                        message,
                                        button,
                                        dialogs::Pending::ResolveConflictsSide {
                                            paths: conflicts.clone(),
                                            incoming: from_incoming,
                                            side_name: side,
                                        },
                                    );
                                }
                            }
                        });
                    });
                }
            });
        });
    }

    /// While a bisect runs: the commit being tested, Good / Bad / Skip, and End.
    fn bisect_banner(&mut self, ui: &mut egui::Ui) {
        use nicegit_core::bisect::BisectMark;
        let Some(status) = self.repos.get_mut(self.active).and_then(|r| r.bisect.as_mut()).and_then(|t| t.get().cloned()).flatten() else {
            return;
        };
        let c = theme::of(ui);
        let idle = self.idle();
        egui::Frame::new().fill(c.subtle_bg).inner_margin(egui::Margin::symmetric(14, 8)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(icon::BUG).size(16.0).color(c.accent));
                match (&status.first_bad, &status.testing) {
                    (Some(first_bad), _) => {
                        let subject = self
                            .snapshot()
                            .and_then(|s| s.commits.iter().find(|c| &c.hash == first_bad))
                            .map(|c| c.subject.clone())
                            .unwrap_or_default();
                        ui.label(
                            RichText::new(tr_fmt!(
                                "First bad commit: {hash} {subject}",
                                hash = nicegit_core::models::short(first_bad),
                                subject = subject
                            ))
                            .strong(),
                        );
                        if ui.button(tr("Show")).clicked() {
                            self.select_commit(first_bad.clone());
                        }
                    }
                    (None, Some(testing)) => {
                        let steps = status
                            .remaining_steps
                            .map(|n| tr_count(" · about {count} step left", " · about {count} steps left", n))
                            .unwrap_or_default();
                        ui.label(
                            RichText::new(tr_fmt!(
                                "Bisecting: test {hash} and mark it{steps}",
                                hash = nicegit_core::models::short(testing),
                                steps = steps
                            ))
                            .strong(),
                        );
                        for (mark, label, action) in [
                            (BisectMark::Good, tr("Good"), tr("Mark good")),
                            (BisectMark::Bad, tr("Bad"), tr("Mark bad")),
                            (BisectMark::Skip, tr("Skip"), tr("Mark skip")),
                        ] {
                            if ui.add_enabled(idle, egui::Button::new(label)).clicked() {
                                let testing = testing.clone();
                                self.act(
                                    action,
                                    self.guard_rewrite(move |client, path| client.mark_bisect(mark, &testing, path).map(Some)),
                                );
                            }
                        }
                    }
                    (None, None) => {
                        ui.label(RichText::new(tr("Bisecting: mark a good and a bad commit to begin.")).strong());
                    }
                }
                ui.add_space(12.0);
                if ui
                    .add_enabled(idle, egui::Button::new(tr_fmt!("End bisect, return to {branch}", branch = status.original_checkout)))
                    .clicked()
                {
                    self.act(
                        tr("End bisect"),
                        self.guard_rewrite(|client, path| client.end_bisect(path).map(|_| Some(tr("Ended the bisect.").into()))),
                    );
                }
            });
        });
    }

    fn diff_panel(&mut self, ui: &mut egui::Ui) {
        let c = theme::of(ui);
        let panel_width = ui.available_width();
        self.diff_panel_width = Some(panel_width);
        let Some(repo) = self.repo() else { return };
        let title = repo.diff.as_ref().map(|d| d.title.clone()).unwrap_or_default();
        let loading = repo.diff_loading;
        let editable = match &repo.selection {
            Selection::Change { entry, staged: false } if entry.kind != nicegit_core::StatusKind::Deleted => Some(entry.path.clone()),
            _ => None,
        };
        // A narrow panel puts the view controls on a second row, so the file's name keeps room.
        // The controls need about 670 points beside History and the file steps, so the header
        // stays on one row only from 960 points, leaving room for the name.
        let wide = panel_width >= 960.0;
        egui::Frame::new().fill(ui.visuals().panel_fill).inner_margin(egui::Margin::symmetric(14, 7)).show(ui, |ui| {
            ui.horizontal(|ui| {
                let back =
                    ui.button(format!("{}  {}", icon::CARET_LEFT, tr("History"))).on_hover_text(tr("Back to the commit history (Esc)"));
                if back.clicked() {
                    self.close_diff();
                }
                // Step through the other changed files without going back to the list, when there
                // are others.
                if let Some((position, count)) = self.diff_file_position().filter(|&(_, count)| count > 1) {
                    let shortcut = if cfg!(target_os = "macos") { "Option" } else { "Alt" };
                    let previous = tr_fmt!("Previous file ({shortcut}-Up)", shortcut = shortcut);
                    if widgets::icon_button(ui, icon::CARET_UP, &previous, position > 0).clicked() {
                        self.step_diff_file(-1);
                    }
                    let next = tr_fmt!("Next file ({shortcut}-Down)", shortcut = shortcut);
                    if widgets::icon_button(ui, icon::CARET_DOWN, &next, position + 1 < count).clicked() {
                        self.step_diff_file(1);
                    }
                    ui.label(
                        RichText::new(tr_fmt!("{position} of {count}", position = position + 1, count = count)).small().color(c.muted),
                    );
                }
                ui.add_space(6.0);
                let file_name = |ui: &mut egui::Ui| {
                    // A spinner stands in for the icon while the diff reloads: one widget either
                    // way, so the buttons after it keep their identities.
                    if loading {
                        ui.add(egui::Spinner::new());
                    } else {
                        ui.label(RichText::new(icon::GIT_DIFF).color(c.muted));
                    }
                    ui.add(egui::Label::new(RichText::new(&title).strong()).truncate()).on_hover_text(&title);
                };
                if wide {
                    // The controls are laid out from the right first, so the file's name takes
                    // only the room left between them and the History button.
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        self.diff_view_controls(ui, panel_width, editable.clone());
                        ui.add_space(8.0);
                        ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), file_name);
                    });
                } else {
                    file_name(ui);
                }
            });
            if !wide {
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    // The controls wrap onto another line when a narrow panel cannot fit them all,
                    // so the last ones added, such as Copy, are never cut off.
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center).with_main_wrap(true), |ui| {
                        self.diff_view_controls(ui, panel_width, editable.clone());
                    });
                });
            }
        });
        ui.separator();
        let split = self.settings.split_diff;
        let words = self.settings.word_diff;
        // Stashing is refused during an unfinished Git operation, so its buttons are not offered.
        let operation_running = self.snapshot().is_some_and(|snapshot| snapshot.operation.is_some());
        let repo = &mut self.repos[self.active];
        repo.diff_options.split = split;
        repo.diff_options.words = words;
        // Lines can be staged one by one once the review has loaded and matches the diff shown.
        // While a refresh reloads the review, the previous one stays, so the buttons remain.
        let review_loading = repo.review.as_mut().is_some_and(|task| task.is_loading());
        if repo.review_loading && !review_loading {
            // The fresh review has arrived, so the lines are matched to it again.
            repo.line_map = None;
        }
        repo.review_loading = review_loading;
        let review = repo.review.as_mut().and_then(|task| task.get()).and_then(|r| r.as_ref().ok()).cloned();
        // The shown diff has a few lines of context and the review the whole file, so lines are
        // matched between them once both have loaded.
        if let (None, Some(review), Some(diff)) = (&repo.line_map, &review, &repo.diff) {
            repo.line_map = Some(nicegit_core::staging::line_map(&diff.lines, review).map(std::sync::Arc::new));
        }
        let line_map = repo.line_map.clone().flatten();
        // Lines of a working change can be staged, unless the words view is showing instead.
        let stageable = review.as_ref().is_some_and(|r| r.line_staging_unavailable.is_none()) && line_map.is_some();
        let selectable = stageable && !words;
        repo.diff_options.selectable = selectable;
        if !selectable {
            repo.diff_options.selected.clear();
        }
        // Each run of changed lines offers Stage, Discard and Stash, or Unstage, while hovered.
        repo.diff_options.block_buttons = review
            .as_ref()
            .map(|review| {
                if review.staged {
                    crate::diff_view::BlockButtons::Unstage
                } else {
                    crate::diff_view::BlockButtons::StageAndDiscard { stash: !operation_running }
                }
            })
            .filter(|_| !words);
        let selected = repo.diff_options.selected.clone();
        if selectable && !selected.is_empty() {
            let staged = review.as_ref().is_some_and(|r| r.staged);
            egui::Frame::new().fill(c.subtle_bg).inner_margin(egui::Margin::symmetric(14, 6)).show(ui, |ui| {
                // Wraps in a narrow diff panel, so every button stays on screen.
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new(tr_count("{count} line selected", "{count} lines selected", selected.len())).color(c.muted));
                    let text = if staged {
                        format!("{}  {}", icon::MINUS_CIRCLE, tr("Unstage selected lines"))
                    } else {
                        format!("{}  {}", icon::PLUS_CIRCLE, tr("Stage selected lines"))
                    };
                    if widgets::primary_button(ui, &text, self.busy.is_none()).clicked() {
                        let lines = line_map.as_deref().and_then(|map| in_review(map, &selected));
                        if let (Some(review), Some(lines)) = (review.clone(), lines) {
                            let label = if staged { tr("Unstage lines") } else { tr("Stage lines") };
                            self.act(label, move |client, path| client.stage_lines(&lines, &review, path).map(|_| None));
                        }
                    }
                    // Unstaged lines can also be discarded, as a run's Discard button does.
                    if !staged {
                        let discard = egui::Button::new(
                            RichText::new(format!("{}  {}", icon::ARROW_COUNTER_CLOCKWISE, tr("Discard selected lines"))).color(c.removed),
                        );
                        if ui.add_enabled(self.busy.is_none(), discard).clicked() {
                            let lines = line_map.as_deref().and_then(|map| in_review(map, &selected));
                            if let (Some(review), Some(lines)) = (review.clone(), lines) {
                                self.run_block_action(crate::diff_view::BlockAction::Discard, lines, review);
                            }
                        }
                        if !operation_running {
                            let stash = egui::Button::new(RichText::new(format!("{}  {}", icon::ARCHIVE, tr("Stash selected lines"))));
                            if ui.add_enabled(self.busy.is_none(), stash).clicked() {
                                let lines = line_map.as_deref().and_then(|map| in_review(map, &selected));
                                if let (Some(review), Some(lines)) = (review.clone(), lines) {
                                    self.run_block_action(crate::diff_view::BlockAction::Stash, lines, review);
                                }
                            }
                        }
                    }
                    if ui.button(tr("Clear")).clicked() {
                        self.repos[self.active].diff_options.selected.clear();
                    }
                });
            });
        } else if selectable {
            ui.horizontal(|ui| {
                ui.add_space(14.0);
                // Wraps to a second line in a narrow diff panel rather than running past its edge.
                ui.add(
                    egui::Label::new(
                        RichText::new(if review.as_ref().is_some_and(|r| r.staged) {
                            tr("Press − beside a line to unstage it, or click and drag over lines to choose several.")
                        } else {
                            tr("Press + beside a line to stage it, or click and drag over lines to choose several.")
                        })
                        .small()
                        .color(c.muted),
                    )
                    .wrap(),
                );
            });
        } else if stageable && words {
            // Said once, so a reader knows why a line cannot be staged here.
            ui.horizontal(|ui| {
                ui.add_space(14.0);
                let verb = if review.as_ref().is_some_and(|r| r.staged) { tr("unstage") } else { tr("stage") };
                ui.add(
                    egui::Label::new(
                        RichText::new(tr_fmt!("Switch to Unified to {verb} single lines.", verb = verb)).small().color(c.muted),
                    )
                    .wrap(),
                );
            });
        }
        let repo = &mut self.repos[self.active];
        let mut block_action = None;
        // Git only says that a binary file differs; for an image, both versions are shown.
        let binary = repo.diff.as_ref().is_some_and(|diff| diff.is_empty());
        let preview = repo.image_preview.as_mut().and_then(|task| task.get()).and_then(|pair| pair.as_ref().ok()).filter(|_| binary);
        if let Some(pair) = preview {
            if repo.image_textures.is_none() {
                repo.image_textures = Some(pair.textures(ui.ctx(), &title));
            }
            egui::Frame::new().inner_margin(egui::Margin::symmetric(14, 10)).show(ui, |ui| pair.show(ui, repo.image_textures.as_ref()));
        } else if let Some(diff) = repo.diff.as_ref() {
            block_action = crate::diff_view::show_with(ui, diff, &mut repo.diff_options).block_action;
        } else {
            ui.take_available_space();
        }
        let lines = block_action.as_ref().and_then(|(_, lines)| line_map.as_deref().and_then(|map| in_review(map, lines)));
        if let (Some((action, _)), Some(lines), Some(review)) = (block_action, lines, review) {
            self.run_block_action(action, lines, review);
        }
    }
}

impl NiceGitApp {
    /// The diff's view controls, laid out from the right: Split, Unified and Words, Hide
    /// whitespace, and Edit for a working file.
    fn diff_view_controls(&mut self, ui: &mut egui::Ui, panel_width: f32, editable: Option<String>) {
        let mut split = self.settings.split_diff;
        let mut words = self.settings.word_diff;
        // A diff with only one side always shows in one column.
        let one_sided = self.repo().and_then(|r| r.diff.as_ref()).is_some_and(|d| d.is_one_sided());
        let narrow = !crate::diff_view::split_fits(panel_width);
        let reason = if one_sided {
            tr("A new or deleted file has only one side, so it is shown in one column")
        } else {
            tr("Widen the diff panel to show changes side by side")
        };
        let available = !one_sided && !narrow;
        // The control shows the layout on screen: Unified whenever Split cannot apply,
        // while the saved preference waits for a diff and a panel that suit it.
        // Only Split is disabled, so the active Unified keeps full emphasis. Words replaces
        // both, so neither Split nor Unified is shown as active while it is on.
        let shown = split && available && !words;
        if ui.add_enabled(available, egui::Button::selectable(shown, tr("Split"))).on_disabled_hover_text(reason).clicked() {
            split = true;
            words = false;
        }
        if ui.add(egui::Button::selectable(!shown && !words, tr("Unified"))).clicked() {
            words = false;
            if available {
                split = false;
            }
        }
        // Words works in any panel width, as its lines wrap in one column.
        if ui.add(egui::Button::selectable(words, tr("Words"))).on_hover_text(tr("Show changed words merged into each line")).clicked() {
            words = true;
        }
        self.settings.split_diff = split;
        self.settings.word_diff = words;
        let mut ignore = self.settings.ignore_whitespace;
        if crate::tools::widgets::checkbox(ui, true, &mut ignore, tr("Hide whitespace")).changed() {
            self.settings.ignore_whitespace = ignore;
            self.reload_diff();
        }
        // How many unchanged lines show around each change; a change reads the diff again.
        let mut context = self.settings.diff_context;
        egui::ComboBox::from_id_salt("diff-context")
            .width(110.0)
            .selected_text(context.label())
            .show_ui(ui, |ui| {
                for option in DiffContextSetting::ALL {
                    ui.selectable_value(&mut context, option, option.label());
                }
            })
            .response
            .on_hover_text(tr("How many unchanged lines to show around each change"))
            .widget_info(|| egui::WidgetInfo::labeled(egui::Role::ComboBox, true, tr("Context lines")));
        if context != self.settings.diff_context {
            self.settings.diff_context = context;
            self.reload_diff();
        }
        if let Some(path) = editable {
            if ui.button(format!("{}  {}", icon::PENCIL_SIMPLE, tr("Edit"))).on_hover_text(tr("Edit this file in NiceGit")).clicked() {
                let repo_path = self.repo().map(|r| r.path.clone()).unwrap_or_default();
                self.open_tool(Box::new(crate::tools::editor::EditorWindow::new(repo_path, path)));
            }
        }
        // Stage or unstage the whole open file without going back to the Changes panel.
        if let Some(Selection::Change { entry, staged }) = self.repo().map(|r| r.selection.clone()) {
            if entry.kind != nicegit_core::StatusKind::Conflicted {
                let (glyph, label) = if staged { (icon::MINUS_CIRCLE, tr("Unstage file")) } else { (icon::PLUS_CIRCLE, tr("Stage file")) };
                let button = ui.add_enabled(self.idle(), egui::Button::new(format!("{glyph}  {label}")));
                if button.on_hover_text("Shift-Ctrl/Cmd-S").clicked() {
                    self.toggle_stage_open_file();
                }
                // Discarding asks first, as from the Changes panel, and can be undone there.
                if !staged && widgets::icon_button(ui, icon::TRASH, tr("Discard this file's changes…"), self.idle()).clicked() {
                    self.confirm_discard(entry.clone());
                }
            }
        }
        // The open working file in the person's own editor, when one is set in Settings.
        let editor = self.settings.external_editor.trim().to_string();
        let working_file = match self.repo().map(|r| (&r.selection, r.path.clone())) {
            Some((Selection::Change { entry, .. }, root)) => Some(root.join(&entry.path)).filter(|file| file.is_file()),
            _ => None,
        };
        if let (false, Some(file)) = (editor.is_empty(), working_file) {
            let tip = tr_fmt!("Open in {editor}", editor = crate::ui::changes::editor_name(&editor));
            if widgets::icon_button(ui, icon::CODE, &tip, true).clicked() {
                if let Err(error) = crate::ui::changes::open_in_editor(&editor, &file) {
                    self.notify(error, true);
                }
            }
        }
        // The diff as Git printed it, to paste into a message or an issue.
        let patch = self
            .repo()
            .and_then(|r| r.diff.as_ref())
            .map(|diff| diff.lines.iter().map(|line| line.text.as_str()).collect::<Vec<_>>().join("\n"));
        if let Some(patch) = patch.filter(|patch| !patch.trim().is_empty()) {
            if widgets::icon_button(ui, icon::COPY, tr("Copy this diff"), true).clicked() {
                ui.ctx().copy_text(format!("{}\n", patch.trim_end()));
                self.notify(tr("Copied the diff."), false);
            }
        }
    }
}

/// How long ago the remotes were fetched, in words.
fn fetched_ago(elapsed: std::time::Duration) -> String {
    match elapsed.as_secs() / 60 {
        0 => tr("Fetched just now").to_string(),
        minutes @ 1..=59 => tr_fmt!("Fetched {minutes} min ago", minutes = minutes),
        minutes => tr_fmt!("Fetched {hours} h ago", hours = minutes / 60),
    }
}

/// The review's indices for lines chosen in the shown diff, or `None` if one has no counterpart.
fn in_review(map: &[Option<usize>], shown: &std::collections::BTreeSet<usize>) -> Option<std::collections::BTreeSet<usize>> {
    shown.iter().map(|&line| map.get(line).copied().flatten()).collect()
}

impl NiceGitApp {
    /// Stages, unstages, or (after confirming) discards one run of a file's changed lines.
    fn run_block_action(
        &mut self,
        action: crate::diff_view::BlockAction,
        lines: std::collections::BTreeSet<usize>,
        review: nicegit_core::staging::FileReview,
    ) {
        use crate::diff_view::BlockAction;
        if self.busy.is_some() {
            return;
        }
        let count = lines.len();
        match action {
            BlockAction::Stage | BlockAction::Unstage => {
                let (label, done) =
                    if action == BlockAction::Stage { (tr("Stage lines"), tr("Staged")) } else { (tr("Unstage lines"), tr("Unstaged")) };
                let name = review.path.clone();
                self.act(label, move |client, path| {
                    client.stage_lines(&lines, &review, path).map(|_| {
                        Some(tr_fmt!(
                            "{done} {lines} in {name}.",
                            done = done,
                            lines = tr_count("{count} line", "{count} lines", count),
                            name = name
                        ))
                    })
                });
            }
            BlockAction::Discard => {
                let what = if review.untracked {
                    tr_fmt!(
                        "{lines} are removed from {path}.",
                        lines = tr_count("{count} new line", "{count} new lines", count),
                        path = review.path
                    )
                } else {
                    tr_fmt!(
                        "{lines} in {path} go back to their staged version.",
                        lines = tr_count("{count} changed line", "{count} changed lines", count),
                        path = review.path
                    )
                };
                let message = tr_fmt!("{what} You can undo this from the Changes panel until the file changes again.", what = what);
                self.confirm(
                    tr("Discard this change?"),
                    message,
                    tr("Discard"),
                    crate::ui::dialogs::Pending::DiscardLines { lines, review },
                );
            }
            BlockAction::Stash => {
                // Checked here as well as by the action, so the dialog does not open for nothing.
                if self.has_unsaved_editor() {
                    self.notify(tr("Save or close the file you are editing before changing the repository."), true);
                    return;
                }
                if self.snapshot().is_some_and(|snapshot| snapshot.operation.is_some()) {
                    return;
                }
                let value = tr_fmt!("Selected lines from {path}", path = review.path);
                self.dialog = Some(crate::ui::dialogs::Dialog::input(
                    tr("Stash lines"),
                    tr("Message (optional)"),
                    &value,
                    crate::ui::dialogs::InputKind::StashLines { lines, review },
                ));
            }
        }
    }
}

/// The edge of a panel that the user drags to resize it.
#[allow(dead_code)]
enum PanelEdge {
    Left(egui::Rect),
    Right(egui::Rect),
    Top(egui::Rect),
}

/// Double-clicking a panel's edge restores its default size, as double-clicking a divider
/// does in the Mac app. egui registers the resize handle as a widget with its own id.
fn restore_on_double_click(ctx: &egui::Context, edges: &[(&str, PanelEdge)]) {
    let now = ctx.input(|i| i.time);
    for (id, _) in edges {
        let handle = crate::app::panel_id(ctx, id).with("__resize");
        let Some(response) = ctx.read_response(handle) else { continue };
        if !response.clicked() && !response.double_clicked() {
            continue;
        }
        // Two clicks on the same edge in quick succession count as a double-click.
        let last_click = egui::Id::unique(("panel_edge_click", *id));
        let previous: Option<f64> = ctx.data(|d| d.get_temp(last_click));
        if response.double_clicked() || previous.is_some_and(|at| now - at <= 0.6) {
            let panel = crate::app::panel_id(ctx, id);
            ctx.data_mut(|data| {
                data.remove::<egui::containers::panel::PanelState>(panel);
                data.remove::<f64>(last_click);
            });
            ctx.request_repaint();
        } else {
            ctx.data_mut(|data| data.insert_temp(last_click, now));
        }
    }
}
