//! Apply patch: applies a patch from a file, or from pasted text, to this checkout. The patch is
//! previewed in the background first, so the files it changes, and whether it applies, are seen
//! before anything changes. The preview keeps the branch and HEAD it was read from, and the apply
//! checks them again before it runs.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use egui::RichText;
use egui_phosphor::regular as icon;
use nicegit_core::models::StatusKind;
use nicegit_core::patch_apply::{ApplyMode, FileStatus, PatchFile, PatchKind, PatchPreview, MAX_PATCH_BYTES};
use nicegit_core::{GitClient, GitError};

use super::{query, widgets, Ctx, Task, ToolWindow};
use crate::theme;
use crate::ui::changes::kind_icon;

/// Only one Apply patch window is open at a time.
const ID: &str = "apply-patch";

/// Whether a file's name says it holds a patch, so it can be dropped onto the window.
pub fn is_patch_file(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| matches!(extension.to_ascii_lowercase().as_str(), "patch" | "diff" | "mbox"))
}

/// A preview that is running, with the contents and checkout it reads.
struct Running {
    generation: u64,
    branch: String,
    head: Option<String>,
    task: Task<nicegit_core::Result<PatchPreview>>,
}

/// A finished preview, kept with the contents and checkout it was read from.
#[derive(Clone)]
struct Shown {
    generation: u64,
    branch: String,
    head: Option<String>,
    result: nicegit_core::Result<PatchPreview>,
}

/// Applies a patch to the checkout it was previewed on.
pub struct ApplyPatchWindow {
    /// The patch file chosen or dropped, with its bytes read when it was loaded.
    file: Option<(String, Arc<Vec<u8>>)>,
    /// A file to load when the window first draws, such as one dropped onto the app.
    pending_file: Option<PathBuf>,
    load_error: Option<String>,
    pasted: String,
    /// Changes whenever the contents change, so a preview of older contents is not used.
    generation: u64,
    running: Option<Running>,
    shown: Option<Shown>,
    mode: ApplyMode,
    three_way: bool,
    closing: bool,
}

impl ApplyPatchWindow {
    /// The window, with `file` loaded when it first draws if one is given.
    pub fn new(file: Option<PathBuf>) -> Self {
        Self {
            file: None,
            pending_file: file,
            load_error: None,
            pasted: String::new(),
            generation: 0,
            running: None,
            shown: None,
            mode: ApplyMode::WorkingTree,
            three_way: false,
            closing: false,
        }
    }

    /// Whether there is a patch to preview: a file, or pasted text.
    fn has_contents(&self) -> bool {
        self.file.is_some() || !self.pasted.trim().is_empty()
    }

    /// The bytes to preview and apply. Pasted text is taken as it is, with its line endings.
    fn contents(&self) -> Option<Arc<Vec<u8>>> {
        if let Some((_, bytes)) = &self.file {
            return Some(bytes.clone());
        }
        (!self.pasted.trim().is_empty()).then(|| Arc::new(self.pasted.clone().into_bytes()))
    }

    /// The contents changed, so any preview is out of date.
    fn changed(&mut self) {
        self.generation += 1;
    }

    /// Reads a patch file. The file must be small enough to preview and apply.
    fn load_file(&mut self, path: &Path) {
        let name = path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
        let too_large = std::fs::metadata(path).map(|meta| meta.len() > MAX_PATCH_BYTES as u64);
        match too_large {
            Ok(true) => self.load_error = Some(format!("{name} is larger than 50 MB. Apply it in smaller parts.")),
            Err(error) => self.load_error = Some(format!("Could not read {name}: {error}")),
            Ok(false) => match std::fs::read(path) {
                Ok(bytes) => {
                    self.file = Some((name, Arc::new(bytes)));
                    self.pasted.clear();
                    self.load_error = None;
                    self.changed();
                }
                Err(error) => self.load_error = Some(format!("Could not read {name}: {error}")),
            },
        }
    }

    /// Loads a file and starts its preview, when it could be read.
    fn open_path(&mut self, path: &Path, ctx: &egui::Context, cx: &Ctx) {
        self.load_file(path);
        if self.file.is_some() {
            self.start_preview(ctx, cx);
        }
    }

    /// Reads the current contents in the background, with the checkout they are read on.
    fn start_preview(&mut self, ctx: &egui::Context, cx: &Ctx) {
        let Some(bytes) = self.contents() else { return };
        let generation = self.generation;
        let branch = cx.snapshot.current_branch.clone();
        let head = cx.snapshot.head_hash.clone();
        let task = query(ctx, cx.repo, move |client: &GitClient, repo| client.preview_patch(&bytes, repo));
        self.running = Some(Running { generation, branch, head, task });
    }

    /// Takes a finished preview, and chooses the mode a mailbox patch needs.
    fn collect(&mut self) {
        let arrived = self.running.as_mut().and_then(|running| running.task.get().cloned());
        let Some(result) = arrived else { return };
        let Some(running) = self.running.take() else { return };
        let mailbox = result.as_ref().is_ok_and(|preview| preview.kind == PatchKind::Mailbox);
        if mailbox {
            self.mode = ApplyMode::Commits;
        } else if self.mode == ApplyMode::Commits {
            self.mode = ApplyMode::WorkingTree;
        }
        self.shown = Some(Shown { generation: running.generation, branch: running.branch, head: running.head, result });
    }

    /// The finished preview for the current contents, if there is one.
    fn current(&self) -> Option<&Shown> {
        self.shown.as_ref().filter(|shown| shown.generation == self.generation)
    }

    /// Why the patch cannot be applied now, or `None` when it can.
    fn apply_blocker(&self, cx: &Ctx) -> Option<String> {
        if !cx.idle {
            return Some("Wait for the current Git action to finish".into());
        }
        if cx.editor_unsaved {
            return Some("Save or close the file you are editing first".into());
        }
        if cx.snapshot.operation.is_some() {
            return Some("Finish or abort the current Git operation first".into());
        }
        if !self.has_contents() {
            return Some("Choose a patch file or paste one".into());
        }
        let Some(shown) = self.current() else { return Some("Preview the patch first".into()) };
        let preview = match &shown.result {
            Ok(preview) => preview,
            Err(error) => return Some(message_of(error)),
        };
        if !preview.applies_cleanly && !(self.three_way && preview.three_way_possible) {
            return Some(if preview.three_way_possible {
                "Turn on the three-way merge to apply it".into()
            } else {
                "The patch does not apply to this checkout".into()
            });
        }
        if self.mode == ApplyMode::Commits && preview.kind != PatchKind::Mailbox {
            return Some("Only a mailbox patch can create commits".into());
        }
        None
    }

    /// The patch's changes to the checkout, as the preview reads them.
    fn preview_view(&mut self, ui: &mut egui::Ui, preview: &PatchPreview) {
        let c = theme::of(ui);
        let (text, color) = if preview.applies_cleanly {
            (format!("{}  Applies cleanly", icon::CHECK), c.added)
        } else if preview.three_way_possible {
            (format!("{}  Needs a three-way merge", icon::WARNING), c.warning)
        } else {
            let problem = preview.problem.clone().unwrap_or_else(|| "The patch does not apply.".into());
            (format!("{}  {problem}", icon::WARNING), c.danger)
        };
        ui.label(RichText::new(text).strong().color(color));
        ui.add_space(4.0);
        if preview.kind == PatchKind::Mailbox {
            ui.label(RichText::new(count_text(preview.commits.len(), "commit")).small().color(c.muted));
            for subject in &preview.commits {
                ui.label(format!("•  {subject}"));
            }
        }
        ui.label(RichText::new(count_text(preview.files.len(), "file")).small().color(c.muted));
        egui::ScrollArea::vertical().id_salt("apply_patch_files").max_height(180.0).auto_shrink([false, true]).show(ui, |ui| {
            for file in &preview.files {
                file_row(ui, file);
            }
        });
    }
}

/// One file's row: its status, its path (with the old path for a rename), and its counts.
fn file_row(ui: &mut egui::Ui, file: &PatchFile) {
    let c = theme::of(ui);
    let kind = match file.status {
        FileStatus::Added => StatusKind::Added,
        FileStatus::Deleted => StatusKind::Deleted,
        FileStatus::Modified => StatusKind::Modified,
        FileStatus::Renamed => StatusKind::Renamed,
    };
    let (glyph, color) = kind_icon(ui, kind);
    let path = match &file.old_path {
        Some(old) => format!("{old} {} {}", icon::ARROW_RIGHT, file.path),
        None => file.path.clone(),
    };
    let counts = if file.binary { "binary".to_string() } else { format!("+{} -{}", file.additions, file.deletions) };
    ui.horizontal(|ui| {
        ui.label(RichText::new(glyph).color(color))
            .widget_info(|| egui::WidgetInfo::labeled(egui::Role::Image, true, crate::ui::changes::kind_title(kind)));
        ui.label(RichText::new(glyph).color(color)).widget_info(|| egui::WidgetInfo::labeled(egui::Role::Image, true, kind.title()));
        ui.add(egui::Label::new(RichText::new(path).monospace().small()).truncate());
        ui.label(RichText::new(counts).small().color(c.muted));
    });
}

fn count_text(count: usize, noun: &str) -> String {
    if count == 1 {
        format!("1 {noun}")
    } else {
        format!("{count} {noun}s")
    }
}

/// The message of a Git error without the command it came from.
fn message_of(error: &GitError) -> String {
    match error {
        GitError::CommandFailed { message, .. } => message.clone(),
        other => other.to_string(),
    }
}

impl ToolWindow for ApplyPatchWindow {
    fn id(&self) -> String {
        ID.to_string()
    }

    fn title(&self) -> String {
        "Apply patch".to_string()
    }

    fn default_size(&self) -> egui::Vec2 {
        egui::vec2(640.0, 700.0)
    }

    fn ui(&mut self, ui: &mut egui::Ui, cx: &mut Ctx) {
        let c = theme::of(ui);
        if let Some(path) = self.pending_file.take() {
            self.open_path(&path, ui.ctx(), cx);
        }
        self.collect();

        ui.add_space(4.0);
        ui.label(
            RichText::new(format!(
                "{}  Applies to {}. Preview a patch to see what it changes first.",
                icon::FILE_PLUS,
                cx.snapshot.current_branch
            ))
            .small()
            .color(c.muted),
        );
        ui.add_space(8.0);

        ui.horizontal(|ui| {
            if ui.button("Choose file…").clicked() {
                if let Some(path) = crate::file_dialog::pick_file("Apply a patch", "Patch", &["patch", "diff", "mbox"]) {
                    self.open_path(&path, ui.ctx(), cx);
                }
            }
            let name = self.file.as_ref().map(|(name, _)| name.clone()).unwrap_or_else(|| "No file chosen".into());
            ui.label(RichText::new(name).color(c.muted));
        });
        if let Some(error) = &self.load_error {
            widgets::error(ui, error);
        }

        ui.add_space(8.0);
        widgets::section(ui, "Or paste a patch");
        ui.add_space(2.0);
        let edited = egui::ScrollArea::vertical()
            .id_salt("apply_patch_text_scroll")
            .max_height(220.0)
            .show(ui, |ui| {
                let edit = ui.add(
                    egui::TextEdit::multiline(&mut self.pasted)
                        .font(egui::TextStyle::Monospace)
                        .desired_rows(10)
                        .desired_width(f32::INFINITY)
                        .id_salt("apply_patch_text"),
                );
                edit.widget_info(|| egui::WidgetInfo::labeled(egui::accesskit::Role::MultilineTextInput, true, "Patch text"));
                edit.changed()
            })
            .inner;
        if edited {
            // Typed or pasted text replaces the file that was chosen.
            self.file = None;
            self.changed();
        }

        ui.add_space(6.0);
        let previewing = self.running.is_some();
        ui.horizontal(|ui| {
            let can_preview = self.has_contents() && !previewing;
            if ui.add_enabled(can_preview, egui::Button::new("Preview")).clicked() {
                self.start_preview(ui.ctx(), cx);
            }
            if previewing {
                ui.spinner();
                ui.label(RichText::new("Reading the patch").small().color(c.muted));
            }
        });

        ui.add_space(8.0);
        widgets::section(ui, "Preview");
        ui.add_space(2.0);
        let shown = self.current().cloned();
        match shown.as_ref().map(|shown| &shown.result) {
            None if previewing => {}
            None => {
                ui.label(RichText::new("Preview the patch to see the files it changes.").small().color(c.muted));
            }
            Some(Err(error)) => widgets::error(ui, &message_of(error)),
            Some(Ok(preview)) => self.preview_view(ui, preview),
        }

        ui.add_space(8.0);
        widgets::section(ui, "Apply");
        ui.add_space(2.0);
        let mailbox = matches!(shown.as_ref().map(|shown| &shown.result), Some(Ok(preview)) if preview.kind == PatchKind::Mailbox);
        ui.radio_value(&mut self.mode, ApplyMode::WorkingTree, "Apply as unstaged changes");
        ui.radio_value(&mut self.mode, ApplyMode::Index, "Apply and stage");
        if mailbox || self.mode == ApplyMode::Commits {
            ui.radio_value(&mut self.mode, ApplyMode::Commits, "Create the commits");
        }
        let three_way_possible = matches!(shown.as_ref().map(|shown| &shown.result), Some(Ok(preview)) if preview.three_way_possible);
        widgets::checkbox(ui, three_way_possible, &mut self.three_way, "Use a three-way merge if needed");

        let reason = self.apply_blocker(cx);
        ui.add_space(12.0);
        ui.separator();
        ui.add_space(8.0);
        let mut confirmed = false;
        ui.horizontal(|ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let button = widgets::primary_button(ui, "Apply", reason.is_none());
                let button = match &reason {
                    Some(reason) => button.on_disabled_hover_text(reason.clone()),
                    None => button.on_hover_text("Applies the patch to this checkout"),
                };
                confirmed = button.clicked();
                if ui.button("Cancel").clicked() {
                    self.closing = true;
                }
                if !cx.idle {
                    ui.spinner();
                    ui.label(RichText::new("Waiting for the current action").small().color(c.muted));
                }
            });
        });

        if confirmed {
            self.apply(shown, cx);
        }
    }

    fn wants_close(&self) -> bool {
        self.closing
    }
}

impl ApplyPatchWindow {
    /// Runs the apply for the previewed patch, on the checkout the preview read. The window closes;
    /// the result is shown as a message.
    fn apply(&mut self, shown: Option<Shown>, cx: &mut Ctx) {
        let (Some(shown), Some(bytes)) = (shown, self.contents()) else { return };
        let Ok(preview) = shown.result else { return };
        let mode = self.mode;
        let three_way = self.three_way && preview.three_way_possible;
        let branch = shown.branch;
        let head = shown.head;
        cx.act("Apply patch", move |client, path| {
            client.require_checkout(&branch, head.as_deref(), "apply", path)?;
            client.apply_patch_with(&bytes, mode, three_way, path).map(Some)
        });
        self.closing = true;
    }
}
