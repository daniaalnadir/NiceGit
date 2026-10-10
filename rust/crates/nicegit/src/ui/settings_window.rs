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
                ui.add_space(10.0);
                widgets::section(ui, "Repository");
                crate::tools::widgets::checkbox(
                    ui,
                    true,
                    &mut self.settings.auto_refresh,
                    "Refresh automatically when files change outside NiceGit",
                );
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
                ui.label(RichText::new(format!("NiceGit {}", env!("CARGO_PKG_VERSION"))).small().color(c.muted));
            });
        if reload_diff {
            self.reload_diff();
        }
        self.show_settings = open;
    }
}
