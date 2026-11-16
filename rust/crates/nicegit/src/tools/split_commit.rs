//! Split commit: divides one commit into two. The changes checked go into the first new commit, the
//! rest into the second, and both get their own message. The core rebuilds the commit and the commits
//! after it; the working tree and index are not touched.

use std::collections::BTreeSet;
use std::path::Path;

use egui::RichText;
use egui_phosphor::regular as icon;
use nicegit_core::models::short;
use nicegit_core::split::{first_patch, split_files, FileChoice, SplitFile, SplitRequest};
use nicegit_core::undo::UndoMode;
use nicegit_core::{GitClient, Snapshot};

use super::{query, widgets, Ctx, Task, ToolWindow};
use crate::theme;

/// The commit's changes, and its message, as read when the window opens.
#[derive(Clone)]
struct Loaded {
    files: Vec<SplitFile>,
    message: String,
    published: bool,
    /// What is checked for each file. All of it starts checked, so the whole commit is in the first one.
    picks: Vec<Pick>,
}

/// What is checked for one file. A file that can only move whole uses `whole`. Any other file uses
/// `hunks`, and counts as checked while some of its hunks are.
#[derive(Clone, Default, PartialEq, Eq)]
struct Pick {
    whole: bool,
    hunks: BTreeSet<usize>,
}

impl Pick {
    fn all(file: &SplitFile) -> Self {
        Pick { whole: true, hunks: (0..file.hunks.len()).collect() }
    }
}

/// Whether any of the file's changes go into the first commit.
fn file_checked(file: &SplitFile, pick: &Pick) -> bool {
    if file.whole_only {
        pick.whole
    } else {
        !pick.hunks.is_empty()
    }
}

/// Checks or unchecks every change of a file.
fn set_file(file: &SplitFile, pick: &mut Pick, checked: bool) {
    if file.whole_only {
        pick.whole = checked;
    } else {
        pick.hunks = if checked { (0..file.hunks.len()).collect() } else { BTreeSet::new() };
    }
}

/// How the file goes into the first commit: not at all, whole, or only its checked hunks.
fn choice(file: &SplitFile, pick: &Pick) -> FileChoice {
    if !file_checked(file, pick) {
        FileChoice::Nothing
    } else if file.whole_only || pick.hunks.len() == file.hunks.len() {
        FileChoice::Whole
    } else {
        FileChoice::Hunks(pick.hunks.clone())
    }
}

/// The number of changes a file offers: one for a file that moves whole, else one per hunk.
fn changes_in(file: &SplitFile) -> usize {
    if file.whole_only {
        1
    } else {
        file.hunks.len()
    }
}

/// How many of a file's changes are checked.
fn checked_changes(file: &SplitFile, pick: &Pick) -> usize {
    if !file_checked(file, pick) {
        0
    } else if file.whole_only {
        1
    } else {
        pick.hunks.len()
    }
}

fn count_text(count: usize) -> String {
    if count == 1 {
        "1 change".to_string()
    } else {
        format!("{count} changes")
    }
}

/// Splits one commit into two, from the window the commit menu opens.
pub struct SplitWindow {
    commit: String,
    short_hash: String,
    subject: String,
    branch: String,
    head: Option<String>,
    load: Option<Task<nicegit_core::Result<Loaded>>>,
    loaded: Option<Loaded>,
    error: Option<String>,
    /// Files whose changes are listed, by their index.
    expanded: BTreeSet<usize>,
    first: String,
    second: String,
    closing: bool,
}

impl SplitWindow {
    /// The window for `hash`, which is on the checked-out branch. The branch and HEAD are captured
    /// now, so the split refuses if the checkout changes before it is confirmed.
    pub fn new(hash: &str, subject: &str, snapshot: &Snapshot) -> Self {
        Self {
            commit: hash.to_string(),
            short_hash: short(hash).to_string(),
            subject: subject.to_string(),
            branch: snapshot.current_branch.clone(),
            head: snapshot.head_hash.clone(),
            load: None,
            loaded: None,
            error: None,
            expanded: BTreeSet::new(),
            first: subject.to_string(),
            second: String::new(),
            closing: false,
        }
    }
}

/// One file's row, with its hunks when they are shown.
fn file_row(ui: &mut egui::Ui, cx: &Ctx, index: usize, file: &SplitFile, pick: &mut Pick, expanded: &mut BTreeSet<usize>) {
    let checked_now = file_checked(file, pick);
    let partly = !file.whole_only && checked_now && pick.hunks.len() < file.hunks.len();
    ui.horizontal(|ui| {
        let mut checked = checked_now;
        let label = if partly { format!("{}  (some changes)", file.path) } else { file.path.clone() };
        if widgets::checkbox(ui, cx.idle, &mut checked, label).changed() {
            set_file(file, pick, checked);
        }
        if file.whole_only {
            ui.label(RichText::new("moves as a whole").small().color(theme::of(ui).muted));
        } else if !file.hunks.is_empty() {
            let open = expanded.contains(&index);
            let glyph = if open { icon::CARET_DOWN } else { icon::CARET_RIGHT };
            let text = format!("{glyph}  {}", count_text(file.hunks.len()));
            if ui.small_button(text).on_hover_text(if open { "Hide the changes" } else { "Show the changes" }).clicked() {
                if open {
                    expanded.remove(&index);
                } else {
                    expanded.insert(index);
                }
            }
        }
    });
    if expanded.contains(&index) && !file.whole_only {
        for (number, hunk) in file.hunks.iter().enumerate() {
            ui.horizontal(|ui| {
                ui.add_space(22.0);
                let mut checked = pick.hunks.contains(&number);
                if widgets::checkbox(ui, cx.idle, &mut checked, format!("Change {}", number + 1)).changed() {
                    if checked {
                        pick.hunks.insert(number);
                    } else {
                        pick.hunks.remove(&number);
                    }
                }
            });
            for change in hunk.changes.iter().take(8) {
                ui.horizontal(|ui| {
                    ui.add_space(44.0);
                    ui.add(egui::Label::new(RichText::new(change).monospace().small()).truncate());
                });
            }
            if hunk.changes.len() > 8 {
                ui.horizontal(|ui| {
                    ui.add_space(44.0);
                    ui.label(RichText::new(format!("{} more lines", hunk.changes.len() - 8)).small().color(theme::of(ui).muted));
                });
            }
        }
    }
}

impl ToolWindow for SplitWindow {
    fn id(&self) -> String {
        format!("split:{}", self.commit)
    }

    fn title(&self) -> String {
        "Split commit".to_string()
    }

    fn default_size(&self) -> egui::Vec2 {
        egui::vec2(640.0, 700.0)
    }

    fn ui(&mut self, ui: &mut egui::Ui, cx: &mut Ctx) {
        let c = theme::of(ui);
        if self.load.is_none() {
            let hash = self.commit.clone();
            self.load = Some(query(ui.ctx(), cx.repo, move |client: &GitClient, repo: &Path| {
                let files = split_files(&client.split_patch(&hash, repo)?);
                let picks = files.iter().map(Pick::all).collect();
                Ok(Loaded {
                    picks,
                    files,
                    message: client.commit_message(&hash, repo)?.trim_end().to_string(),
                    published: client.commit_is_published(&hash, repo)?,
                })
            }));
        }
        if self.loaded.is_none() && self.error.is_none() {
            let arrived = self.load.as_mut().and_then(|task| task.get().cloned());
            match arrived {
                Some(Ok(loaded)) => {
                    if self.second.is_empty() {
                        self.second = loaded.message.clone();
                    }
                    self.loaded = Some(loaded);
                }
                Some(Err(error)) => self.error = Some(error.to_string()),
                None => {}
            }
        }

        ui.add_space(4.0);
        ui.horizontal(|ui| {
            widgets::hash_label(ui, &self.commit);
            ui.label(RichText::new(&self.subject).strong().color(ui.visuals().text_color()));
        });
        ui.label(
            RichText::new(format!("{}  Rewrites {} as two commits. The commits after it are replayed.", icon::SCISSORS, self.branch))
                .small()
                .color(c.muted),
        );
        ui.add_space(8.0);

        if let Some(error) = &self.error {
            widgets::error(ui, error);
            return;
        }
        let Some(loaded) = self.loaded.as_mut() else {
            widgets::loading(ui, "Reading the commit's changes");
            return;
        };
        if loaded.published {
            widgets::callout(
                ui,
                "This commit is already on a remote. Splitting it rewrites it, so you will need to force-push, and anyone who has it must reconcile their copy.",
                true,
            );
            ui.add_space(8.0);
        }

        widgets::section(ui, "Changes in the first commit");
        ui.add_space(2.0);
        if loaded.files.is_empty() {
            ui.label(RichText::new("This commit has no changes to split.").small().color(c.muted));
        }
        egui::ScrollArea::vertical().id_salt("split_files").max_height(200.0).auto_shrink([false, false]).show(ui, |ui| {
            for (index, file) in loaded.files.iter().enumerate() {
                file_row(ui, cx, index, file, &mut loaded.picks[index], &mut self.expanded);
            }
        });

        let (first_count, total): (usize, usize) = loaded
            .files
            .iter()
            .zip(&loaded.picks)
            .fold((0, 0), |(checked, total), (file, pick)| (checked + checked_changes(file, pick), total + changes_in(file)));
        let second_count = total - first_count;
        ui.add_space(6.0);
        ui.label(
            RichText::new(format!("{} in the first commit, {} in the second.", count_text(first_count), count_text(second_count)))
                .small()
                .color(c.muted),
        );

        ui.add_space(8.0);
        // Each box has a name for assistive tools, since its text is the commit message rather than a label.
        widgets::section(ui, "First commit");
        let first =
            ui.add(egui::TextEdit::multiline(&mut self.first).desired_rows(2).desired_width(f32::INFINITY).id_salt("split_first_message"));
        first.widget_info(|| egui::WidgetInfo::labeled(egui::accesskit::Role::MultilineTextInput, true, "First commit message"));
        ui.add_space(6.0);
        widgets::section(ui, "Second commit");
        let second = ui
            .add(egui::TextEdit::multiline(&mut self.second).desired_rows(4).desired_width(f32::INFINITY).id_salt("split_second_message"));
        second.widget_info(|| egui::WidgetInfo::labeled(egui::accesskit::Role::MultilineTextInput, true, "Second commit message"));

        let reason = if !cx.idle {
            Some("Wait for the current Git action to finish")
        } else if self.head.is_none() {
            Some("Check out a branch first")
        } else if first_count == 0 {
            Some("Check at least one change for the first commit")
        } else if second_count == 0 {
            Some("Leave at least one change for the second commit")
        } else if self.first.trim().is_empty() || self.second.trim().is_empty() {
            Some("Both commits need a message")
        } else {
            None
        };

        ui.add_space(12.0);
        ui.separator();
        ui.add_space(8.0);
        let mut confirmed = false;
        ui.horizontal(|ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let button = widgets::primary_button(ui, "Split", reason.is_none());
                let button = match reason {
                    Some(reason) => button.on_disabled_hover_text(reason),
                    None => button.on_hover_text("Rewrites the commit as two commits"),
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
            if let Some(head) = self.head.clone() {
                let choices: Vec<FileChoice> = loaded.files.iter().zip(&loaded.picks).map(|(file, pick)| choice(file, pick)).collect();
                match first_patch(&loaded.files, &choices) {
                    Ok(patch) => {
                        let request = SplitRequest {
                            branch: self.branch.clone(),
                            expected_head: head,
                            commit: self.commit.clone(),
                            first_patch: patch,
                            first_message: self.first.trim().to_string(),
                            second_message: self.second.trim().to_string(),
                        };
                        let short_hash = self.short_hash.clone();
                        cx.act_recording(
                            format!("Split {}", self.short_hash),
                            "Split commit",
                            Some(self.subject.clone()),
                            UndoMode::Soft,
                            move |client, repo| {
                                client.split_commit(&request, repo)?;
                                Ok(Some(format!("Split {short_hash} into two commits.")))
                            },
                        );
                        self.closing = true;
                    }
                    Err(error) => cx.notice(error.to_string(), true),
                }
            }
        }
    }

    fn wants_close(&self) -> bool {
        self.closing
    }
}
