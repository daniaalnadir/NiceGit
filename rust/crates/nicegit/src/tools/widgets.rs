//! Small shared building blocks for tool windows, so they look alike.

use egui::{Color32, RichText, Ui};
use egui_phosphor::regular as icon;

use crate::theme;

/// A compact button showing an icon, with a tooltip.
pub fn icon_button(ui: &mut Ui, glyph: &str, tooltip: &str, enabled: bool) -> egui::Response {
    ui.add_enabled(enabled, egui::Button::new(RichText::new(glyph).size(15.0)).frame(false).min_size(egui::vec2(24.0, 24.0)))
        .on_hover_text(tooltip)
        .on_disabled_hover_text(tooltip)
}

/// A button with an icon followed by text.
pub fn labeled_button(ui: &mut Ui, glyph: &str, text: &str, enabled: bool) -> egui::Response {
    ui.add_enabled(enabled, egui::Button::new(format!("{glyph}  {text}")))
}

/// The prominent button of a dialog or form, filled with the accent colour.
pub fn primary_button(ui: &mut Ui, text: &str, enabled: bool) -> egui::Response {
    let c = theme::of(ui);
    // A disabled primary button looks like an ordinary one, so it does not invite a click.
    if !enabled {
        return ui.add_enabled(false, egui::Button::new(RichText::new(text).font(theme::strong(13.0))));
    }
    ui.add(egui::Button::new(RichText::new(text).font(theme::strong(13.0)).color(c.accent_text)).fill(c.accent))
}

/// A button for an action that discards or deletes something.
pub fn danger_button(ui: &mut Ui, text: &str, enabled: bool) -> egui::Response {
    let c = theme::of(ui);
    if !enabled {
        return ui.add_enabled(false, egui::Button::new(RichText::new(text).font(theme::strong(13.0))));
    }
    ui.add(egui::Button::new(RichText::new(text).font(theme::strong(13.0)).color(Color32::WHITE)).fill(c.danger))
}

/// An uppercase section caption.
pub fn section(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(text.to_uppercase()).small().strong().color(theme::of(ui).muted));
}

/// A centred, quiet message for an empty list or a view with nothing selected.
pub fn empty_state(ui: &mut Ui, glyph: &str, text: &str) {
    ui.vertical_centered(|ui| {
        ui.add_space(24.0);
        ui.label(RichText::new(glyph).size(28.0).color(theme::of(ui).muted));
        ui.add_space(4.0);
        ui.label(RichText::new(text).color(theme::of(ui).muted));
    });
}

/// A spinner with a short message, for data still loading.
pub fn loading(ui: &mut Ui, text: &str) {
    ui.horizontal(|ui| {
        ui.spinner();
        ui.label(RichText::new(text).color(theme::of(ui).muted));
    });
}

/// An error message in the danger colour, wrapping to the available width.
pub fn error(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(format!("{}  {text}", icon::WARNING)).color(theme::of(ui).danger));
}

/// A boxed notice, for example a warning above a destructive form.
pub fn callout(ui: &mut Ui, text: &str, warning: bool) {
    let c = theme::of(ui);
    egui::Frame::new()
        .fill(if warning { c.banner_bg } else { c.subtle_bg })
        .corner_radius(6.0)
        .inner_margin(egui::Margin::symmetric(10, 8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            let glyph = if warning { icon::WARNING } else { icon::INFO };
            ui.label(RichText::new(format!("{glyph}  {text}")).color(if warning { c.warning } else { ui.visuals().text_color() }));
        });
}

/// A rounded label for a branch, tag, or other reference.
pub fn pill(ui: &mut Ui, text: &str, fill: Color32) -> egui::Response {
    egui::Frame::new()
        .fill(fill)
        .corner_radius(4.0)
        .inner_margin(egui::Margin::symmetric(6, 1))
        .show(ui, |ui| ui.label(RichText::new(text).small().color(Color32::WHITE)))
        .response
}

/// The colour for one of Git's change letters (A, M, D, R, C, T, U).
pub fn change_letter_color(ui: &Ui, letter: &str) -> Color32 {
    let c = theme::of(ui);
    match letter.chars().next() {
        Some('A') => c.added,
        Some('D') => c.removed,
        Some('R') | Some('C') => c.renamed,
        Some('U') => c.conflict,
        _ => c.modified,
    }
}

/// A single-line text field that fills the width.
pub fn text_field(ui: &mut Ui, value: &mut String, hint: &str) -> egui::Response {
    ui.add(egui::TextEdit::singleline(value).hint_text(hint).desired_width(f32::INFINITY))
}

/// A search field with a magnifying-glass prefix.
pub fn search_field(ui: &mut Ui, value: &mut String, hint: &str) -> egui::Response {
    ui.horizontal(|ui| {
        ui.label(RichText::new(icon::MAGNIFYING_GLASS).color(theme::of(ui).muted));
        ui.add(egui::TextEdit::singleline(value).hint_text(hint).desired_width(f32::INFINITY))
    })
    .inner
}

/// A short commit ID in monospace.
pub fn hash_label(ui: &mut Ui, hash: &str) -> egui::Response {
    ui.label(RichText::new(&hash[..hash.len().min(8)]).monospace().color(theme::of(ui).muted))
}
