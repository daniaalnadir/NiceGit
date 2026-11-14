//! The output of a custom action that asked to be shown: what the program printed and what it
//! wrote to its error stream, read only, with a Copy button.

use egui::RichText;

use super::{Ctx, ToolWindow};
use crate::custom_actions::Captured;
use crate::theme;

pub struct CustomOutputWindow {
    name: String,
    captured: Captured,
}

impl CustomOutputWindow {
    pub fn new(name: String, captured: Captured) -> Self {
        Self { name, captured }
    }

    /// Both streams as one text, for the Copy button.
    fn combined(&self) -> String {
        match (self.captured.stdout.is_empty(), self.captured.stderr.is_empty()) {
            (_, true) => self.captured.stdout.clone(),
            (true, false) => self.captured.stderr.clone(),
            (false, false) => format!("{}\n{}", self.captured.stdout.trim_end(), self.captured.stderr),
        }
    }
}

impl ToolWindow for CustomOutputWindow {
    fn id(&self) -> String {
        format!("custom-output:{}", self.name)
    }

    fn title(&self) -> String {
        format!("{} output", self.name)
    }

    fn default_size(&self) -> egui::Vec2 {
        egui::vec2(640.0, 420.0)
    }

    fn ui(&mut self, ui: &mut egui::Ui, _cx: &mut Ctx) {
        let c = theme::of(ui);
        ui.horizontal(|ui| {
            if ui.button("Copy").clicked() {
                ui.ctx().copy_text(self.combined());
            }
        });
        ui.separator();
        egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            if self.captured.stdout.is_empty() && self.captured.stderr.is_empty() {
                ui.label(RichText::new("The program printed nothing.").color(c.muted));
            }
            if !self.captured.stdout.is_empty() {
                ui.label(RichText::new(&self.captured.stdout).monospace());
            }
            if !self.captured.stderr.is_empty() {
                ui.add_space(8.0);
                ui.label(RichText::new("Error output").small().color(c.muted));
                ui.label(RichText::new(&self.captured.stderr).monospace().color(c.danger));
            }
        });
    }
}
