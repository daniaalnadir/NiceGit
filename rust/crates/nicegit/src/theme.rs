//! Colours, fonts, and spacing. Every view takes its colours from here so light and dark
//! appearances and the graph palettes stay consistent.

use std::sync::atomic::{AtomicU8, Ordering};

use egui::{Color32, CornerRadius, FontFamily, FontId, Margin, Stroke, TextStyle, Visuals};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Appearance {
    #[default]
    System,
    Light,
    Dark,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum GraphPalette {
    #[default]
    Standard,
    /// Okabe–Ito colours, distinguishable with the common forms of colour blindness.
    ColorBlindSafe,
    Muted,
}

impl GraphPalette {
    pub fn title(self) -> &'static str {
        match self {
            GraphPalette::Standard => "Standard",
            GraphPalette::ColorBlindSafe => "Colour-blind safe",
            GraphPalette::Muted => "Muted",
        }
    }

    fn colors(self) -> &'static [Color32; 8] {
        match self {
            GraphPalette::Standard => &STANDARD,
            GraphPalette::ColorBlindSafe => &COLOR_BLIND_SAFE,
            GraphPalette::Muted => &MUTED,
        }
    }
}

const STANDARD: [Color32; 8] = [
    Color32::from_rgb(0x3f, 0xc1, 0x7f),
    Color32::from_rgb(0x4c, 0x8d, 0xf6),
    Color32::from_rgb(0xf0, 0x8c, 0x2e),
    Color32::from_rgb(0xb3, 0x6b, 0xe8),
    Color32::from_rgb(0xef, 0x55, 0x6f),
    Color32::from_rgb(0x22, 0xb8, 0xc8),
    Color32::from_rgb(0xd9, 0xb1, 0x2a),
    Color32::from_rgb(0x8f, 0x99, 0xad),
];

const COLOR_BLIND_SAFE: [Color32; 8] = [
    Color32::from_rgb(0x00, 0x9e, 0x73),
    Color32::from_rgb(0x00, 0x72, 0xb2),
    Color32::from_rgb(0xe6, 0x9f, 0x00),
    Color32::from_rgb(0xcc, 0x79, 0xa7),
    Color32::from_rgb(0xd5, 0x5e, 0x00),
    Color32::from_rgb(0x56, 0xb4, 0xe9),
    Color32::from_rgb(0xf0, 0xe4, 0x42),
    Color32::from_rgb(0x99, 0x99, 0x99),
];

const MUTED: [Color32; 8] = [
    Color32::from_rgb(0x6f, 0xa8, 0x8c),
    Color32::from_rgb(0x74, 0x92, 0xbd),
    Color32::from_rgb(0xc4, 0x93, 0x6b),
    Color32::from_rgb(0x9d, 0x83, 0xb8),
    Color32::from_rgb(0xbe, 0x7a, 0x85),
    Color32::from_rgb(0x6c, 0xa6, 0xad),
    Color32::from_rgb(0xb4, 0xa4, 0x6a),
    Color32::from_rgb(0x8c, 0x92, 0x9c),
];

static PALETTE: AtomicU8 = AtomicU8::new(0);

pub fn set_graph_palette(palette: GraphPalette) {
    PALETTE.store(palette as u8, Ordering::Relaxed);
}

pub fn graph_palette() -> GraphPalette {
    match PALETTE.load(Ordering::Relaxed) {
        1 => GraphPalette::ColorBlindSafe,
        2 => GraphPalette::Muted,
        _ => GraphPalette::Standard,
    }
}

/// The colour of graph line `index` in the chosen palette.
pub fn graph_color(index: usize) -> Color32 {
    let colors = graph_palette().colors();
    colors[index % colors.len()]
}

pub const GRAPH_COLOR_COUNT: usize = 8;

/// Semantic colours for the current appearance.
#[derive(Clone, Copy)]
pub struct Colors {
    pub accent: Color32,
    pub accent_text: Color32,
    pub added: Color32,
    pub removed: Color32,
    pub modified: Color32,
    pub renamed: Color32,
    pub conflict: Color32,
    pub warning: Color32,
    pub danger: Color32,
    pub muted: Color32,
    pub subtle_bg: Color32,
    pub card_bg: Color32,
    pub border: Color32,
    pub added_bg: Color32,
    pub removed_bg: Color32,
    pub added_word_bg: Color32,
    pub removed_word_bg: Color32,
    pub banner_bg: Color32,
    pub tag: Color32,
    pub local_branch: Color32,
    pub remote_branch: Color32,
    pub head: Color32,
}

pub fn colors(dark: bool) -> Colors {
    if dark {
        Colors {
            accent: Color32::from_rgb(0x3f, 0xc1, 0x7f),
            accent_text: Color32::from_rgb(0x0d, 0x1a, 0x12),
            added: Color32::from_rgb(0x4a, 0xd0, 0x8a),
            removed: Color32::from_rgb(0xf2, 0x6d, 0x7d),
            modified: Color32::from_rgb(0x6c, 0xa6, 0xff),
            renamed: Color32::from_rgb(0xc4, 0x8c, 0xf2),
            conflict: Color32::from_rgb(0xf5, 0xa5, 0x3c),
            warning: Color32::from_rgb(0xf5, 0xb8, 0x42),
            danger: Color32::from_rgb(0xf2, 0x5f, 0x6c),
            muted: Color32::from_rgb(0x8a, 0x93, 0xa6),
            subtle_bg: Color32::from_rgb(0x1d, 0x21, 0x29),
            card_bg: Color32::from_rgb(0x20, 0x24, 0x2d),
            border: Color32::from_rgb(0x2e, 0x33, 0x3e),
            added_bg: Color32::from_rgb(0x15, 0x33, 0x24),
            removed_bg: Color32::from_rgb(0x3c, 0x1c, 0x22),
            added_word_bg: Color32::from_rgb(0x1f, 0x5a, 0x3a),
            removed_word_bg: Color32::from_rgb(0x6b, 0x26, 0x30),
            banner_bg: Color32::from_rgb(0x4a, 0x37, 0x14),
            tag: Color32::from_rgb(0x8e, 0x6d, 0x18),
            local_branch: Color32::from_rgb(0x2c, 0x7a, 0x52),
            remote_branch: Color32::from_rgb(0x2f, 0x5c, 0x9e),
            head: Color32::from_rgb(0x3f, 0xc1, 0x7f),
        }
    } else {
        Colors {
            accent: Color32::from_rgb(0x1f, 0x9d, 0x5f),
            accent_text: Color32::WHITE,
            added: Color32::from_rgb(0x1a, 0x8f, 0x52),
            removed: Color32::from_rgb(0xc8, 0x33, 0x45),
            modified: Color32::from_rgb(0x2f, 0x6f, 0xd6),
            renamed: Color32::from_rgb(0x8a, 0x4f, 0xc7),
            conflict: Color32::from_rgb(0xc9, 0x74, 0x0a),
            warning: Color32::from_rgb(0xa8, 0x6b, 0x00),
            danger: Color32::from_rgb(0xc8, 0x33, 0x45),
            muted: Color32::from_rgb(0x6b, 0x72, 0x80),
            subtle_bg: Color32::from_rgb(0xf1, 0xf3, 0xf6),
            card_bg: Color32::WHITE,
            border: Color32::from_rgb(0xdc, 0xe0, 0xe6),
            added_bg: Color32::from_rgb(0xe3, 0xf6, 0xea),
            removed_bg: Color32::from_rgb(0xfc, 0xe6, 0xe9),
            added_word_bg: Color32::from_rgb(0xb5, 0xe8, 0xc8),
            removed_word_bg: Color32::from_rgb(0xf6, 0xbf, 0xc7),
            banner_bg: Color32::from_rgb(0xfd, 0xf0, 0xd5),
            tag: Color32::from_rgb(0xb0, 0x86, 0x14),
            local_branch: Color32::from_rgb(0x1f, 0x8a, 0x55),
            remote_branch: Color32::from_rgb(0x2f, 0x6f, 0xd6),
            head: Color32::from_rgb(0x1f, 0x9d, 0x5f),
        }
    }
}

/// Colours for the appearance `ui` is drawn in.
pub fn of(ui: &egui::Ui) -> Colors {
    colors(ui.visuals().dark_mode)
}

pub fn install_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    egui_phosphor::add_to_fonts(&mut fonts, egui_phosphor::Variant::Regular);
    ctx.set_fonts(fonts);
}

fn visuals(dark: bool) -> Visuals {
    let c = colors(dark);
    let mut visuals = if dark { Visuals::dark() } else { Visuals::light() };
    let (window, panel, extreme, faint, text) = if dark {
        (
            Color32::from_rgb(0x1a, 0x1d, 0x24),
            Color32::from_rgb(0x16, 0x19, 0x1f),
            Color32::from_rgb(0x10, 0x12, 0x17),
            Color32::from_rgb(0x1e, 0x22, 0x2a),
            Color32::from_rgb(0xe4, 0xe7, 0xee),
        )
    } else {
        (
            Color32::WHITE,
            Color32::from_rgb(0xf7, 0xf8, 0xfa),
            Color32::WHITE,
            Color32::from_rgb(0xf1, 0xf3, 0xf6),
            Color32::from_rgb(0x1f, 0x23, 0x2b),
        )
    };
    visuals.window_fill = window;
    visuals.panel_fill = panel;
    visuals.extreme_bg_color = extreme;
    visuals.faint_bg_color = faint;
    visuals.override_text_color = Some(text);
    visuals.window_stroke = Stroke::new(1.0, c.border);
    visuals.window_corner_radius = CornerRadius::same(10);
    visuals.menu_corner_radius = CornerRadius::same(8);
    visuals.selection.bg_fill = if dark { Color32::from_rgb(0x1f, 0x4f, 0x3a) } else { Color32::from_rgb(0xcf, 0xef, 0xdc) };
    visuals.selection.stroke = Stroke::new(1.0, text);
    visuals.hyperlink_color = c.modified;
    for widget in [
        &mut visuals.widgets.noninteractive,
        &mut visuals.widgets.inactive,
        &mut visuals.widgets.hovered,
        &mut visuals.widgets.active,
        &mut visuals.widgets.open,
    ] {
        widget.corner_radius = CornerRadius::same(6);
    }
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, c.border);
    visuals.widgets.inactive.weak_bg_fill = if dark { Color32::from_rgb(0x25, 0x2a, 0x34) } else { Color32::from_rgb(0xec, 0xee, 0xf2) };
    visuals.widgets.inactive.bg_fill = visuals.widgets.inactive.weak_bg_fill;
    visuals.widgets.hovered.weak_bg_fill = if dark { Color32::from_rgb(0x2e, 0x34, 0x40) } else { Color32::from_rgb(0xe2, 0xe6, 0xec) };
    visuals.widgets.hovered.bg_fill = visuals.widgets.hovered.weak_bg_fill;
    visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, c.border);
    visuals.widgets.active.weak_bg_fill = if dark { Color32::from_rgb(0x36, 0x3d, 0x4b) } else { Color32::from_rgb(0xd8, 0xdd, 0xe5) };
    visuals.widgets.inactive.bg_stroke = Stroke::NONE;
    visuals.striped = false;
    visuals
}

pub fn apply(ctx: &egui::Context, appearance: Appearance) {
    let theme = match appearance {
        Appearance::System => egui::ThemePreference::System,
        Appearance::Light => egui::ThemePreference::Light,
        Appearance::Dark => egui::ThemePreference::Dark,
    };
    ctx.set_theme(theme);
    ctx.set_visuals_of(egui::Theme::Dark, visuals(true));
    ctx.set_visuals_of(egui::Theme::Light, visuals(false));
    ctx.all_styles_mut(|style| {
        style.text_styles = [
            (TextStyle::Heading, FontId::new(18.0, FontFamily::Proportional)),
            (TextStyle::Body, FontId::new(13.5, FontFamily::Proportional)),
            (TextStyle::Button, FontId::new(13.5, FontFamily::Proportional)),
            (TextStyle::Small, FontId::new(11.5, FontFamily::Proportional)),
            (TextStyle::Monospace, FontId::new(12.5, FontFamily::Monospace)),
        ]
        .into();
        style.spacing.item_spacing = egui::vec2(8.0, 6.0);
        style.spacing.button_padding = egui::vec2(9.0, 4.0);
        style.spacing.window_margin = Margin::same(14);
        style.spacing.menu_margin = Margin::same(6);
        style.spacing.interact_size.y = 24.0;
        style.spacing.scroll.floating = true;
    });
}
