use egui::{RichText, Ui};
use egui_phosphor::regular as icon;
use nicegit_core::line_counts::LineCount;
use nicegit_core::models::short;
use nicegit_core::{Stash, StashFile};

use crate::app::{NiceGitApp, Selection};
use crate::ui::history::middle_ellipsis;

/// A commit's signature summary, filled in by a background thread: None while loading.
type SignatureCell = std::sync::Arc<std::sync::Mutex<Option<Option<crate::git_ext::SignatureSummary>>>>;
use crate::theme;
use crate::tools::{self, widgets};

impl NiceGitApp {
    /// The inspector for the selected commit or stash, shown in the Changes panel's place.
    pub fn inspector(&mut self, ui: &mut Ui) {
        match self.repo().map(|r| r.selection.clone()) {
            Some(Selection::Commit { .. }) => self.commit_inspector(ui),
            Some(Selection::Stash { .. }) => self.stash_inspector(ui),
            _ => {}
        }
    }

    /// The commit inspector: message, metadata, signature, and changed files.
    fn commit_inspector(&mut self, ui: &mut Ui) {
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
                            ui.label(RichText::new(body.as_str()).color(ui.visuals().text_color().gamma_multiply(0.85)));
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
                                        // Long names are shortened so each pill stays on one line; the full name shows on hover.
                                        widgets::pill(ui, &middle_ellipsis(&text, 26), fill).on_hover_text(&text);
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
                            if let Some((text, color, help)) = &signature {
                                caption(ui, "Signature");
                                let label = ui.label(RichText::new(text).color(*color));
                                if !help.is_empty() {
                                    label.on_hover_text(help);
                                }
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
                let counts = self.repos[self.active]
                    .commit_line_counts
                    .as_mut()
                    .filter(|(counted, _)| *counted == hash)
                    .and_then(|(_, task)| task.get().cloned().flatten())
                    .unwrap_or_default();
                for (status, path) in &files {
                    let selected = file.as_deref() == Some(path.as_str());
                    let response = changed_file_row(ui, status, path, selected, counts.get(path).copied(), None);
                    if response.clicked() {
                        self.select_commit_file(hash.clone(), Some(path.clone()));
                    }
                    response.context_menu(|ui| self.commit_file_menu(ui, &hash, path, status));
                }
            });
        });
    }

    /// The stash inspector: the stash's message and reference, and the files it saved. Choosing
    /// a file shows just its changes; restoring one is offered from its menu.
    fn stash_inspector(&mut self, ui: &mut Ui) {
        let c = theme::of(ui);
        let Some(Selection::Stash { hash, file }) = self.repo().map(|r| r.selection.clone()) else { return };
        let Some(snapshot) = self.snapshot().cloned() else { return };
        let Some(stash) = snapshot.stashes.iter().find(|s| s.hash == hash).cloned() else { return };
        // The files are listed for this stash only; a listing for another stash is not shown.
        let listing = self.repo().and_then(|r| r.stash_files.clone()).filter(|(listed, _)| *listed == hash).map(|(_, result)| result);

        egui::Frame::new().inner_margin(egui::Margin { left: 16, right: 14, top: 12, bottom: 8 }).show(ui, |ui| {
            ui.horizontal(|ui| {
                if ui.button(format!("{}  Changes", icon::ARROW_LEFT)).on_hover_text("Back to the Changes panel (Esc)").clicked() {
                    self.clear_selection();
                }
            });
        });
        ui.separator();
        egui::ScrollArea::vertical().id_salt("inspector").auto_shrink(false).show(ui, |ui| {
            egui::Frame::new().inner_margin(egui::Margin { left: 16, right: 14, top: 8, bottom: 12 }).show(ui, |ui| {
                ui.label(RichText::new(stash.message.as_str()).size(16.0).strong());
                ui.add_space(10.0);
                egui::Grid::new("stash_meta").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
                    let caption = |ui: &mut Ui, text: &str| {
                        ui.label(RichText::new(text).small().color(c.muted));
                    };
                    caption(ui, "Stash");
                    ui.label(RichText::new(stash.reference.as_str()).monospace());
                    ui.end_row();
                    caption(ui, "Commit");
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(short(&stash.hash)).monospace());
                        if widgets::icon_button(ui, icon::COPY, "Copy the full stash commit ID", true).clicked() {
                            ui.ctx().copy_text(stash.hash.clone());
                        }
                    });
                    ui.end_row();
                });
                ui.add_space(14.0);
                ui.horizontal(|ui| match &listing {
                    Some(Ok(files)) => widgets::section(ui, &format!("Changed files ({})", files.len())),
                    _ => widgets::section(ui, "Changed files"),
                });
                ui.add_space(2.0);
                if ui.add(egui::Button::selectable(file.is_none(), RichText::new(format!("{}  Show all changes", icon::FILES)))).clicked() {
                    self.select_stash_file(stash.clone(), None);
                }
                match &listing {
                    None => widgets::loading(ui, "Listing files…"),
                    Some(Err(error)) => widgets::error(ui, error),
                    Some(Ok(files)) => {
                        for entry in files {
                            let selected = file.as_deref() == Some(entry.path.as_str());
                            let tag = entry.untracked.then_some("untracked");
                            let response = changed_file_row(ui, &entry.status, &entry.path, selected, None, tag);
                            if response.clicked() {
                                self.select_stash_file(stash.clone(), Some(entry.path.clone()));
                            }
                            response.context_menu(|ui| self.stash_file_menu(ui, &stash, entry));
                        }
                    }
                }
            });
        });
    }

    fn stash_file_menu(&mut self, ui: &mut Ui, stash: &Stash, file: &StashFile) {
        let Some(snapshot) = self.snapshot().cloned() else { return };
        ui.set_min_width(240.0);
        // A file the stash deletes has no version in it to restore.
        if !file.status.starts_with('D') {
            let allowed = self.idle() && snapshot.operation.is_none() && !self.has_unsaved_editor() && snapshot.is_on_branch();
            if ui
                .add_enabled(allowed, egui::Button::new(format!("{}  Restore this file from the stash", icon::ARROW_COUNTER_CLOCKWISE)))
                .clicked()
            {
                ui.close();
                self.confirm(
                    format!("Restore {}?", file.path),
                    format!(
                        "This replaces {} in your working files and in the staged version with its version from {}. Its current changes are lost; other files and the branch are not changed.",
                        file.path, stash.reference
                    ),
                    "Restore",
                    crate::ui::dialogs::Pending::RestoreFromStash {
                        path: file.path.clone(),
                        stash: stash.clone(),
                        untracked: file.untracked,
                        branch: snapshot.current_branch.clone(),
                        head: snapshot.head_hash.clone(),
                    },
                );
            }
            ui.separator();
        }
        if ui.button(format!("{}  Copy path", icon::COPY)).clicked() {
            ui.close();
            ui.ctx().copy_text(file.path.clone());
        }
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
        // A file deleted in this commit does not exist at it, so there is no page to open.
        if !status.starts_with('D') {
            crate::git_ext::github_open_menu(
                ui,
                "Open file on GitHub",
                crate::git_ext::github_file_links(&snapshot, hash, path, &self.settings.github_enterprise_hosts),
                crate::git_ext::PUSHED_COMMIT_NOTE,
            );
        }
        ui.separator();
        for (before, text) in [(false, "Restore this version"), (true, "Restore version before this commit")] {
            if ui
                .add_enabled(idle && snapshot.is_on_branch(), egui::Button::new(format!("{}  {text}", icon::ARROW_COUNTER_CLOCKWISE)))
                .clicked()
            {
                ui.close();
                let source = if before { format!("{hash}^") } else { hash.to_string() };
                // Say whether the file comes back or goes away, before anything changes.
                let deleted = (status.starts_with('A') && before) || (status.starts_with('D') && !before);
                let what = if deleted {
                    format!("{path} does not exist in that version, so it will be deleted.")
                } else {
                    format!("{path} will be replaced with that version.")
                };
                self.confirm(
                    format!("Restore {path}?"),
                    format!("{what} Staged and unstaged changes to it are lost; other files and the branch are not changed."),
                    "Restore",
                    crate::ui::dialogs::Pending::RestoreFile {
                        path: path.to_string(),
                        source,
                        branch: snapshot.current_branch.clone(),
                        head: snapshot.head_hash.clone(),
                    },
                );
            }
        }
        ui.separator();
        if ui.button(format!("{}  Copy path", icon::COPY)).clicked() {
            ui.close();
            ui.ctx().copy_text(path.to_string());
        }
    }

    /// A commit's signature status, loaded once per commit and cached in egui memory.
    fn signature_for(&mut self, hash: &str, ctx: &egui::Context) -> Option<(String, egui::Color32, String)> {
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
        value.flatten().map(|summary| (summary.text, signature_color(summary.level, &c), summary.help))
    }
}

/// One changed file in an inspector's list: its change letter, then its path, with an optional
/// tag and line counts laid out from the right. A long path is shortened, not the counts.
fn changed_file_row(ui: &mut Ui, status: &str, path: &str, selected: bool, count: Option<LineCount>, tag: Option<&str>) -> egui::Response {
    let c = theme::of(ui);
    let color = widgets::change_letter_color(ui, status);
    ui.horizontal(|ui| {
        ui.label(RichText::new(status.chars().next().unwrap_or('M').to_string()).monospace().strong().color(color));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if let Some(count) = count {
                crate::ui::changes::line_count(ui, count);
            }
            if let Some(tag) = tag {
                ui.label(RichText::new(tag).small().color(c.muted));
            }
            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                ui.add(egui::Button::selectable(selected, path).truncate())
            })
            .inner
        })
        .inner
    })
    .inner
    .on_hover_text(path)
}

/// The colour for a signature's level, as the Mac app colours it: green when verified, orange
/// for an untrusted key, red when bad, and secondary when it cannot be checked.
fn signature_color(level: u8, c: &theme::Colors) -> egui::Color32 {
    match level {
        0 => c.added,
        1 => c.warning,
        2 => c.danger,
        _ => c.muted,
    }
}

#[cfg(test)]
mod tests {
    use super::signature_color;
    use crate::theme;

    #[test]
    fn signature_levels_take_the_mac_apps_colours() {
        for dark in [false, true] {
            let c = theme::colors(dark);
            assert_eq!(signature_color(0, &c), c.added, "verified is green");
            assert_eq!(signature_color(1, &c), c.warning, "untrusted is orange");
            assert_eq!(signature_color(2, &c), c.danger, "bad is red");
            assert_eq!(signature_color(3, &c), c.muted, "cannot be checked is secondary");
        }
    }
}
