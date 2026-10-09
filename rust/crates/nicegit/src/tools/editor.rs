//! A simple editor for working files. While it holds unsaved text, the app refuses Git
//! operations that could rewrite working files, and it will not overwrite a file that
//! changed on disk since it was opened.

use std::path::PathBuf;
use std::time::SystemTime;

use egui::RichText;
use egui_phosphor::regular as icon;

use crate::theme;
use crate::tools::{widgets, Ctx, ToolWindow};

pub struct EditorWindow {
    root: PathBuf,
    path: String,
    text: String,
    saved: String,
    modified: Option<SystemTime>,
    error: Option<String>,
    loaded: bool,
    close: bool,
}

impl EditorWindow {
    pub fn new(root: PathBuf, path: String) -> Self {
        let mut editor =
            Self { root, path, text: String::new(), saved: String::new(), modified: None, error: None, loaded: false, close: false };
        editor.load();
        editor
    }

    fn file(&self) -> PathBuf {
        self.path.split('/').fold(self.root.clone(), |path, part| path.join(part))
    }

    fn load(&mut self) {
        let file = self.file();
        match std::fs::read(&file) {
            Ok(bytes) => match String::from_utf8(bytes) {
                Ok(text) => {
                    self.text = text.clone();
                    self.saved = text;
                    self.modified = std::fs::metadata(&file).and_then(|m| m.modified()).ok();
                    self.error = None;
                    self.loaded = true;
                }
                Err(_) => self.error = Some("This file is not UTF-8 text, so it cannot be edited here.".into()),
            },
            Err(error) => self.error = Some(error.to_string()),
        }
    }

    fn save(&mut self) -> Result<(), String> {
        let file = self.file();
        let current = std::fs::metadata(&file).and_then(|m| m.modified()).ok();
        if current != self.modified {
            return Err("The file changed on disk since you opened it. Reload it, then reapply your edits.".into());
        }
        std::fs::write(&file, &self.text).map_err(|e| e.to_string())?;
        self.saved = self.text.clone();
        self.modified = std::fs::metadata(&file).and_then(|m| m.modified()).ok();
        Ok(())
    }
}

impl ToolWindow for EditorWindow {
    fn id(&self) -> String {
        format!("editor:{}", self.path)
    }

    fn title(&self) -> String {
        let name = self.path.rsplit('/').next().unwrap_or(&self.path);
        if self.has_unsaved_changes() {
            format!("{name} — edited")
        } else {
            name.to_string()
        }
    }

    fn default_size(&self) -> egui::Vec2 {
        egui::vec2(820.0, 620.0)
    }

    fn has_unsaved_changes(&self) -> bool {
        self.loaded && self.text != self.saved
    }

    fn wants_close(&self) -> bool {
        self.close
    }

    fn ui(&mut self, ui: &mut egui::Ui, cx: &mut Ctx) {
        let c = theme::of(ui);
        ui.horizontal(|ui| {
            ui.label(RichText::new(&self.path).monospace().color(c.muted));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let dirty = self.has_unsaved_changes();
                let save_shortcut =
                    ui.input_mut(|i| i.consume_shortcut(&egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::S)));
                if widgets::primary_button(ui, &format!("{}  Save", icon::FLOPPY_DISK), dirty).clicked() || (save_shortcut && dirty) {
                    match self.save() {
                        Ok(()) => {
                            cx.notice(format!("Saved {}.", self.path), false);
                            cx.refresh();
                        }
                        Err(error) => self.error = Some(error),
                    }
                }
                if ui.add_enabled(dirty, egui::Button::new("Revert")).on_hover_text("Discard your edits and reload the file").clicked() {
                    self.load();
                }
                if ui.button("Close").clicked() && !dirty {
                    self.close = true;
                }
            });
        });
        if let Some(error) = &self.error {
            widgets::error(ui, error);
        }
        ui.separator();
        if !self.loaded {
            return;
        }
        egui::ScrollArea::both().auto_shrink(false).show(ui, |ui| {
            // The box has a margin around the text, and a muted gutter of line numbers that
            // uses the editor's font, so each number sits beside its line.
            egui::Frame::new()
                .fill(ui.visuals().extreme_bg_color)
                .stroke(egui::Stroke::new(1.0, c.border))
                .corner_radius(6.0)
                .inner_margin(egui::Margin::same(8))
                .show(ui, |ui| {
                    let lines = self.text.split('\n').count();
                    let digits = lines.to_string().len();
                    let numbers: Vec<String> = (1..=lines).map(|line| format!("{line:>digits$}")).collect();
                    ui.horizontal_top(|ui| {
                        ui.spacing_mut().item_spacing.x = 10.0;
                        let gutter = RichText::new(numbers.join("\n")).monospace().color(c.muted);
                        ui.add(egui::Label::new(gutter).wrap_mode(egui::TextWrapMode::Extend));
                        ui.add(
                            egui::TextEdit::multiline(&mut self.text)
                                .code_editor()
                                .frame(egui::Frame::NONE)
                                .desired_width(f32::INFINITY)
                                .desired_rows(30)
                                .lock_focus(true),
                        );
                    });
                });
        });
    }
}
