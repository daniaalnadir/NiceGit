use egui::{RichText, Ui};
use egui_phosphor::regular as icon;
use nicegit_core::models::short;

use crate::app::{NiceGitApp, Selection};

/// A commit's signature summary, filled in by a background thread: None while loading.
type SignatureCell = std::sync::Arc<std::sync::Mutex<Option<Option<(String, u8)>>>>;
use crate::theme;
use crate::tools::{self, widgets};

impl NiceGitApp {
    /// The commit inspector: message, metadata, signature, and changed files.
    pub fn inspector(&mut self, ui: &mut Ui) {
        let c = theme::of(ui);
        let Some(Selection::Commit { hash, file }) = self.repo().map(|r| r.selection.clone()) else { return };
        let Some(snapshot) = self.snapshot().cloned() else { return };
        let summary = snapshot.commits.iter().find(|c| c.hash == hash).cloned();
        let details = self.repos[self.active].details.as_mut().and_then(|task| task.get().map(|r| r.clone().map_err(|e| e.to_string())));
        let files = self.repo().map(|r| r.commit_files.clone()).unwrap_or_default();
        let signature = self.signature_for(&hash, ui.ctx());

        egui::Frame::new().inner_margin(egui::Margin { left: 16, right: 14, top: 12, bottom: 8 }).show(ui, |ui| {
            ui.horizontal(|ui| {
                if ui.button(format!("{}  Changes", icon::ARROW_LEFT)).on_hover_text("Back to the Changes panel (Esc)").clicked() {
                    self.clear_selection();
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if let Some(commit) = &summary {
                        let commit = commit.clone();
                        let snapshot = snapshot.clone();
                        ui.menu_button(RichText::new(icon::DOTS_THREE).size(16.0), |ui| {
                            let can_switch = self.idle() && snapshot.operation.is_none();
                            self.commit_menu(ui, &commit, &snapshot, can_switch);
                        });
                    }
                });
            });
        });
        ui.separator();
        egui::ScrollArea::vertical().id_salt("inspector").auto_shrink(false).show(ui, |ui| {
            egui::Frame::new().inner_margin(egui::Margin { left: 16, right: 14, top: 8, bottom: 12 }).show(ui, |ui| {
                match &details {
                    None => widgets::loading(ui, "Loading commit…"),
                    Some(Err(error)) => widgets::error(ui, error),
                    Some(Ok(details)) => {
                        let (subject, body) = details
                            .message
                            .split_once('\n')
                            .map(|(s, b)| (s.to_string(), b.trim().to_string()))
                            .unwrap_or((details.message.clone(), String::new()));
                        ui.label(RichText::new(subject).size(16.0).strong());
                        if !body.is_empty() {
                            ui.add_space(4.0);
                            ui.label(RichText::new(body).color(ui.visuals().text_color().gamma_multiply(0.85)));
                        }
                        ui.add_space(10.0);
                        if let Some(commit) = &summary {
                            if !commit.refs.is_empty() {
                                ui.horizontal_wrapped(|ui| {
                                    for reference in &commit.refs {
                                        let (text, fill) = match reference.strip_prefix("tag: ") {
                                            Some(tag) => (format!("{}  {tag}", icon::TAG), c.tag),
                                            None => (
                                                reference.trim_start_matches("HEAD -> ").to_string(),
                                                if reference.contains('/') { c.remote_branch } else { c.local_branch },
                                            ),
                                        };
                                        widgets::pill(ui, &text, fill);
                                    }
                                });
                                ui.add_space(8.0);
                            }
                        }
                        egui::Grid::new("commit_meta").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
                            let caption = |ui: &mut Ui, text: &str| {
                                ui.label(RichText::new(text).small().color(c.muted));
                            };
                            caption(ui, "Author");
                            ui.vertical(|ui| {
                                ui.spacing_mut().item_spacing.y = 0.0;
                                ui.label(&details.author);
                                ui.label(RichText::new(&details.author_email).small().color(c.muted));
                            });
                            ui.end_row();
                            caption(ui, "Date");
                            ui.label(&details.author_date);
                            ui.end_row();
                            if details.committer != details.author {
                                caption(ui, "Committer");
                                ui.label(format!("{} · {}", details.committer, details.committer_date));
                                ui.end_row();
                            }
                            caption(ui, "Commit");
                            ui.horizontal(|ui| {
                                ui.label(RichText::new(short(&details.hash)).monospace());
                                if widgets::icon_button(ui, icon::COPY, "Copy the full commit ID", true).clicked() {
                                    ui.ctx().copy_text(details.hash.clone());
                                }
                            });
                            ui.end_row();
                            if !details.parents.is_empty() {
                                caption(ui, if details.parents.len() > 1 { "Parents" } else { "Parent" });
                                ui.horizontal(|ui| {
                                    for parent in &details.parents {
                                        if ui.link(RichText::new(short(parent)).monospace()).clicked() {
                                            self.select_commit(parent.clone());
                                        }
                                    }
                                });
                                ui.end_row();
                            }
                            if let Some((text, color)) = &signature {
                                caption(ui, "Signature");
                                ui.label(RichText::new(text).color(*color));
                                ui.end_row();
                            }
                        });
                    }
                }
                ui.add_space(14.0);
                ui.horizontal(|ui| {
                    widgets::section(ui, &format!("Changed files ({})", files.len()));
                });
                ui.add_space(2.0);
                let all_selected = file.is_none();
                if ui.add(egui::Button::selectable(all_selected, RichText::new(format!("{}  All files", icon::FILES)))).clicked() {
                    self.select_commit_file(hash.clone(), None);
                }
                for (status, path) in &files {
                    let selected = file.as_deref() == Some(path.as_str());
                    let color = widgets::change_letter_color(ui, status);
                    let response = ui
                        .horizontal(|ui| {
                            ui.label(RichText::new(status.chars().next().unwrap_or('M').to_string()).monospace().strong().color(color));
                            ui.add(egui::Button::selectable(selected, path.as_str()).truncate())
                        })
                        .inner
                        .on_hover_text(path);
                    if response.clicked() {
                        self.select_commit_file(hash.clone(), Some(path.clone()));
                    }
                    response.context_menu(|ui| self.commit_file_menu(ui, &hash, path, status));
                }
            });
        });
    }

    fn commit_file_menu(&mut self, ui: &mut Ui, hash: &str, path: &str, status: &str) {
        let Some(snapshot) = self.snapshot().cloned() else { return };
        let idle = self.idle() && snapshot.operation.is_none();
        ui.set_min_width(240.0);
        if ui.button(format!("{}  File history", icon::CLOCK_COUNTER_CLOCKWISE)).clicked() {
            ui.close();
            self.open_tool(Box::new(tools::file_history::FileHistoryWindow::new(path.to_string())));
        }
        if !status.starts_with('D') && ui.button(format!("{}  Blame at this commit", icon::USER_LIST)).clicked() {
            ui.close();
            self.open_tool(Box::new(tools::blame::BlameWindow::new(path.to_string(), Some(hash.to_string()))));
        }
        ui.separator();
        for (before, text) in [(false, "Restore this version"), (true, "Restore version before this commit")] {
            if ui
                .add_enabled(idle && snapshot.is_on_branch(), egui::Button::new(format!("{}  {text}", icon::ARROW_COUNTER_CLOCKWISE)))
                .clicked()
            {
                ui.close();
                let source = if before { format!("{hash}^") } else { hash.to_string() };
                let (path, branch, head) = (path.to_string(), snapshot.current_branch.clone(), snapshot.head_hash.clone());
                self.act("Restore file", move |client, repo| {
                    client
                        .restore(&path, &source, &branch, head.as_deref(), repo)
                        .map(|_| Some(format!("Restored {path}. Review and commit the staged change.")))
                });
            }
        }
        ui.separator();
        if ui.button(format!("{}  Copy path", icon::COPY)).clicked() {
            ui.close();
            ui.ctx().copy_text(path.to_string());
        }
    }

    /// A commit's signature status, loaded once per commit and cached in egui memory.
    fn signature_for(&mut self, hash: &str, ctx: &egui::Context) -> Option<(String, egui::Color32)> {
        let id = egui::Id::new(("signature", hash.to_string()));
        let repo = self.repo()?.path.clone();
        let dark = ctx.theme() == egui::Theme::Dark;
        let c = theme::colors(dark);
        let cached: Option<SignatureCell> = ctx.data(|d| d.get_temp(id));
        let cell = match cached {
            Some(cell) => cell,
            None => {
                let cell = std::sync::Arc::new(std::sync::Mutex::new(None));
                ctx.data_mut(|d| d.insert_temp(id, cell.clone()));
                let (writer, hash, ctx2) = (cell.clone(), hash.to_string(), ctx.clone());
                std::thread::spawn(move || {
                    let status = crate::git_ext::signature_summary(&hash, &repo);
                    *writer.lock().unwrap_or_else(|p| p.into_inner()) = Some(status);
                    ctx2.request_repaint();
                });
                cell
            }
        };
        let value = cell.lock().unwrap_or_else(|p| p.into_inner()).clone();
        value.flatten().map(|(text, level)| {
            let color = match level {
                0 => c.added,
                1 => c.warning,
                2 => c.danger,
                _ => c.muted,
            };
            (text, color)
        })
    }
}
