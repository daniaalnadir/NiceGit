#![allow(dead_code)]
//! Compares two commits, or a commit with the working files: the changed files on the left, and
//! the selected file's change on the right. Changed PNG images show both versions side by side.

use std::path::Path;

use egui::{Color32, ColorImage, RichText, TextureHandle, TextureOptions, Ui};
use egui_phosphor::regular as icon;
use nicegit_core::compare::ComparedFile;
use nicegit_core::diff::parse_diff;
use nicegit_core::{short, GitClient, GitError, Snapshot};

use crate::diff_view::{self, DiffContent};
use crate::theme;
use crate::tools::{query, widgets, Ctx, Task, ToolWindow};

/// Extensions shown as images, as in the Mac app. HEIC is decoded through macOS's own image
/// tool, so on Windows and Linux it shows its size only.
const IMAGE_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "gif", "bmp", "webp", "tif", "tiff", "ico", "icns", "heic"];

pub struct CompareWindow {
    from: String,
    /// The newer side, or `None` for the working files.
    to: Option<String>,
    files: Refreshing<(), Vec<ComparedFile>>,
    /// The selected file's detail, keyed by its path.
    detail: Refreshing<String, Detail>,
    /// The path the detail was last requested for.
    detail_path: Option<String>,
    /// Whether that detail leaves out whitespace-only changes.
    detail_ignores_whitespace: bool,
    /// The textures for the shown image pair, rebuilt when a new detail arrives.
    textures: Option<Textures>,
    selected: Option<String>,
    split: bool,
    stale: bool,
}

/// What the right-hand pane shows for a selected file.
enum Detail {
    Diff(DiffContent),
    Images(ImagePair),
}

/// Both versions of an image. A side is `None` when the file does not exist on that side.
struct ImagePair {
    before: Option<ImageSide>,
    after: Option<ImageSide>,
}

struct ImageSide {
    bytes: usize,
    /// The decoded picture, or `None` when this build cannot decode the format.
    image: Option<ColorImage>,
}

struct Textures {
    before: Option<TextureHandle>,
    after: Option<TextureHandle>,
}

impl CompareWindow {
    /// Compares `from` (the older side) with `to`, or with the working files when `to` is `None`.
    pub fn new(from: String, to: Option<String>) -> Self {
        Self {
            from,
            to,
            files: Refreshing::new(),
            detail: Refreshing::new(),
            detail_path: None,
            detail_ignores_whitespace: false,
            textures: None,
            selected: None,
            split: false,
            stale: true,
        }
    }

    fn label_for(snapshot: &Snapshot, hash: &str) -> String {
        match snapshot.commits.iter().find(|commit| commit.hash == hash) {
            Some(commit) => format!("{} {}", commit.short_hash, commit.subject),
            None => short(hash).to_string(),
        }
    }
}

impl ToolWindow for CompareWindow {
    fn id(&self) -> String {
        format!("compare:{}:{}", self.from, self.to.as_deref().unwrap_or("working"))
    }

    fn title(&self) -> String {
        "Compare".to_string()
    }

    fn default_size(&self) -> egui::Vec2 {
        egui::vec2(980.0, 600.0)
    }

    fn repository_changed(&mut self, _snapshot: &Snapshot) {
        // Working-file comparisons change with every edit, so reread both lists and details.
        self.stale = true;
        self.detail_path = None;
    }

    fn ui(&mut self, ui: &mut Ui, cx: &mut Ctx) {
        let ctx = ui.ctx().clone();
        if self.stale {
            self.stale = false;
            let from = self.from.clone();
            let to = self.to.clone();
            let task = query(&ctx, cx.repo, move |client, directory| client.compare_files(&from, to.as_deref(), directory));
            self.files.start((), task);
        }
        self.files.settle();
        if let Some(Ok(files)) = self.files.value(&()) {
            if self.selected.is_none() {
                self.selected = files.first().map(|file| file.path.clone());
            }
        }

        // Request the selected file's detail whenever the selection differs from what was asked.
        // The whitespace setting changing asks again too.
        if self.detail_path != self.selected || self.detail_ignores_whitespace != cx.ignore_whitespace {
            if let Some(path) = self.selected.clone() {
                let from = self.from.clone();
                let to = self.to.clone();
                let request_path = path.clone();
                let ignore_whitespace = cx.ignore_whitespace;
                self.detail_ignores_whitespace = ignore_whitespace;
                let task = query(&ctx, cx.repo, move |client, directory| {
                    load_detail(client, directory, &from, to.as_deref(), &request_path, ignore_whitespace)
                });
                self.detail.start(path.clone(), task);
                self.detail_path = Some(path);
                self.textures = None;
            }
        }
        if self.detail.settle() {
            self.textures = None;
        }

        self.header(ui, cx.snapshot);
        ui.separator();
        match self.files.value(&()) {
            None => widgets::loading(ui, "Comparing files"),
            Some(Err(error)) => widgets::error(ui, &message(error)),
            Some(Ok(files)) if files.is_empty() => {
                widgets::empty_state(ui, icon::CHECK_CIRCLE, "No differences in tracked files");
            }
            Some(Ok(_)) => {
                let mut clicked: Option<String> = None;
                egui::Panel::left(ui.id().with("compare_files")).resizable(true).default_size(280.0).size_range(180.0..=460.0).show(
                    ui,
                    |ui| {
                        if let Some(Ok(files)) = self.files.value(&()) {
                            clicked = file_list(ui, files, self.selected.as_deref());
                        }
                    },
                );
                if let Some(path) = clicked {
                    self.selected = Some(path);
                }
                egui::CentralPanel::default().show(ui, |ui| self.detail_pane(ui, &ctx));
            }
        }
    }
}

impl CompareWindow {
    fn header(&mut self, ui: &mut Ui, snapshot: &Snapshot) {
        let c = theme::of(ui);
        let to_label = match &self.to {
            Some(to) => Self::label_for(snapshot, to),
            None => "working files".to_string(),
        };
        ui.horizontal(|ui| {
            ui.label(RichText::new(icon::SCALES).size(16.0).color(c.muted));
            ui.vertical(|ui| {
                ui.label(
                    RichText::new(format!(
                        "{} {} {}",
                        Self::label_for(snapshot, &self.from),
                        egui_phosphor::regular::ARROW_RIGHT,
                        to_label
                    ))
                    .strong(),
                );
                let count = match self.files.value(&()) {
                    Some(Ok(files)) => format!("{} changed {}", files.len(), if files.len() == 1 { "file" } else { "files" }),
                    Some(Err(_)) => "Comparison unavailable".to_string(),
                    None => "Comparing…".to_string(),
                };
                ui.label(RichText::new(count).small().color(c.muted));
            });
        });
    }

    fn detail_pane(&mut self, ui: &mut Ui, ctx: &egui::Context) {
        let Some(path) = self.selected.clone() else {
            widgets::empty_state(ui, icon::GIT_DIFF, "Select a file to see how it changed");
            return;
        };
        let c = theme::of(ui);
        ui.horizontal(|ui| {
            ui.label(RichText::new(&path).strong()).on_hover_text(&path);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                // Only text diffs can be shown side by side; images always use the two panes.
                if !is_image(&path) {
                    crate::tools::widgets::checkbox(ui, true, &mut self.split, "Side by side");
                }
            });
        });
        ui.separator();
        if self.textures.is_none() {
            if let Some(Ok(Detail::Images(pair))) = self.detail.value(&path) {
                self.textures = Some(Textures {
                    before: pair
                        .before
                        .as_ref()
                        .and_then(|side| side.image.as_ref())
                        .map(|image| ctx.load_texture(format!("{path}-before"), image.clone(), TextureOptions::LINEAR)),
                    after: pair
                        .after
                        .as_ref()
                        .and_then(|side| side.image.as_ref())
                        .map(|image| ctx.load_texture(format!("{path}-after"), image.clone(), TextureOptions::LINEAR)),
                });
            }
        }
        match self.detail.value(&path) {
            None => widgets::loading(ui, "Loading the change"),
            Some(Err(error)) => widgets::error(ui, &message(error)),
            Some(Ok(Detail::Diff(content))) => diff_view::show(ui, content, self.split),
            Some(Ok(Detail::Images(pair))) => {
                let textures = self.textures.as_ref();
                ui.columns(2, |columns| {
                    image_pane(
                        &mut columns[0],
                        "Before",
                        pair.before.as_ref(),
                        textures.and_then(|t| t.before.as_ref()),
                        "Added in this change",
                        c.muted,
                    );
                    image_pane(
                        &mut columns[1],
                        "After",
                        pair.after.as_ref(),
                        textures.and_then(|t| t.after.as_ref()),
                        "Deleted in this change",
                        c.muted,
                    );
                });
            }
        }
    }
}

/// The changed files, with a coloured change letter. Returns the file clicked this frame.
fn file_list(ui: &mut Ui, files: &[ComparedFile], selected: Option<&str>) -> Option<String> {
    let mut clicked = None;
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        for file in files {
            let is_selected = selected == Some(file.path.as_str());
            ui.horizontal(|ui| {
                ui.label(RichText::new(&file.status).monospace().strong().color(widgets::change_letter_color(ui, &file.status)));
                let response = ui.selectable_label(is_selected, file.path.as_str()).on_hover_text(&file.path);
                if response.clicked() {
                    clicked = Some(file.path.clone());
                }
            });
        }
    });
    clicked
}

/// One version of an image with its dimensions and file size.
fn image_pane(ui: &mut Ui, title: &str, side: Option<&ImageSide>, texture: Option<&TextureHandle>, missing: &str, muted: Color32) {
    ui.vertical_centered(|ui| {
        ui.horizontal(|ui| {
            ui.label(RichText::new(title).strong());
            if let Some(side) = side {
                let dimensions = side.image.as_ref().map(|image| format!("{} × {} · ", image.size[0], image.size[1])).unwrap_or_default();
                ui.label(RichText::new(format!("{dimensions}{}", human_size(side.bytes))).monospace().small().color(muted));
            }
        });
        ui.add_space(6.0);
        match (side, texture) {
            (None, _) => {
                ui.add_space(24.0);
                ui.label(RichText::new(missing).color(muted));
            }
            (Some(side), Some(texture)) if side.image.is_some() => {
                let available = ui.available_size();
                let image = egui::Image::from_texture(texture).max_size(egui::vec2(available.x.max(40.0), (available.y - 8.0).max(40.0)));
                ui.add(image).on_hover_text(format!("{title} image"));
            }
            (Some(_), _) => {
                ui.add_space(24.0);
                ui.label(RichText::new("This format cannot be previewed").color(muted));
            }
        }
    });
}

/// Whether a path is an image by its extension.
fn is_image(path: &str) -> bool {
    Path::new(path)
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| IMAGE_EXTENSIONS.contains(&extension.to_ascii_lowercase().as_str()))
}

/// The change to one file: either its text diff, or both images for an image file.
fn load_detail(
    client: &GitClient,
    directory: &Path,
    from: &str,
    to: Option<&str>,
    path: &str,
    ignore_whitespace: bool,
) -> nicegit_core::Result<Detail> {
    if is_image(path) {
        let before = client.file_bytes_at(Some(from), path, directory)?.map(decode_side);
        let after = client.file_bytes_at(to, path, directory)?.map(decode_side);
        return Ok(Detail::Images(ImagePair { before, after }));
    }
    let patch = client.compare_file_diff(from, to, path, ignore_whitespace, directory)?;
    Ok(Detail::Diff(DiffContent::new(path.to_string(), parse_diff(&patch))))
}

/// Decodes one version of an image. Formats this build cannot decode keep their size only.
fn decode_side(bytes: Vec<u8>) -> ImageSide {
    let size = bytes.len();
    let decoded = image::load_from_memory(&bytes).ok().or_else(|| largest_icns_png(&bytes)).or_else(|| decode_with_sips(&bytes));
    let image = decoded.map(|decoded| {
        let rgba = decoded.to_rgba8();
        let dimensions = [rgba.width() as usize, rgba.height() as usize];
        ColorImage::from_rgba_unmultiplied(dimensions, rgba.as_raw())
    });
    ImageSide { bytes: size, image }
}

/// Decodes formats such as HEIC with `sips`, which every Mac has, by converting to PNG.
#[cfg(target_os = "macos")]
fn decode_with_sips(bytes: &[u8]) -> Option<image::DynamicImage> {
    let folder = tempfile_folder()?;
    let (input, output) = (folder.join("image"), folder.join("image.png"));
    let decoded = std::fs::write(&input, bytes).ok().and_then(|_| {
        let converted = std::process::Command::new("/usr/bin/sips")
            .args(["-s", "format", "png"])
            .arg(&input)
            .arg("--out")
            .arg(&output)
            .output()
            .ok()?;
        converted.status.success().then(|| image::open(&output).ok()).flatten()
    });
    let _ = std::fs::remove_dir_all(&folder);
    decoded
}

#[cfg(not(target_os = "macos"))]
fn decode_with_sips(_bytes: &[u8]) -> Option<image::DynamicImage> {
    None
}

/// A new private folder for one conversion.
#[cfg(target_os = "macos")]
fn tempfile_folder() -> Option<std::path::PathBuf> {
    let unique = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).ok()?.as_nanos();
    let folder = std::env::temp_dir().join(format!("nicegit-image-{}-{unique}", std::process::id()));
    std::fs::create_dir(&folder).ok()?;
    Some(folder)
}

/// The largest PNG image inside an Apple icon file. Each entry is a four-byte type and a
/// four-byte big-endian length that includes this header; modern icon sizes store PNG data.
fn largest_icns_png(bytes: &[u8]) -> Option<image::DynamicImage> {
    if bytes.get(..4) != Some(b"icns") {
        return None;
    }
    let mut offset = 8;
    let mut best: Option<image::DynamicImage> = None;
    while let Some(header) = bytes.get(offset..offset + 8) {
        let length = u32::from_be_bytes([header[4], header[5], header[6], header[7]]) as usize;
        if length < 8 {
            break;
        }
        let data = bytes.get(offset + 8..offset + length)?;
        if data.starts_with(b"\x89PNG") {
            if let Ok(image) = image::load_from_memory_with_format(data, image::ImageFormat::Png) {
                if best.as_ref().is_none_or(|current| image.width() > current.width()) {
                    best = Some(image);
                }
            }
        }
        offset += length;
    }
    best
}

/// A byte count as a short human-readable size, such as "12.4 KB".
fn human_size(bytes: usize) -> String {
    const UNITS: [&str; 4] = ["KB", "MB", "GB", "TB"];
    if bytes < 1000 {
        return format!("{bytes} bytes");
    }
    // After the first division the value is in kilobytes, the first unit.
    let mut value = bytes as f64 / 1000.0;
    let mut unit = 0;
    while value >= 1000.0 && unit < UNITS.len() - 1 {
        value /= 1000.0;
        unit += 1;
    }
    format!("{value:.1} {}", UNITS[unit])
}

fn message(error: &GitError) -> String {
    match error {
        GitError::CommandFailed { message, .. } => message.clone(),
        other => other.to_string(),
    }
}

/// The latest answer to a background query. A refresh keeps the previous answer on screen until
/// the new one arrives, so the window does not flicker while it reloads.
struct Refreshing<K, T> {
    shown: Option<(K, Task<nicegit_core::Result<T>>)>,
    pending: Option<(K, Task<nicegit_core::Result<T>>)>,
}

impl<K: PartialEq, T: Send + 'static> Refreshing<K, T> {
    fn new() -> Self {
        Self { shown: None, pending: None }
    }

    /// Replaces any request still in flight; only the newest answer is kept.
    fn start(&mut self, key: K, task: Task<nicegit_core::Result<T>>) {
        self.pending = Some((key, task));
    }

    /// Moves a finished request into place. Returns true when a new answer arrived.
    fn settle(&mut self) -> bool {
        let finished = self.pending.as_mut().is_some_and(|(_, task)| !task.is_pending());
        if finished {
            self.shown = self.pending.take();
        }
        finished
    }

    /// The answer for `key`, once it has arrived.
    fn value(&mut self, key: &K) -> Option<&nicegit_core::Result<T>> {
        match &mut self.shown {
            Some((shown, task)) if shown == key => task.get(),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn encoded(width: u32, height: u32, format: image::ImageFormat) -> Vec<u8> {
        let mut bytes = std::io::Cursor::new(Vec::new());
        image::RgbaImage::from_pixel(width, height, image::Rgba([10, 20, 30, 255])).write_to(&mut bytes, format).unwrap();
        bytes.into_inner()
    }

    fn dimensions(side: &ImageSide) -> Option<[usize; 2]> {
        side.image.as_ref().map(|image| image.size)
    }

    #[test]
    fn tiff_and_ico_images_are_decoded() {
        assert_eq!(dimensions(&decode_side(encoded(5, 4, image::ImageFormat::Tiff))), Some([5, 4]));
        assert_eq!(dimensions(&decode_side(encoded(16, 16, image::ImageFormat::Ico))), Some([16, 16]));
    }

    #[test]
    fn apple_icons_show_their_largest_png() {
        let mut entries = Vec::new();
        for (kind, size) in [(b"ic07", 128), (b"ic08", 256), (b"icp4", 16)] {
            let png = encoded(size, size, image::ImageFormat::Png);
            entries.extend_from_slice(kind);
            entries.extend_from_slice(&(png.len() as u32 + 8).to_be_bytes());
            entries.extend_from_slice(&png);
        }
        let mut file = b"icns".to_vec();
        file.extend_from_slice(&(entries.len() as u32 + 8).to_be_bytes());
        file.extend_from_slice(&entries);
        assert_eq!(dimensions(&decode_side(file)), Some([256, 256]));
        assert!(decode_side(b"icns\0\0\0\x08".to_vec()).image.is_none(), "an empty icon file has no image");
    }

    #[test]
    fn heic_is_listed_as_an_image_and_unreadable_bytes_have_no_preview() {
        assert!(is_image("photos/IMG_0001.HEIC"));
        assert!(decode_side(b"not really heic".to_vec()).image.is_none());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn heic_images_are_decoded_on_macos() {
        let folder = tempfile::tempdir().unwrap();
        let (png, heic) = (folder.path().join("in.png"), folder.path().join("out.heic"));
        std::fs::write(&png, encoded(6, 5, image::ImageFormat::Png)).unwrap();
        let made =
            std::process::Command::new("/usr/bin/sips").args(["-s", "format", "heic"]).arg(&png).arg("--out").arg(&heic).output().unwrap();
        assert!(made.status.success(), "{}", String::from_utf8_lossy(&made.stderr));
        let side = decode_side(std::fs::read(&heic).unwrap());
        assert_eq!(dimensions(&side), Some([6, 5]));
    }
}
