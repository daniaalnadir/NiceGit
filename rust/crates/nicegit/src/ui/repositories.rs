use std::path::{Path, PathBuf};

use egui::{RichText, Ui};
use egui_phosphor::regular as icon;

use crate::app::NiceGitApp;
use crate::theme;
use crate::tools::widgets;
use crate::ui::dialogs::Pending;

impl NiceGitApp {
    /// The leftmost column: open repository tabs.
    pub fn repositories_column(&mut self, ui: &mut Ui) {
        let c = theme::of(ui);
        egui::Frame::new().inner_margin(egui::Margin { left: 12, right: 10, top: 14, bottom: 8 }).show(ui, |ui| {
            ui.horizontal(|ui| {
                if widgets::icon_button(ui, icon::SIDEBAR_SIMPLE, "Hide repositories", true).clicked() {
                    self.settings.show_repositories = false;
                }
                ui.label(RichText::new("Repositories").strong().size(15.0));
            });
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                widgets::section(ui, "Open tabs");
                ui.label(RichText::new(self.repos.len().to_string()).small().monospace().color(c.muted));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if widgets::icon_button(ui, icon::PLUS, "Open another repository", self.busy.is_none()).clicked() {
                        self.choose_folder();
                    }
                    self.workspaces_menu(ui);
                });
            });
            ui.add_space(4.0);
            if self.repos.is_empty() {
                ui.label(RichText::new("No open repositories").small().color(c.muted));
            }
            let mut switch = None;
            let mut close = None;
            let mut tab_action: Option<(usize, TabAction)> = None;
            let tab_count = self.repos.len();
            egui::ScrollArea::vertical().id_salt("repositories").auto_shrink([false, true]).max_height(ui.available_height() * 0.6).show(
                ui,
                |ui| {
                    for (index, repo) in self.repos.iter().enumerate() {
                        let active = index == self.active;
                        let fill = if active { c.card_bg } else { egui::Color32::TRANSPARENT };
                        let frame = egui::Frame::new().fill(fill).corner_radius(8.0).inner_margin(egui::Margin::symmetric(8, 6));
                        // The tab senses clicks underneath its contents, so its close button stays clickable.
                        let response = ui
                            .scope_builder(egui::UiBuilder::new().sense(egui::Sense::click()), |ui| {
                                frame.show(ui, |ui| {
                                    // Selectable labels sense clicks and drags, so they would take the tab's clicks.
                                    ui.style_mut().interaction.selectable_labels = false;
                                    ui.set_width(ui.available_width());
                                    ui.horizontal(|ui| {
                                        let glyph = if active { icon::FOLDER_OPEN } else { icon::FOLDER };
                                        ui.label(RichText::new(glyph).size(17.0).color(if active { c.accent } else { c.muted }));
                                        ui.vertical(|ui| {
                                            ui.spacing_mut().item_spacing.y = 0.0;
                                            let name = RichText::new(repo.name());
                                            // Uncommitted changes beside the name, so work in progress shows in every
                                            // tab; the row stops short of the close button.
                                            let changes = repo.snapshot.as_ref().map_or(0, |snapshot| snapshot.status.len());
                                            let row = egui::vec2((ui.available_width() - 24.0).max(40.0), 18.0);
                                            ui.allocate_ui_with_layout(row, egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                                if changes > 0 {
                                                    let plural = if changes == 1 { "" } else { "s" };
                                                    ui.label(
                                                        RichText::new(format!("{} {changes}", icon::PENCIL_SIMPLE))
                                                            .small()
                                                            .color(c.modified),
                                                    )
                                                    .on_hover_text(format!("{changes} uncommitted change{plural}"));
                                                }
                                                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                                                    ui.add(egui::Label::new(if active { name.strong() } else { name }).truncate());
                                                });
                                            });
                                            let path = repo.path.display().to_string();
                                            ui.add(
                                                egui::Label::new(RichText::new(middle_truncate(&path, 30)).small().color(c.muted))
                                                    .truncate(),
                                            );
                                        });
                                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                            if repo.loading {
                                                ui.spinner();
                                            } else {
                                                let close_button = ui
                                                    .add(egui::Button::new(RichText::new(icon::X).color(c.muted)).frame(false))
                                                    .on_hover_text("Close tab");
                                                let name = format!("Close {}", repo.name());
                                                close_button.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &name));
                                                if close_button.clicked() {
                                                    close = Some(index);
                                                }
                                            }
                                        });
                                    });
                                })
                            })
                            .response
                            .on_hover_text(repo.path.display().to_string());
                        if response.clicked() {
                            switch = Some(index);
                        }
                        // Middle-click closes a tab, as in browsers.
                        if response.middle_clicked() {
                            close = Some(index);
                        }
                        response.context_menu(|ui| {
                            if ui.button(format!("{}  Close", icon::X)).clicked() {
                                tab_action = Some((index, TabAction::Close));
                                ui.close();
                            }
                            if ui.add_enabled(tab_count > 1, egui::Button::new(format!("{}  Close other tabs", icon::X_SQUARE))).clicked() {
                                tab_action = Some((index, TabAction::CloseOthers));
                                ui.close();
                            }
                            ui.separator();
                            if ui.button(format!("{}  Copy path", icon::COPY)).clicked() {
                                tab_action = Some((index, TabAction::CopyPath));
                                ui.close();
                            }
                            if ui.button(format!("{}  Show in file manager", icon::FOLDER_OPEN)).clicked() {
                                tab_action = Some((index, TabAction::Reveal));
                                ui.close();
                            }
                        });
                        ui.add_space(2.0);
                    }
                },
            );
            if let Some(index) = close {
                self.close_tab(index);
                return;
            }
            if let Some(index) = switch {
                self.switch_to(index);
                return;
            }
            if let Some((index, action)) = tab_action {
                match action {
                    TabAction::Close => self.close_tab(index),
                    TabAction::CloseOthers => self.close_other_tabs(index),
                    TabAction::CopyPath => {
                        if let Some(repo) = self.repos.get(index) {
                            ui.ctx().copy_text(repo.path.display().to_string());
                        }
                    }
                    TabAction::Reveal => {
                        if let Some(repo) = self.repos.get(index) {
                            crate::ui::changes::reveal(&repo.path);
                        }
                    }
                }
                return;
            }
            self.recent_repositories(ui);
        });
    }

    /// Recently opened repositories that are not open as tabs, newest first. The list collapses
    /// under its header, and its rows can be opened or forgotten; forgetting never touches a folder.
    fn recent_repositories(&mut self, ui: &mut Ui) {
        let c = theme::of(ui);
        let open_tabs: Vec<PathBuf> = self.repos.iter().map(|r| r.path.clone()).collect();
        let rows: Vec<RecentRow> = self
            .settings
            .recent
            .iter()
            .filter(|path| !open_tabs.contains(path))
            .map(|path| RecentRow::new(path, self.settings.recent_opened_at.get(path).copied()))
            .collect();
        if rows.is_empty() {
            return;
        }
        ui.add_space(10.0);
        let expanded = self.settings.recent_repositories_open;
        ui.horizontal(|ui| {
            let caret = if expanded { icon::CARET_DOWN } else { icon::CARET_RIGHT };
            let header = ui.add(
                egui::Button::new(RichText::new(format!("{caret}  RECENT REPOSITORIES")).small().strong().color(c.muted)).frame(false),
            );
            if header.clicked() {
                self.settings.recent_repositories_open = !expanded;
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(RichText::new(rows.len().to_string()).small().monospace().color(c.accent));
            });
        });
        if !expanded {
            return;
        }
        ui.add_space(2.0);
        // The footer's links sit below the list, which takes the rest of the column and scrolls.
        let footer_height = 28.0;
        let max_height = (ui.available_height() - footer_height).max(80.0);
        let picked = egui::ScrollArea::vertical()
            .id_salt("recent_repositories")
            .auto_shrink([false, true])
            .max_height(max_height)
            .show(ui, |ui| {
                let mut picked = None;
                for row in &rows {
                    if let Some(action) = recent_row(ui, row, c) {
                        picked = Some(action);
                    }
                }
                picked
            })
            .inner;
        let missing = rows.iter().filter(|row| row.missing).count();
        let mut footer = None;
        ui.horizontal(|ui| {
            let remove_missing = egui::Button::new(RichText::new("Remove missing").small().color(c.accent)).frame(false);
            if ui
                .add_enabled(missing > 0, remove_missing)
                .on_hover_text("Forget the repositories whose folders no longer exist. No folder is deleted.")
                .clicked()
            {
                footer = Some(RecentAction::RemoveMissing);
            }
            let clear = egui::Button::new(RichText::new("Clear all…").small().color(c.accent)).frame(false);
            if ui.add(clear).on_hover_text("Forget every repository in this list. No folder is deleted.").clicked() {
                footer = Some(RecentAction::ClearAll);
            }
        });
        match picked.or(footer) {
            Some(RecentAction::Open(path)) => self.open(path),
            Some(RecentAction::Forget(path)) => self.settings.forget_recent(&path),
            Some(RecentAction::CopyPath(path)) => ui.ctx().copy_text(path.display().to_string()),
            Some(RecentAction::Reveal(path)) => crate::ui::changes::reveal(&path),
            Some(RecentAction::RemoveMissing) => {
                let gone: Vec<PathBuf> = rows.iter().filter(|row| row.missing).map(|row| row.path.clone()).collect();
                self.forget_recent_paths(&gone);
            }
            Some(RecentAction::ClearAll) => {
                // The paths are captured now, so the confirmation clears exactly the rows shown.
                let paths: Vec<PathBuf> = rows.iter().map(|row| row.path.clone()).collect();
                let plural = if paths.len() == 1 { "y" } else { "ies" };
                self.confirm(
                    format!("Clear {} recent repositor{plural}?", paths.len()),
                    "They are removed from the recent list. Their folders are not deleted.",
                    "Clear",
                    Pending::ForgetRecent { paths },
                );
            }
            None => {}
        }
    }

    /// Forgets the given recent repositories, as a confirmed Clear all does, and says how many went.
    pub fn forget_recent_paths(&mut self, paths: &[PathBuf]) {
        for path in paths {
            self.settings.forget_recent(path);
        }
        let plural = if paths.len() == 1 { "y" } else { "ies" };
        self.notify(format!("Removed {} repositor{plural} from recent. Their folders are unchanged.", paths.len()), false);
    }
}

/// A remembered repository in the Recent list, with what its row shows.
struct RecentRow {
    path: PathBuf,
    name: String,
    parent: String,
    /// When it was last opened, in seconds since the Unix epoch, if that was recorded.
    opened: Option<u64>,
    /// Whether the folder is gone or is no longer a Git repository. Such a row cannot be opened.
    missing: bool,
}

impl RecentRow {
    fn new(path: &Path, opened: Option<u64>) -> Self {
        RecentRow {
            path: path.to_path_buf(),
            name: path.file_name().map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned()),
            parent: path.parent().map(|parent| parent.display().to_string()).unwrap_or_default(),
            opened,
            missing: !(path.is_dir() && path.join(".git").exists()),
        }
    }
}

/// What a click in the Recent list does, applied once the rows are drawn.
enum RecentAction {
    Open(PathBuf),
    Forget(PathBuf),
    CopyPath(PathBuf),
    Reveal(PathBuf),
    RemoveMissing,
    ClearAll,
}

/// One row of the Recent list: folder, name, parent path, and when it was opened. The × shows
/// while the pointer is over the row, and a right-click menu offers the same actions and more.
fn recent_row(ui: &mut Ui, row: &RecentRow, c: theme::Colors) -> Option<RecentAction> {
    let row_id = ui.make_persistent_id(("recent repository", &row.path));
    // Where the row was drawn last frame. Hover is tested against it, not against the row's
    // response, which stops reporting hover once the pointer moves onto the × inside it.
    let rect_id = row_id.with("rect");
    let last_rect = ui.ctx().data(|data| data.get_temp::<egui::Rect>(rect_id));
    let hovered = last_rect.is_some_and(|rect| ui.rect_contains_pointer(rect));
    let fill = if hovered && !row.missing { c.card_bg } else { egui::Color32::TRANSPARENT };
    let frame = egui::Frame::new().fill(fill).corner_radius(8.0).inner_margin(egui::Margin::symmetric(8, 6));
    let mut action = None;
    let response = ui
        .scope_builder(egui::UiBuilder::new().scope_id(row_id).sense(egui::Sense::click()), |ui| {
            frame.show(ui, |ui| {
                // Selectable labels sense clicks and drags, so they would take the row's clicks.
                ui.style_mut().interaction.selectable_labels = false;
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.label(RichText::new(icon::FOLDER).size(17.0).color(c.muted));
                    // The text stops short of the × so a long name truncates rather than pushing the × out.
                    let text_width = (ui.available_width() - 26.0).max(40.0);
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 0.0;
                        ui.set_max_width(text_width);
                        let name = if row.missing { RichText::new(&row.name).color(c.muted) } else { RichText::new(&row.name) };
                        ui.add(egui::Label::new(name).truncate());
                        let parent = middle_truncate(&row.parent, 30);
                        ui.add(egui::Label::new(RichText::new(parent).small().color(c.muted)).truncate());
                        let detail = if row.missing {
                            Some("Folder not found".to_string())
                        } else {
                            row.opened.map(|time| crate::tools::blame::relative_time(time as i64))
                        };
                        if let Some(detail) = detail {
                            ui.label(RichText::new(detail).small().color(c.muted));
                        }
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if hovered {
                            let label = format!("Remove {} from recent repositories", row.name);
                            let remove = ui
                                .add(egui::Button::new(RichText::new(icon::X).color(c.muted)).frame(false))
                                .on_hover_text("Remove from recent repositories. The folder is not deleted.");
                            remove.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &label));
                            if remove.clicked() {
                                action = Some(RecentAction::Forget(row.path.clone()));
                            }
                        }
                    });
                });
            });
        })
        .response;
    ui.ctx().data_mut(|data| data.insert_temp(rect_id, response.rect));
    let label =
        if row.missing { format!("Recent repository {}, folder not found", row.name) } else { format!("Recent repository {}", row.name) };
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, !row.missing, &label));
    let response = response.on_hover_text(row.path.display().to_string());
    // A missing folder cannot be opened, and a click on the × was handled above.
    if response.clicked() && !row.missing && action.is_none() {
        action = Some(RecentAction::Open(row.path.clone()));
    }
    response.context_menu(|ui| {
        if ui.button(format!("{}  Remove from recent repositories", icon::X)).clicked() {
            action = Some(RecentAction::Forget(row.path.clone()));
            ui.close();
        }
        if ui.button(format!("{}  Copy path", icon::COPY)).clicked() {
            action = Some(RecentAction::CopyPath(row.path.clone()));
            ui.close();
        }
        if ui.button(format!("{}  Show in file manager", icon::FOLDER_OPEN)).clicked() {
            action = Some(RecentAction::Reveal(row.path.clone()));
            ui.close();
        }
    });
    ui.add_space(2.0);
    action
}

/// What the right-click menu of a repository tab does, applied after the tabs are drawn.
#[derive(Clone, Copy)]
enum TabAction {
    Close,
    CloseOthers,
    CopyPath,
    Reveal,
}

/// Shortens a long path by replacing its middle with an ellipsis.
pub fn middle_truncate(text: &str, max: usize) -> String {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= max {
        return text.to_string();
    }
    let head = max / 2 - 1;
    let tail = max - head - 1;
    format!("{}…{}", chars[..head].iter().collect::<String>(), chars[chars.len() - tail..].iter().collect::<String>())
}
