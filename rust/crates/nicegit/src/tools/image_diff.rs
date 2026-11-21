//! The ways to compare the two versions of a changed image: side by side (in `compare`), swiped
//! with a divider, faded over each other, or with the changed pixels highlighted. Every view
//! draws the textures the pair already has, so switching modes uploads nothing new, except the
//! difference picture, which is made once per pair.

use egui::{
    Color32, ColorImage, CursorIcon, Painter, Pos2, Rect, Response, RichText, Sense, Shape, Stroke, TextureOptions, Ui, Vec2, WidgetInfo,
};
use serde::{Deserialize, Serialize};

use crate::i18n::{tr, tr_fmt};
use crate::theme;
use crate::tools::compare::{ImagePair, Textures};

/// How the two versions of an image are compared.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ImageDiffMode {
    /// The two versions next to each other, each at its own size.
    #[default]
    SideBySide,
    /// Both versions in one place, split by a divider the person drags.
    Swipe,
    /// The new version drawn over the old one, with adjustable opacity.
    Onion,
    /// The pixels that changed, highlighted.
    Difference,
}

impl ImageDiffMode {
    pub const ALL: [ImageDiffMode; 4] = [ImageDiffMode::SideBySide, ImageDiffMode::Swipe, ImageDiffMode::Onion, ImageDiffMode::Difference];

    fn label(self) -> &'static str {
        match self {
            ImageDiffMode::SideBySide => tr("Side by side"),
            ImageDiffMode::Swipe => tr("Swipe"),
            ImageDiffMode::Onion => tr("Onion skin"),
            ImageDiffMode::Difference => tr("Difference"),
        }
    }

    fn hint(self) -> &'static str {
        match self {
            ImageDiffMode::SideBySide => tr("Show both versions next to each other"),
            ImageDiffMode::Swipe => tr("Drag the divider to compare the two versions"),
            ImageDiffMode::Onion => tr("Fade the new version over the old one"),
            ImageDiffMode::Difference => tr("Highlight the pixels that changed"),
        }
    }
}

/// The positions of the swipe divider and the onion skin opacity, kept per window.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ImageControls {
    /// The divider's place across the canvas, from 0 (all new) to 1 (all old).
    pub split: f32,
    /// The new version's opacity in onion skin, from 0 to 1.
    pub opacity: f32,
}

impl Default for ImageControls {
    fn default() -> Self {
        Self { split: 0.5, opacity: 0.5 }
    }
}

/// The pixels that differ between two images of the same size.
pub(crate) struct Difference {
    /// The picture to draw: changed pixels in magenta, the others as the old picture in grey.
    pub image: ColorImage,
    pub changed: usize,
    pub total: usize,
}

impl Difference {
    /// The share of pixels that changed, from 0 to 100.
    pub fn percent(&self) -> f64 {
        if self.total == 0 {
            return 0.0;
        }
        self.changed as f64 * 100.0 / self.total as f64
    }
}

/// A channel must differ by more than this to count as a changed pixel, so compression noise
/// and rounding do not light up the picture.
const CHANGE_THRESHOLD: u8 = 8;

/// The largest scale a canvas is drawn at, so small images are not blown up past 2x.
const MAX_SCALE: f32 = 2.0;

/// Space kept below the canvas for the opacity slider and the caption.
const RESERVED_BELOW: f32 = 60.0;

/// The smallest canvas drawn, so a cramped panel still shows something.
const MIN_CANVAS: f32 = 40.0;

const FULL_UV: Rect = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));

/// Compares two images pixel by pixel. A pixel counts as changed when any channel differs by more
/// than [`CHANGE_THRESHOLD`]. Returns `None` when the sizes differ.
pub(crate) fn compare_images(old: &ColorImage, new: &ColorImage) -> Option<Difference> {
    if old.size != new.size {
        return None;
    }
    let mut changed = 0;
    let mut pixels = Vec::with_capacity(old.pixels.len());
    for (before, after) in old.pixels.iter().zip(&new.pixels) {
        if differs(*before, *after) {
            changed += 1;
            pixels.push(Color32::from_rgb(255, 0, 255));
        } else {
            // The old pixel in grey at a quarter opacity, so the picture reads as context.
            let [red, green, blue, _] = before.to_srgba_unmultiplied();
            let grey = (u32::from(red) * 299 + u32::from(green) * 587 + u32::from(blue) * 114) / 1000;
            pixels.push(Color32::from_rgba_unmultiplied(grey as u8, grey as u8, grey as u8, 64));
        }
    }
    let image = ColorImage { size: old.size, source_size: Vec2::new(old.size[0] as f32, old.size[1] as f32), pixels };
    Some(Difference { image, changed, total: old.pixels.len() })
}

/// Whether two pixels differ by more than the threshold in any channel.
fn differs(before: Color32, after: Color32) -> bool {
    before.to_array().iter().zip(after.to_array()).any(|(a, b)| a.abs_diff(b) > CHANGE_THRESHOLD)
}

/// The scale that fits `canvas` into `available`, keeping its shape and never above 2x.
pub(crate) fn fit_scale(canvas: Vec2, available: Vec2) -> f32 {
    if canvas.x <= 0.0 || canvas.y <= 0.0 {
        return 1.0;
    }
    (available.x / canvas.x).min(available.y / canvas.y).min(MAX_SCALE)
}

/// The part of an image that shows on one side of the swipe divider, as the rect it fills on the
/// canvas and the part of the image to draw there (in uv coordinates). `before` is the left side,
/// up to the divider; otherwise the right side, from the divider on.
pub(crate) fn swipe_piece(image: Rect, divider: f32, before: bool) -> Option<(Rect, Rect)> {
    let width = image.width();
    if width <= 0.0 {
        return None;
    }
    let (left, right) = if before { (image.left(), divider.min(image.right())) } else { (divider.max(image.left()), image.right()) };
    if right <= left {
        return None;
    }
    let u_left = ((left - image.left()) / width).clamp(0.0, 1.0);
    let u_right = ((right - image.left()) / width).clamp(0.0, 1.0);
    let dest = Rect::from_min_max(Pos2::new(left, image.top()), Pos2::new(right, image.bottom()));
    Some((dest, Rect::from_min_max(Pos2::new(u_left, 0.0), Pos2::new(u_right, 1.0))))
}

/// The sizes of both versions, when both exist and decode. Only such a pair can be compared.
fn decoded_sizes(pair: &ImagePair) -> Option<([usize; 2], [usize; 2])> {
    let size = |side: &Option<crate::tools::compare::ImageSide>| side.as_ref().and_then(|side| side.image.as_ref()).map(|image| image.size);
    Some((size(&pair.before)?, size(&pair.after)?))
}

/// The mode a pair is drawn in: the wanted one when the pair supports it, otherwise side by side.
/// The difference needs both versions at one size.
pub(crate) fn effective_mode(pair: &ImagePair, wanted: ImageDiffMode) -> ImageDiffMode {
    match decoded_sizes(pair) {
        None => ImageDiffMode::SideBySide,
        Some((before, after)) if wanted == ImageDiffMode::Difference && before != after => ImageDiffMode::SideBySide,
        Some(_) => wanted,
    }
}

/// Draws the segmented control and returns the mode the person picked this frame. The control is
/// shown only when the pair offers more than one mode.
pub(crate) fn mode_control(ui: &mut Ui, pair: &ImagePair, current: ImageDiffMode) -> Option<ImageDiffMode> {
    let (before, after) = decoded_sizes(pair)?;
    let mut chosen = None;
    ui.horizontal_wrapped(|ui| {
        for mode in ImageDiffMode::ALL {
            let button = egui::Button::selectable(current == mode, mode.label());
            let response = if mode == ImageDiffMode::Difference && before != after {
                ui.add_enabled(false, button).on_disabled_hover_text(tr("The images are different sizes"))
            } else {
                ui.add(button).on_hover_text(mode.hint())
            };
            if response.clicked() && mode != current {
                chosen = Some(mode);
            }
        }
    });
    chosen
}

/// Draws the swipe, onion skin, or difference view of a pair. Side by side is drawn by the pair.
pub(crate) fn show_overlay(
    pair: &ImagePair,
    ui: &mut Ui,
    textures: Option<&mut Textures>,
    controls: &mut ImageControls,
    mode: ImageDiffMode,
) {
    let Some((before, after)) = decoded_sizes(pair) else { return };
    ui.vertical_centered(|ui| match mode {
        ImageDiffMode::Swipe => show_swipe(ui, [before, after], textures.as_deref(), controls),
        ImageDiffMode::Onion => show_onion(ui, [before, after], textures.as_deref(), controls),
        ImageDiffMode::Difference => show_difference(ui, pair, textures),
        ImageDiffMode::SideBySide => {}
    });
}

/// Allocates the canvas both images are drawn on: the larger of the two sizes, fitted to the
/// space left and drawn at one scale. Returns the canvas's response, whose rect is the canvas,
/// and the scale.
fn allocate_canvas(ui: &mut Ui, sizes: [[usize; 2]; 2], sense: Sense) -> (Response, f32) {
    let canvas = Vec2::new(sizes[0][0].max(sizes[1][0]) as f32, sizes[0][1].max(sizes[1][1]) as f32);
    let available = (ui.available_size() - Vec2::new(0.0, RESERVED_BELOW)).max(Vec2::splat(MIN_CANVAS));
    let scale = fit_scale(canvas, available);
    let (_, response) = ui.allocate_exact_size(canvas * scale, sense);
    (response, scale)
}

/// The rect an image fills on the canvas: aligned to the top left, at its own size times the scale.
fn image_rect(canvas: Rect, size: [usize; 2], scale: f32) -> Rect {
    Rect::from_min_size(canvas.min, Vec2::new(size[0] as f32, size[1] as f32) * scale)
}

fn show_swipe(ui: &mut Ui, sizes: [[usize; 2]; 2], textures: Option<&Textures>, controls: &mut ImageControls) {
    let (response, scale) = allocate_canvas(ui, sizes, Sense::click_and_drag());
    let canvas = response.rect;
    let painter = ui.painter_at(canvas);
    painter.add(checkerboard_shape(canvas, ui.visuals().dark_mode));
    if response.dragged() || response.clicked() {
        if let Some(pointer) = response.interact_pointer_pos() {
            controls.split = ((pointer.x - canvas.left()) / canvas.width()).clamp(0.0, 1.0);
        }
    }
    let divider = canvas.left() + controls.split * canvas.width();
    if let Some(textures) = textures {
        let sides = [(sizes[0], textures.before.as_ref(), true), (sizes[1], textures.after.as_ref(), false)];
        for (size, texture, before) in sides {
            let (Some(texture), Some((dest, uv))) = (texture, swipe_piece(image_rect(canvas, size, scale), divider, before)) else {
                continue;
            };
            painter.image(texture.id(), dest, uv, Color32::WHITE);
        }
    }
    let near_divider = response.hover_pos().is_some_and(|pointer| (pointer.x - divider).abs() <= 8.0);
    if near_divider || response.dragged() {
        ui.ctx().set_cursor_icon(CursorIcon::ResizeHorizontal);
    }
    draw_divider(&painter, canvas, divider);
    draw_corner_label(&painter, canvas.left_top(), tr("Before"), false);
    draw_corner_label(&painter, canvas.right_top(), tr("After"), true);
    response.widget_info(|| WidgetInfo::slider(true, f64::from(controls.split) * 100.0, tr("Swipe divider")));
    caption(ui, sizes);
}

/// The divider line and its round handle, across the canvas at `divider`.
fn draw_divider(painter: &Painter, canvas: Rect, divider: f32) {
    let line = Stroke::new(2.0, Color32::WHITE);
    painter.line_segment([Pos2::new(divider, canvas.top()), Pos2::new(divider, canvas.bottom())], line);
    let handle = Pos2::new(divider, canvas.center().y);
    painter.circle_filled(handle, 9.0, Color32::WHITE);
    painter.circle_stroke(handle, 9.0, Stroke::new(1.0, Color32::from_black_alpha(160)));
}

/// A small word in a dark chip in a top corner of the canvas. `right` anchors it to the corner's
/// right edge instead of its left.
fn draw_corner_label(painter: &Painter, corner: Pos2, text: &str, right: bool) {
    let galley = painter.layout_no_wrap(text.to_string(), egui::FontId::proportional(12.0), Color32::WHITE);
    let size = galley.size() + Vec2::new(10.0, 4.0);
    let min = if right { Pos2::new(corner.x - size.x - 6.0, corner.y + 6.0) } else { Pos2::new(corner.x + 6.0, corner.y + 6.0) };
    painter.rect_filled(Rect::from_min_size(min, size), 4.0, Color32::from_black_alpha(150));
    painter.galley(min + Vec2::new(5.0, 2.0), galley, Color32::WHITE);
}

fn show_onion(ui: &mut Ui, sizes: [[usize; 2]; 2], textures: Option<&Textures>, controls: &mut ImageControls) {
    let (response, scale) = allocate_canvas(ui, sizes, Sense::hover());
    let canvas = response.rect;
    let painter = ui.painter_at(canvas);
    painter.add(checkerboard_shape(canvas, ui.visuals().dark_mode));
    if let Some(textures) = textures {
        if let Some(before) = &textures.before {
            painter.image(before.id(), image_rect(canvas, sizes[0], scale), FULL_UV, Color32::WHITE);
        }
        if let Some(after) = &textures.after {
            let alpha = (controls.opacity.clamp(0.0, 1.0) * 255.0).round() as u8;
            painter.image(after.id(), image_rect(canvas, sizes[1], scale), FULL_UV, Color32::from_white_alpha(alpha));
        }
    }
    let slider = egui::Slider::new(&mut controls.opacity, 0.0..=1.0)
        .text(tr("Opacity"))
        .custom_formatter(|value, _| format!("{:.0} %", value * 100.0));
    ui.add(slider);
    caption(ui, sizes);
}

fn show_difference(ui: &mut Ui, pair: &ImagePair, textures: Option<&mut Textures>) {
    let Some(difference) = pair.difference.get_or_init(|| pair_difference(pair)).as_ref() else { return };
    let Some(textures) = textures else { return };
    let texture = textures
        .difference
        .get_or_insert_with(|| ui.ctx().load_texture("image-difference", difference.image.clone(), TextureOptions::LINEAR));
    let texture = texture.id();
    let size = difference.image.size;
    let (response, scale) = allocate_canvas(ui, [size, size], Sense::hover());
    let canvas = response.rect;
    let painter = ui.painter_at(canvas);
    painter.add(checkerboard_shape(canvas, ui.visuals().dark_mode));
    painter.image(texture, image_rect(canvas, size, scale), FULL_UV, Color32::WHITE);
    let percent = format!("{:.1}", difference.percent());
    ui.label(RichText::new(tr_fmt!("{percent} % of pixels changed", percent = percent)).color(theme::of(ui).muted));
}

/// The difference of a pair's two versions, or `None` when they are not the same size.
fn pair_difference(pair: &ImagePair) -> Option<Difference> {
    let before = pair.before.as_ref()?.image.as_ref()?;
    let after = pair.after.as_ref()?.image.as_ref()?;
    compare_images(before, after)
}

/// The size caption under the canvas, shown only when the two sizes differ.
fn caption(ui: &mut Ui, sizes: [[usize; 2]; 2]) {
    if sizes[0] == sizes[1] {
        return;
    }
    let text = tr_fmt!("Before {before} · After {after}", before = dimensions(sizes[0]), after = dimensions(sizes[1]));
    ui.label(RichText::new(text).small().monospace().color(theme::of(ui).muted));
}

fn dimensions(size: [usize; 2]) -> String {
    format!("{}×{}", size[0], size[1])
}

/// A checkerboard behind an image, so transparency shows. Drawn as one mesh.
pub(crate) fn checkerboard_shape(rect: Rect, dark: bool) -> Shape {
    let (light, shade) = if dark {
        (Color32::from_rgb(0x3a, 0x3f, 0x4a), Color32::from_rgb(0x2a, 0x2f, 0x38))
    } else {
        (Color32::from_rgb(0xff, 0xff, 0xff), Color32::from_rgb(0xe3, 0xe6, 0xeb))
    };
    const TILE: f32 = 8.0;
    let mut mesh = egui::Mesh::default();
    mesh.add_colored_rect(rect, light);
    let columns = (rect.width() / TILE).ceil() as usize;
    let rows = (rect.height() / TILE).ceil() as usize;
    for row in 0..rows {
        for column in (row % 2..columns).step_by(2) {
            let min = rect.min + Vec2::new(column as f32 * TILE, row as f32 * TILE);
            mesh.add_colored_rect(Rect::from_min_size(min, Vec2::splat(TILE)).intersect(rect), shade);
        }
    }
    Shape::mesh(mesh)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image(width: usize, height: usize, colour: Color32) -> ColorImage {
        ColorImage::filled([width, height], colour)
    }

    #[test]
    fn equal_images_have_no_changed_pixels() {
        let old = image(4, 3, Color32::from_rgb(40, 120, 200));
        let difference = compare_images(&old, &old.clone()).unwrap();
        assert_eq!(difference.changed, 0);
        assert_eq!(difference.total, 12);
        assert_eq!(difference.percent(), 0.0);
    }

    #[test]
    fn one_changed_pixel_is_counted_and_drawn_magenta() {
        let old = image(4, 4, Color32::WHITE);
        let mut new = old.clone();
        new.pixels[5] = Color32::BLACK;
        let difference = compare_images(&old, &new).unwrap();
        assert_eq!(difference.changed, 1);
        assert_eq!(difference.total, 16);
        assert!((difference.percent() - 6.25).abs() < 1e-9);
        assert_eq!(difference.image.pixels[5], Color32::from_rgb(255, 0, 255));
        assert_ne!(difference.image.pixels[0], Color32::from_rgb(255, 0, 255));
    }

    #[test]
    fn small_channel_changes_are_not_counted() {
        let old = image(2, 1, Color32::from_rgb(100, 100, 100));
        let mut new = old.clone();
        // Eight levels is within the threshold; nine is over it.
        new.pixels[0] = Color32::from_rgb(108, 92, 100);
        assert_eq!(compare_images(&old, &new).unwrap().changed, 0);
        new.pixels[1] = Color32::from_rgb(100, 100, 109);
        assert_eq!(compare_images(&old, &new).unwrap().changed, 1);
    }

    #[test]
    fn a_change_in_the_alpha_channel_counts() {
        let old = image(1, 1, Color32::from_rgba_unmultiplied(0, 0, 0, 255));
        let new = image(1, 1, Color32::from_rgba_unmultiplied(0, 0, 0, 200));
        assert_eq!(compare_images(&old, &new).unwrap().changed, 1);
    }

    #[test]
    fn different_sizes_have_no_difference() {
        assert!(compare_images(&image(2, 2, Color32::WHITE), &image(3, 2, Color32::WHITE)).is_none());
    }

    #[test]
    fn fit_scale_fits_the_limiting_side_and_never_exceeds_two() {
        // A 640 by 480 canvas in a 320 by 480 space is limited by its width.
        assert_eq!(fit_scale(Vec2::new(640.0, 480.0), Vec2::new(320.0, 480.0)), 0.5);
        // A tall canvas in a wide space is limited by its height.
        assert_eq!(fit_scale(Vec2::new(200.0, 400.0), Vec2::new(900.0, 200.0)), 0.5);
        // Small images are drawn at most twice their size.
        assert_eq!(fit_scale(Vec2::new(100.0, 50.0), Vec2::new(1000.0, 1000.0)), 2.0);
        assert_eq!(fit_scale(Vec2::ZERO, Vec2::new(10.0, 10.0)), 1.0);
    }

    #[test]
    fn swipe_splits_an_image_at_the_divider() {
        let image_rect = Rect::from_min_size(Pos2::new(10.0, 20.0), Vec2::new(100.0, 50.0));
        let (before, before_uv) = swipe_piece(image_rect, 40.0, true).unwrap();
        assert_eq!((before.left(), before.right()), (10.0, 40.0));
        assert!((before_uv.left() - 0.0).abs() < 1e-6 && (before_uv.right() - 0.3).abs() < 1e-6);
        let (after, after_uv) = swipe_piece(image_rect, 40.0, false).unwrap();
        assert_eq!((after.left(), after.right()), (40.0, 110.0));
        assert!((after_uv.left() - 0.3).abs() < 1e-6 && (after_uv.right() - 1.0).abs() < 1e-6);
    }

    #[test]
    fn swipe_shows_only_the_side_the_divider_leaves_visible() {
        let image_rect = Rect::from_min_size(Pos2::new(0.0, 0.0), Vec2::new(100.0, 10.0));
        // The divider is past the image: the old version fills it and the new one is not drawn.
        let (dest, uv) = swipe_piece(image_rect, 150.0, true).unwrap();
        assert_eq!((dest.right(), uv.right()), (100.0, 1.0));
        assert!(swipe_piece(image_rect, 150.0, false).is_none());
        // The divider is before the image: only the new version shows.
        assert!(swipe_piece(image_rect, -20.0, true).is_none());
        let (_, uv) = swipe_piece(image_rect, -20.0, false).unwrap();
        assert_eq!((uv.left(), uv.right()), (0.0, 1.0));
        // An image at the divider gives nothing for the side it has no room on.
        assert!(swipe_piece(image_rect, 0.0, true).is_none());
    }

    #[test]
    fn the_default_controls_start_in_the_middle() {
        let controls = ImageControls::default();
        assert_eq!((controls.split, controls.opacity), (0.5, 0.5));
    }
}
