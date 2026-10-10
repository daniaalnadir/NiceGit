use egui::RichText;

use crate::app::NiceGitApp;
use crate::theme::{self, Appearance, GraphPalette};
use crate::tools::widgets;

impl NiceGitApp {
    pub fn settings_window(&mut self, ctx: &egui::Context) {
        if !self.show_settings {
            return;
        }
        let mut open = true;
        let mut reload_diff = false;
        let mut show_shortcuts = false;
        egui::Window::new("Settings")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(420.0)
            .pivot(egui::Align2::CENTER_CENTER)
            .default_pos(ctx.content_rect().center())
            .show(ctx, |ui| {
                let c = theme::of(ui);
                widgets::section(ui, "Appearance");
                ui.horizontal(|ui| {
                    for (value, label) in [(Appearance::System, "System"), (Appearance::Light, "Light"), (Appearance::Dark, "Dark")] {
                        ui.selectable_value(&mut self.settings.appearance, value, label);
                    }
                });
                ui.add_space(10.0);
                widgets::section(ui, "Graph colours");
                for palette in [GraphPalette::Standard, GraphPalette::ColorBlindSafe, GraphPalette::Muted] {
                    ui.horizontal(|ui| {
                        // A fixed-width label column keeps every row's swatches at the same x.
                        let label_size = egui::vec2(150.0, ui.spacing().interact_size.y);
                        ui.allocate_ui_with_layout(label_size, egui::Layout::left_to_right(egui::Align::Center), |ui| {
                            // The space is otherwise shrunk to the label's own width.
                            ui.set_min_width(label_size.x);
                            if crate::tools::widgets::radio(ui, self.settings.graph_palette == palette, palette.title()).clicked() {
                                self.settings.graph_palette = palette;
                            }
                        });
                        let previous = theme::graph_palette();
                        theme::set_graph_palette(palette);
                        for index in 0..theme::GRAPH_COLOR_COUNT {
                            let (rect, _) = ui.allocate_exact_size(egui::vec2(14.0, 14.0), egui::Sense::hover());
                            ui.painter().circle_filled(rect.center(), 6.5, theme::graph_color(index));
                        }
                        theme::set_graph_palette(previous);
                    });
                }
                ui.add_space(10.0);
                widgets::section(ui, "Diffs");
                reload_diff |=
                    crate::tools::widgets::checkbox(ui, true, &mut self.settings.ignore_whitespace, "Hide whitespace-only changes")
                        .changed();
                crate::tools::widgets::checkbox(ui, true, &mut self.settings.split_diff, "Show diffs side by side");
                // Said plainly when the choice is not in effect at the panel's current width.
                let minimum = crate::diff_view::MIN_SPLIT_WIDTH;
                let too_narrow = self.diff_panel_width.is_some_and(|width| !crate::diff_view::split_fits(width));
                let (note, color) = if self.settings.split_diff && too_narrow {
                    (
                        format!("Not in effect now: the diff panel is narrower than {minimum} points. Widen it to see both sides."),
                        theme::of(ui).warning,
                    )
                } else {
                    (format!("Needs a diff panel at least {minimum} points wide."), theme::of(ui).muted)
                };
                // A warning is set at body size so it reads as clearly as the setting it explains.
                let note = RichText::new(note).color(color);
                ui.label(if too_narrow && self.settings.split_diff { note } else { note.small() });
                ui.add_space(10.0);
                widgets::section(ui, "Repository");
                crate::tools::widgets::checkbox(
                    ui,
                    true,
                    &mut self.settings.auto_refresh,
                    "Refresh automatically when files change outside NiceGit",
                );
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.label("Fetch from remotes");
                    for (minutes, label) in [(0, "Off"), (5, "5 min"), (15, "15 min"), (30, "30 min"), (60, "1 hour")] {
                        ui.selectable_value(&mut self.settings.auto_fetch_minutes, minutes, label);
                    }
                })
                .response
                .on_hover_text("How often the open repository is fetched in the background, so ahead and behind counts stay current");
                ui.add_space(10.0);
                widgets::section(ui, "Layout");
                if ui.button("Restore default panel widths").on_hover_text("Columns and panels return to their original sizes").clicked() {
                    ctx.data_mut(|data| {
                        for id in ["repositories", "sidebar", "changes", "diff", "terminal"] {
                            data.remove::<egui::containers::panel::PanelState>(egui::Id::new(id));
                        }
                    });
                }
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    ui.label(RichText::new(format!("NiceGit {}", env!("CARGO_PKG_VERSION"))).small().color(c.muted));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.link("Keyboard shortcuts").clicked() {
                            show_shortcuts = true;
                        }
                    });
                });
            });
        if reload_diff {
            self.reload_diff();
        }
        self.show_settings = open;
        self.show_shortcuts |= show_shortcuts;
    }
}
