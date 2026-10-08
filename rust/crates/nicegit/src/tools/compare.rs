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

/// Extensions shown as images. Only PNG can be decoded in this build; the others report that
/// the preview is unavailable.
const IMAGE_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "gif", "bmp", "webp", "tif", "tiff", "ico"];

pub struct CompareWindow {
    from: String,
    /// The newer side, or `None` for the working files.
    to: Option<String>,
    files: Refreshing<(), Vec<ComparedFile>>,
    /// The selected file's detail, keyed by its path.
    detail: Refreshing<String, Detail>,
    /// The path the detail was last requested for.
    detail_path: Option<String>,
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
        if self.detail_path != self.selected {
            if let Some(path) = self.selected.clone() {
                let from = self.from.clone();
                let to = self.to.clone();
                let request_path = path.clone();
                let task =
                    query(&ctx, cx.repo, move |client, directory| load_detail(client, directory, &from, to.as_deref(), &request_path));
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
                ui.label(RichText::new(format!("{} → {}", Self::label_for(snapshot, &self.from), to_label)).strong());
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
                    ui.checkbox(&mut self.split, "Side by side");
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
fn load_detail(client: &GitClient, directory: &Path, from: &str, to: Option<&str>, path: &str) -> nicegit_core::Result<Detail> {
    if is_image(path) {
        let before = client.file_bytes_at(Some(from), path, directory)?.map(decode_side);
        let after = client.file_bytes_at(to, path, directory)?.map(decode_side);
        return Ok(Detail::Images(ImagePair { before, after }));
    }
    let patch = client.compare_file_diff(from, to, path, false, directory)?;
    Ok(Detail::Diff(DiffContent::new(path.to_string(), parse_diff(&patch))))
}

/// Decodes one version of an image. Formats this build cannot decode keep their size only.
fn decode_side(bytes: Vec<u8>) -> ImageSide {
    let size = bytes.len();
    let image = image::load_from_memory(&bytes).ok().map(|decoded| {
        let rgba = decoded.to_rgba8();
        let dimensions = [rgba.width() as usize, rgba.height() as usize];
        ColorImage::from_rgba_unmultiplied(dimensions, rgba.as_raw())
    });
    ImageSide { bytes: size, image }
}

/// A byte count as a short human-readable size, such as "12.4 KB".
fn human_size(bytes: usize) -> String {
    const UNITS: [&str; 4] = ["bytes", "KB", "MB", "GB"];
    if bytes < 1000 {
        return format!("{bytes} bytes");
    }
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
