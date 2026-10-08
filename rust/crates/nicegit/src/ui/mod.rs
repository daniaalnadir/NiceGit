//! The main window: repositories column, branches sidebar, toolbar and history in the middle,
//! and the changes panel or commit inspector on the right.

pub mod changes;
pub mod dialogs;
pub mod history;
pub mod inspector;
pub mod palette;
pub mod repositories;
pub mod settings_window;
pub mod sidebar;
pub mod toolbar;

use egui::{Key, KeyboardShortcut, Modifiers, RichText};
use egui_phosphor::regular as icon;

use crate::app::{NiceGitApp, Selection};
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
        if ctx.input_mut(|i| i.consume_shortcut(&command(Key::Comma))) {
            self.show_settings = !self.show_settings;
        }
        if ctx.input_mut(|i| i.consume_shortcut(&command(Key::Enter))) && self.can_commit() {
            self.commit();
        }
        if ctx.input_mut(|i| i.consume_shortcut(&KeyboardShortcut::new(Modifiers::CTRL, Key::Backtick))) {
            self.toggle_terminal(ctx);
        }
        // Arrow keys move through the graph when no text field has focus.
        let typing = ctx.memory(|m| m.focused().is_some());
        if !typing && self.dialog.is_none() && self.palette.is_none() {
            let (up, down, escape) =
                ctx.input(|i| (i.key_pressed(Key::ArrowUp), i.key_pressed(Key::ArrowDown), i.key_pressed(Key::Escape)));
            if up || down {
                self.move_selection(if up { -1 } else { 1 });
            }
            if escape {
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

    pub fn toggle_terminal(&mut self, ctx: &egui::Context) {
        if let Some(index) = self.tools.iter().position(|t| t.id() == "terminal") {
            self.tools.remove(index);
        } else if let Some(repo) = self.repo() {
            let path = repo.path.clone();
            self.open_tool(Box::new(crate::tools::terminal::TerminalWindow::new(ctx, path)));
        }
    }

    pub fn layout(&mut self, ui: &mut egui::Ui) {
        let c = theme::of(ui);
        let panel = egui::Frame::new().fill(ui.visuals().panel_fill).inner_margin(egui::Margin::ZERO);
        if self.settings.show_repositories {
            egui::Panel::left("repositories")
                .resizable(true)
                .default_size(220.0)
                .size_range(170.0..=320.0)
                .frame(panel.fill(c.subtle_bg))
                .show(ui, |ui| self.repositories_column(ui));
        }
        if self.snapshot().is_some() {
            egui::Panel::left("sidebar")
                .resizable(true)
                .default_size(260.0)
                .size_range(200.0..=440.0)
                .frame(panel)
                .show(ui, |ui| self.sidebar(ui));
            egui::Panel::right("changes").resizable(true).default_size(400.0).size_range(300.0..=640.0).frame(panel).show(ui, |ui| {
                if matches!(self.repo().map(|r| &r.selection), Some(Selection::Commit { .. })) {
                    self.inspector(ui);
                } else {
                    self.changes_panel(ui);
                }
            });
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
            let showing_diff = self.repo().is_some_and(|r| {
                matches!(r.selection, Selection::Change { .. } | Selection::Stash { .. } | Selection::Commit { file: Some(_), .. })
            });
            if showing_diff {
                egui::Panel::bottom("diff")
                    .resizable(true)
                    .default_size(340.0)
                    .size_range(140.0..=900.0)
                    .frame(egui::Frame::new().fill(ui.visuals().extreme_bg_color).inner_margin(egui::Margin::symmetric(0, 0)))
                    .show(ui, |ui| self.diff_panel(ui));
            }
            self.operation_banner(ui);
            self.history(ui);
        });
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
        if let Some(folder) = rfd::FileDialog::new().set_title("Choose a folder for the new repository").pick_folder() {
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
                let color = if notice.is_error { c.danger } else { c.accent };
                let glyph = if notice.is_error { icon::WARNING_CIRCLE } else { icon::CHECK_CIRCLE };
                ui.label(RichText::new(glyph).color(color));
                ui.add(
                    egui::Label::new(RichText::new(&notice.text).color(if notice.is_error { c.danger } else { ui.visuals().text_color() }))
                        .truncate(),
                );
                if ui.add(egui::Button::new(RichText::new(icon::X).small()).frame(false)).on_hover_text("Dismiss").clicked() {
                    self.notice = None;
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
                let text = if conflicts.is_empty() {
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

    fn diff_panel(&mut self, ui: &mut egui::Ui) {
        let c = theme::of(ui);
        let Some(repo) = self.repo() else { return };
        let title = repo.diff.as_ref().map(|d| d.title.clone()).unwrap_or_default();
        let loading = repo.diff_loading;
        let editable = match &repo.selection {
            Selection::Change { entry, staged: false } if entry.kind != nicegit_core::StatusKind::Deleted => Some(entry.path.clone()),
            _ => None,
        };
        egui::Frame::new().fill(ui.visuals().panel_fill).inner_margin(egui::Margin::symmetric(14, 7)).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new(icon::GIT_DIFF).color(c.muted));
                ui.add(egui::Label::new(RichText::new(&title).strong()).truncate());
                if loading {
                    ui.spinner();
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if widgets::icon_button(ui, icon::X, "Close diff", true).clicked() {
                        self.clear_selection();
                    }
                    let mut split = self.settings.split_diff;
                    ui.selectable_value(&mut split, true, "Split");
                    ui.selectable_value(&mut split, false, "Unified");
                    self.settings.split_diff = split;
                    let mut ignore = self.settings.ignore_whitespace;
                    if ui.checkbox(&mut ignore, "Hide whitespace").changed() {
                        self.settings.ignore_whitespace = ignore;
                        self.reload_diff();
                    }
                    if let Some(path) = editable {
                        if ui.button(format!("{}  Edit", icon::PENCIL_SIMPLE)).on_hover_text("Edit this file in NiceGit").clicked() {
                            let repo_path = self.repo().map(|r| r.path.clone()).unwrap_or_default();
                            self.open_tool(Box::new(crate::tools::editor::EditorWindow::new(repo_path, path)));
                        }
                    }
                });
            });
        });
        ui.separator();
        let split = self.settings.split_diff;
        let repo = &mut self.repos[self.active];
        repo.diff_options.split = split;
        // Lines can be staged one by one once the review has loaded and matches the diff shown.
        let review = repo.review.as_mut().and_then(|task| task.get()).and_then(|r| r.as_ref().ok()).cloned();
        let selectable =
            review.as_ref().is_some_and(|r| r.line_staging_unavailable.is_none() && repo.diff.as_ref().is_some_and(|d| d.lines == r.lines));
        repo.diff_options.selectable = selectable;
        if !selectable {
            repo.diff_options.selected.clear();
        }
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
                        if let Some(review) = review.clone() {
                            let label = if staged { "Unstage lines" } else { "Stage lines" };
                            self.act(label, move |client, path| client.stage_lines(&selected, &review, path).map(|_| None));
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
                ui.label(RichText::new("Click or drag over changed lines to stage just those lines.").small().color(c.muted));
            });
        }
        let repo = &mut self.repos[self.active];
        if let Some(diff) = repo.diff.as_ref() {
            crate::diff_view::show_with(ui, diff, &mut repo.diff_options);
        } else {
            ui.take_available_space();
        }
    }
}
