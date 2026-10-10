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
            // The window title already names the file, so the path only adds information when the
            // file sits in a subfolder.
            if self.path.contains('/') {
                ui.label(RichText::new(&self.path).monospace().color(c.muted));
            }
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
        egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            // The box has a margin around the text, and a muted gutter of line numbers that
            // uses the editor's font. Long lines wrap to the window, as in the Mac app, and only
            // the first row of each line is numbered.
            egui::Frame::new()
                .fill(ui.visuals().extreme_bg_color)
                .stroke(egui::Stroke::new(1.0, c.border))
                .corner_radius(6.0)
                .inner_margin(egui::Margin::same(8))
                .show(ui, |ui| {
                    const GUTTER_SPACING: f32 = 10.0;
                    let font = egui::TextStyle::Monospace.resolve(ui.style());
                    let color = ui.visuals().text_color();
                    let digits = self.text.split('\n').count().to_string().len();
                    let digit_width = ui.ctx().fonts_mut(|fonts| fonts.glyph_width(&font, '0'));
                    // The text has what is left beside the numbers, their divider, and the gaps around it.
                    let wrap_width =
                        (ui.available_width() - digits as f32 * digit_width - 2.0 * GUTTER_SPACING - 1.0).max(20.0 * digit_width);
                    // The gutter and the editor share one layout, so the numbers follow the wrapping.
                    let layout = |ui: &egui::Ui, text: &str| {
                        let job = egui::text::LayoutJob::simple(text.to_owned(), font.clone(), color, wrap_width);
                        ui.ctx().fonts_mut(|fonts| fonts.layout_job(job))
                    };
                    let galley = layout(ui, &self.text);
                    let numbers = gutter_numbers(galley.rows.iter().map(|row| row.ends_with_newline), digits);
                    let mut layouter = |ui: &egui::Ui, text: &dyn egui::TextBuffer, _wrap_width: f32| layout(ui, text.as_str());
                    ui.horizontal_top(|ui| {
                        ui.spacing_mut().item_spacing.x = GUTTER_SPACING;
                        let gutter = RichText::new(numbers.join("\n")).monospace().color(c.muted);
                        ui.add(egui::Label::new(gutter).wrap_mode(egui::TextWrapMode::Extend));
                        ui.separator();
                        ui.add(
                            egui::TextEdit::multiline(&mut self.text)
                                .code_editor()
                                .frame(egui::Frame::NONE)
                                .desired_width(wrap_width)
                                .desired_rows(30)
                                .lock_focus(true)
                                .layouter(&mut layouter),
                        )
                        .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, "File contents"));
                    });
                });
        });
    }
}

/// The gutter's text for each row of laid-out text: a right-aligned line number on the first
/// row of each line, and nothing on the rows a long line wraps onto.
fn gutter_numbers(rows_end_lines: impl Iterator<Item = bool>, digits: usize) -> Vec<String> {
    let mut line = 0;
    let mut starts_line = true;
    rows_end_lines
        .map(|ends_line| {
            let number = if starts_line {
                line += 1;
                format!("{line:>digits$}")
            } else {
                String::new()
            };
            starts_line = ends_line;
            number
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::gutter_numbers;

    #[test]
    fn wrapped_rows_are_not_numbered() {
        // Line 1 wraps onto a second row; line 2 fits; line 3 is the empty last line.
        let rows = [false, true, true, false];
        assert_eq!(gutter_numbers(rows.into_iter(), 2), [" 1", "", " 2", " 3"]);
    }

    #[test]
    fn numbers_are_right_aligned_to_the_widest() {
        let rows = std::iter::repeat_n(true, 9).chain([false]);
        let numbers = gutter_numbers(rows, 2);
        assert_eq!(numbers.first().map(String::as_str), Some(" 1"));
        assert_eq!(numbers.last().map(String::as_str), Some("10"));
    }
}
