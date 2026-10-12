//! The main window: repositories column, branches sidebar, toolbar and history in the middle,
//! and the changes panel or commit inspector on the right.

pub mod changes;
pub mod clone_picker;
pub mod dialogs;
pub mod github_section;
pub mod history;
pub mod history_columns;
pub mod inspector;
pub mod palette;
pub mod pull_request;
pub mod repositories;
pub mod settings_window;
pub mod shortcuts_window;
pub mod sidebar;
pub mod toolbar;

use egui::{Key, KeyboardShortcut, Modifiers, RichText};
use egui_phosphor::regular as icon;

use crate::app::{NiceGitApp, Selection};
use crate::settings::DiffContextSetting;
use crate::theme;
use crate::tools::widgets;

impl NiceGitApp {
    pub fn shortcuts(&mut self, ctx: &egui::Context) {
        let command = |key| KeyboardShortcut::new(Modifiers::COMMAND, key);
        let command_shift = |key| KeyboardShortcut::new(Modifiers::COMMAND | Modifiers::SHIFT, key);
        let command_alt = |key| KeyboardShortcut::new(Modifiers::COMMAND | Modifiers::ALT, key);
        if ctx.input_mut(|i| i.consume_shortcut(&command_shift(Key::P))) {
            self.toggle_palette();
        }
        if ctx.input_mut(|i| i.consume_shortcut(&command_shift(Key::F))) && self.snapshot().is_some() {
            self.open_tool(Box::new(crate::tools::commit_search::CommitSearchWindow::new()));
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
        if ctx.input_mut(|i| i.consume_shortcut(&command(Key::W))) && self.dialog.is_none() && self.palette.is_none() {
            if self.tools.last().is_some_and(|tool| tool.has_unsaved_changes()) {
                self.notify("Save or close the file you are editing first.", true);
            } else if !self.tools.is_empty() {
                self.tools.pop();
            } else if !self.repos.is_empty() {
                self.close_tab(self.active);
            }
        }
        if ctx.input_mut(|i| i.consume_shortcut(&command_shift(Key::S))) && self.showing_diff() {
            self.toggle_stage_open_file();
        }
        // Checked first: Command-Enter also matches while Shift is held.
        if ctx.input_mut(|i| i.consume_shortcut(&command_shift(Key::Enter))) {
            self.commit_and_push();
        }
        if ctx.input_mut(|i| i.consume_shortcut(&command(Key::Enter))) && self.can_commit() {
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
        // Arrow keys move through the graph when no text field has focus.
        let typing = ctx.memory(|m| m.focused().is_some());
        if !typing && self.dialog.is_none() && self.palette.is_none() {
            let (up, down, escape) =
                ctx.input(|i| (i.key_pressed(Key::ArrowUp), i.key_pressed(Key::ArrowDown), i.key_pressed(Key::Escape)));
            let diff = self.showing_diff();
            let alt = ctx.input(|i| i.modifiers.alt);
            // While a diff is open, Option (Alt) with an arrow opens the previous or next file;
            // the graph is hidden, so the arrows alone do not move through it.
            if (up || down) && diff && alt {
                self.step_diff_file(if up { -1 } else { 1 });
            } else if (up || down) && !diff {
                self.move_selection(if up { -1 } else { 1 });
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

    fn move_selection(&mut self, delta: isize) {
        let Some(repo) = self.repo() else { return };
        let count = repo.rows.len();
        if count == 0 {
            return;
        }
        let current = match &repo.selection {
            Selection::Commit { hash, .. } => (0..count).find(|&i| repo.commit_at(i).is_some_and(|c| &c.hash == hash)),
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
        match repo.commit_at(next).map(|c| c.hash.clone()) {
            Some(hash) => self.select_commit(hash),
            None => self.select_working_tree(),
        }
        if let Some(repo) = self.repo_mut() {
            repo.scroll_to_selection = true;
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
                    ui.label(RichText::new(format!("{}  Terminal", icon::TERMINAL_WINDOW)).strong());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        hide = widgets::icon_button(ui, icon::CARET_DOWN, "Hide the terminal (Ctrl-`); its shells keep running", true)
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
                    if matches!(self.repo().map(|r| &r.selection), Some(Selection::Commit { .. } | Selection::Stash { .. })) {
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
                    self.diff_panel(ui);
                });
            } else {
                self.history(ui);
            }
        });
        restore_on_double_click(ui.ctx(), &edges);
    }

    fn empty_window(&mut self, ui: &mut egui::Ui) {
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
                ui.label(RichText::new("Git was not found. Install Git, then restart NiceGit.").color(c.danger));
                return;
            }
            if loading {
                widgets::loading(ui, "Opening repository…");
                return;
            }
            if let Some(error) = error {
                ui.label(RichText::new(error).color(c.danger));
                ui.add_space(8.0);
            } else {
                ui.label(RichText::new("A fast Git client for macOS, Windows, and Linux.").color(c.muted));
                ui.add_space(14.0);
            }
            ui.horizontal(|ui| {
                let width = 420.0_f32.min(ui.available_width());
                ui.add_space((ui.available_width() - width).max(0.0) / 2.0);
                if widgets::primary_button(ui, &format!("{}  Open repository…", icon::FOLDER_OPEN), true).clicked() {
                    self.choose_folder();
                }
                if ui.button(format!("{}  Clone…", icon::DOWNLOAD_SIMPLE)).clicked() {
                    self.dialog = Some(dialogs::Dialog::clone_repository());
                }
                if ui.button(format!("{}  New repository…", icon::PLUS)).clicked() {
                    self.create_repository();
                }
            });
            let recent: Vec<_> = self.settings.recent.iter().filter(|p| p.exists()).take(6).cloned().collect();
            if !recent.is_empty() {
                ui.add_space(24.0);
                widgets::section(ui, "Recent");
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
            ui.label(RichText::new("Ctrl/Cmd-O to open · Shift-Ctrl/Cmd-P for commands").small().color(c.muted));
        });
    }

    pub fn create_repository(&mut self) {
        if let Some(folder) = crate::file_dialog::pick_folder("Choose a folder for the new repository") {
            let path = folder.clone();
            match nicegit_core::GitClient::new().initialize(&path) {
                Ok(()) => self.open(folder),
                Err(error) => self.notify(error.to_string(), true),
            }
        }
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
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    dismiss = ui.add(egui::Button::new(RichText::new(icon::X).small()).frame(false)).on_hover_text("Dismiss").clicked();
                    if action == Some(crate::app::NoticeAction::Stash) {
                        stash = ui.add_enabled(self.idle(), egui::Button::new("Stash changes…")).clicked();
                    }
                    if action == Some(crate::app::NoticeAction::SetIdentity) {
                        identity = ui.button("Set your identity…").clicked();
                    }
                    if action == Some(crate::app::NoticeAction::Integrate) {
                        pull = ui
                            .add_enabled(self.idle(), egui::Button::new("Merge or rebase…"))
                            .on_hover_text("Fetch, then choose whether to merge the remote's commits or rebase onto them")
                            .clicked();
                    }
                    if let Some(crate::app::NoticeAction::OpenUrl(url)) = &action {
                        if ui.button("Open").on_hover_text(url.as_str()).clicked() {
                            open_link = Some(url.clone());
                        }
                    }
                    ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                        // The full message, and Git's own words when the message explains them.
                        let full = match &detail {
                            Some(detail) => format!("{}\n\nGit said:\n{detail}", text.text()),
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
                        "Stash changes",
                        "Message (optional)",
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
                return;
            }
            if let Some(label) = self.busy.clone() {
                ui.spinner();
                ui.label(RichText::new(format!("{label}…")).color(c.muted));
                return;
            }
            ui.label(RichText::new(icon::GIT_BRANCH).color(c.muted));
            ui.label(RichText::new(format!("Current checkout: {}", snapshot.current_branch)).monospace().small().color(c.muted));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if let Some(head) = &snapshot.head_hash {
                    ui.label(RichText::new(nicegit_core::models::short(head)).monospace().small().color(c.muted));
                }
                // When the remotes were last fetched, or why the background fetch failed.
                if let Some(repo) = self.repo().filter(|_| !snapshot.remotes.is_empty()) {
                    if let Some(problem) = &repo.fetch_problem {
                        let plain = crate::app::remote_problem(problem).map(|plain| format!("{plain}\n\n")).unwrap_or_default();
                        ui.label(RichText::new(format!("{}  Couldn't fetch", icon::CLOUD_SLASH)).small().color(c.warning))
                            .on_hover_text(format!("The last automatic fetch failed. {plain}Git said:\n{problem}"));
                    } else if let Some(when) = repo.last_fetched {
                        ui.label(RichText::new(fetched_ago(when.elapsed())).small().color(c.muted))
                            .on_hover_text("Remotes are fetched automatically; change how often in Settings.");
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
                    "Rebase stopped to edit a commit: stage your changes, then Continue.".to_string()
                } else if conflicts.is_empty() {
                    format!("A {} is in progress. Continue when ready, or abort to return to where you started.", operation.name())
                } else {
                    format!("A {} stopped with {} conflicted file(s). Resolve each one, then continue.", operation.name(), conflicts.len())
                };
                ui.label(RichText::new(text).color(c.warning).strong());
            });
            if !conflicts.is_empty() {
                ui.horizontal_wrapped(|ui| {
                    for path in &conflicts {
                        if ui.button(format!("{}  {path}", icon::GIT_DIFF)).on_hover_text("Open the conflict editor").clicked() {
                            self.open_tool(Box::new(crate::tools::conflict::ConflictWindow::new(path.clone())));
                        }
                    }
                });
            }
            ui.horizontal(|ui| {
                let idle = self.idle();
                if widgets::primary_button(ui, "Continue", idle && conflicts.is_empty()).clicked() {
                    self.act_recording("Continue", operation.name(), move |client, path| {
                        client.continue_operation(operation, path).map(|_| None)
                    });
                }
                if ui.add_enabled(idle, egui::Button::new("Abort…")).clicked() {
                    self.confirm(
                        format!("Abort the {}?", operation.name()),
                        "Git returns the branch and files to the state before it started. Resolutions you made are lost.",
                        "Abort",
                        dialogs::Pending::Abort(operation),
                    );
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
                        ui.label(RichText::new(format!("First bad commit: {} {subject}", nicegit_core::models::short(first_bad))).strong());
                        if ui.button("Show").clicked() {
                            self.select_commit(first_bad.clone());
                        }
                    }
                    (None, Some(testing)) => {
                        let steps = status
                            .remaining_steps
                            .map(|n| format!(" · about {n} step{} left", if n == 1 { "" } else { "s" }))
                            .unwrap_or_default();
                        ui.label(
                            RichText::new(format!("Bisecting: test {} and mark it{steps}", nicegit_core::models::short(testing))).strong(),
                        );
                        for (mark, label) in [(BisectMark::Good, "Good"), (BisectMark::Bad, "Bad"), (BisectMark::Skip, "Skip")] {
                            if ui.add_enabled(idle, egui::Button::new(label)).clicked() {
                                let testing = testing.clone();
                                self.act(&format!("Mark {}", label.to_lowercase()), move |client, path| {
                                    client.mark_bisect(mark, &testing, path).map(Some)
                                });
                            }
                        }
                    }
                    (None, None) => {
                        ui.label(RichText::new("Bisecting: mark a good and a bad commit to begin.").strong());
                    }
                }
                ui.add_space(12.0);
                if ui.add_enabled(idle, egui::Button::new(format!("End bisect, return to {}", status.original_checkout))).clicked() {
                    self.act("End bisect", |client, path| client.end_bisect(path).map(|_| Some("Ended the bisect.".into())));
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
                let back = ui.button(format!("{}  History", icon::CARET_LEFT)).on_hover_text("Back to the commit history (Esc)");
                if back.clicked() {
                    self.close_diff();
                }
                // Step through the other changed files without going back to the list, when there
                // are others.
                if let Some((position, count)) = self.diff_file_position().filter(|&(_, count)| count > 1) {
                    let shortcut = if cfg!(target_os = "macos") { "Option" } else { "Alt" };
                    let previous = format!("Previous file ({shortcut}-Up)");
                    if widgets::icon_button(ui, icon::CARET_UP, &previous, position > 0).clicked() {
                        self.step_diff_file(-1);
                    }
                    let next = format!("Next file ({shortcut}-Down)");
                    if widgets::icon_button(ui, icon::CARET_DOWN, &next, position + 1 < count).clicked() {
                        self.step_diff_file(1);
                    }
                    ui.label(RichText::new(format!("{} of {count}", position + 1)).small().color(c.muted));
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
        let repo = &mut self.repos[self.active];
        repo.diff_options.split = split;
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
        let selectable = review.as_ref().is_some_and(|r| r.line_staging_unavailable.is_none()) && line_map.is_some();
        repo.diff_options.selectable = selectable;
        if !selectable {
            repo.diff_options.selected.clear();
        }
        // Each run of changed lines offers Stage and Discard, or Unstage, while hovered.
        repo.diff_options.block_buttons = review.as_ref().map(|review| {
            if review.staged {
                crate::diff_view::BlockButtons::Unstage
            } else {
                crate::diff_view::BlockButtons::StageAndDiscard
            }
        });
        let selected = repo.diff_options.selected.clone();
        if selectable && !selected.is_empty() {
            let staged = review.as_ref().is_some_and(|r| r.staged);
            egui::Frame::new().fill(c.subtle_bg).inner_margin(egui::Margin::symmetric(14, 6)).show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(format!("{} line{} selected", selected.len(), if selected.len() == 1 { "" } else { "s" }))
                            .color(c.muted),
                    );
                    let text = if staged {
                        format!("{}  Unstage selected lines", icon::MINUS_CIRCLE)
                    } else {
                        format!("{}  Stage selected lines", icon::PLUS_CIRCLE)
                    };
                    if widgets::primary_button(ui, &text, self.busy.is_none()).clicked() {
                        let lines = line_map.as_deref().and_then(|map| in_review(map, &selected));
                        if let (Some(review), Some(lines)) = (review.clone(), lines) {
                            let label = if staged { "Unstage lines" } else { "Stage lines" };
                            self.act(label, move |client, path| client.stage_lines(&lines, &review, path).map(|_| None));
                        }
                    }
                    // Unstaged lines can also be discarded, as a run's Discard button does.
                    if !staged {
                        let discard = egui::Button::new(
                            RichText::new(format!("{}  Discard selected lines", icon::ARROW_COUNTER_CLOCKWISE)).color(c.removed),
                        );
                        if ui.add_enabled(self.busy.is_none(), discard).clicked() {
                            let lines = line_map.as_deref().and_then(|map| in_review(map, &selected));
                            if let (Some(review), Some(lines)) = (review.clone(), lines) {
                                self.run_block_action(crate::diff_view::BlockAction::Discard, lines, review);
                            }
                        }
                    }
                    if ui.button("Clear").clicked() {
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
                            "Press − beside a line to unstage it, or click and drag over lines to choose several."
                        } else {
                            "Press + beside a line to stage it, or click and drag over lines to choose several."
                        })
                        .small()
                        .color(c.muted),
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
    /// The diff's view controls, laid out from the right: Split and Unified, Hide whitespace,
    /// and Edit for a working file.
    fn diff_view_controls(&mut self, ui: &mut egui::Ui, panel_width: f32, editable: Option<String>) {
        let mut split = self.settings.split_diff;
        // A diff with only one side always shows in one column.
        let one_sided = self.repo().and_then(|r| r.diff.as_ref()).is_some_and(|d| d.is_one_sided());
        let narrow = !crate::diff_view::split_fits(panel_width);
        let reason = if one_sided {
            "A new or deleted file has only one side, so it is shown in one column"
        } else {
            "Widen the diff panel to show changes side by side"
        };
        let available = !one_sided && !narrow;
        // The control shows the layout on screen: Unified whenever Split cannot apply,
        // while the saved preference waits for a diff and a panel that suit it.
        // Only Split is disabled, so the active Unified keeps full emphasis.
        let shown = split && available;
        if ui.add_enabled(available, egui::Button::selectable(shown, "Split")).on_disabled_hover_text(reason).clicked() {
            split = true;
        }
        if ui.add(egui::Button::selectable(!shown, "Unified")).clicked() && available {
            split = false;
        }
        self.settings.split_diff = split;
        let mut ignore = self.settings.ignore_whitespace;
        if crate::tools::widgets::checkbox(ui, true, &mut ignore, "Hide whitespace").changed() {
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
            .on_hover_text("How many unchanged lines to show around each change")
            .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::ComboBox, true, "Context lines"));
        if context != self.settings.diff_context {
            self.settings.diff_context = context;
            self.reload_diff();
        }
        if let Some(path) = editable {
            if ui.button(format!("{}  Edit", icon::PENCIL_SIMPLE)).on_hover_text("Edit this file in NiceGit").clicked() {
                let repo_path = self.repo().map(|r| r.path.clone()).unwrap_or_default();
                self.open_tool(Box::new(crate::tools::editor::EditorWindow::new(repo_path, path)));
            }
        }
        // Stage or unstage the whole open file without going back to the Changes panel.
        if let Some(Selection::Change { entry, staged }) = self.repo().map(|r| r.selection.clone()) {
            if entry.kind != nicegit_core::StatusKind::Conflicted {
                let (glyph, label) = if staged { (icon::MINUS_CIRCLE, "Unstage file") } else { (icon::PLUS_CIRCLE, "Stage file") };
                let button = ui.add_enabled(self.idle(), egui::Button::new(format!("{glyph}  {label}")));
                if button.on_hover_text("Shift-Ctrl/Cmd-S").clicked() {
                    self.toggle_stage_open_file();
                }
                // Discarding asks first, as from the Changes panel, and can be undone there.
                if !staged && widgets::icon_button(ui, icon::TRASH, "Discard this file's changes…", self.idle()).clicked() {
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
            let tip = format!("Open in {}", crate::ui::changes::editor_name(&editor));
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
            if widgets::icon_button(ui, icon::COPY, "Copy this diff", true).clicked() {
                ui.ctx().copy_text(format!("{}\n", patch.trim_end()));
                self.notify("Copied the diff.", false);
            }
        }
    }
}

/// How long ago the remotes were fetched, in words.
fn fetched_ago(elapsed: std::time::Duration) -> String {
    match elapsed.as_secs() / 60 {
        0 => "Fetched just now".to_string(),
        minutes @ 1..=59 => format!("Fetched {minutes} min ago"),
        minutes => format!("Fetched {} h ago", minutes / 60),
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
        let plural = if count == 1 { "" } else { "s" };
        match action {
            BlockAction::Stage | BlockAction::Unstage => {
                let (label, done) = if action == BlockAction::Stage { ("Stage lines", "Staged") } else { ("Unstage lines", "Unstaged") };
                let name = review.path.clone();
                self.act(label, move |client, path| {
                    client.stage_lines(&lines, &review, path).map(|_| Some(format!("{done} {count} line{plural} in {name}.")))
                });
            }
            BlockAction::Discard => {
                let what = if review.untracked {
                    format!("{count} new line{plural} are removed from {}.", review.path)
                } else {
                    format!("{count} changed line{plural} in {} go back to their staged version.", review.path)
                };
                let message = format!("{what} You can undo this from the Changes panel until the file changes again.");
                self.confirm("Discard this change?", message, "Discard", crate::ui::dialogs::Pending::DiscardLines { lines, review });
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
        let handle = egui::Id::new(*id).with("__resize");
        let Some(response) = ctx.read_response(handle) else { continue };
        if !response.clicked() && !response.double_clicked() {
            continue;
        }
        // Two clicks on the same edge in quick succession count as a double-click.
        let last_click = egui::Id::new(("panel_edge_click", *id));
        let previous: Option<f64> = ctx.data(|d| d.get_temp(last_click));
        if response.double_clicked() || previous.is_some_and(|at| now - at <= 0.6) {
            ctx.data_mut(|data| {
                data.remove::<egui::containers::panel::PanelState>(egui::Id::new(*id));
                data.remove::<f64>(last_click);
            });
            ctx.request_repaint();
        } else {
            ctx.data_mut(|data| data.insert_temp(last_click, now));
        }
    }
}
