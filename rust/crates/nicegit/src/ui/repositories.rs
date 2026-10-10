use egui::{RichText, Ui};
use egui_phosphor::regular as icon;

use crate::app::NiceGitApp;
use crate::theme;
use crate::tools::widgets;

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
                                                close_button
                                                    .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &name));
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
            // Recently opened repositories that are not open as tabs.
            let open: Vec<_> = self.repos.iter().map(|r| r.path.clone()).collect();
            let recent: Vec<_> = self.settings.recent.iter().filter(|p| !open.contains(p) && p.exists()).take(8).cloned().collect();
            if !recent.is_empty() {
                ui.add_space(10.0);
                widgets::section(ui, "Recent");
                ui.add_space(2.0);
                for path in recent {
                    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    let response = ui
                        .add(
                            egui::Button::new(RichText::new(format!("{}  {name}", icon::CLOCK_COUNTER_CLOCKWISE)).color(c.muted))
                                .frame(false),
                        )
                        .on_hover_text(path.display().to_string());
                    if response.clicked() {
                        self.open(path);
                    }
                }
            }
        });
    }
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
