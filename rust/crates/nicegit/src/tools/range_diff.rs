//! Compares two versions of a series of commits, as `git range-diff` does: which commits stayed
//! the same, changed, were removed, or were added after a rebase, amend, or force push. The
//! revisions are read in the background, and nothing in the repository changes.

use std::path::Path;

use egui::{Color32, Margin, RichText, Sense, TextStyle, Ui};
use egui_phosphor::regular as icon;
use nicegit_core::range_diff::{RangeDiff, RangeDiffEntry, RangeDiffInputs, RangeDiffStatus};
use nicegit_core::{GitError, Result};

use crate::theme;
use crate::tools::{query, widgets, Ctx, Task, ToolWindow};

/// Where the four revisions come from when the window opens.
#[derive(Clone)]
pub enum Start {
    /// A local branch: its earlier tip from the reflog, against its current tip.
    Branch(String),
    /// Two tips, such as a remote-tracking tip and a local one, from their merge base.
    Tips { old: String, new: String },
    /// Nothing is filled in, so the revisions are typed.
    Blank,
}

/// The four revisions in the window's fields.
#[derive(Clone, Default)]
struct Revisions {
    old_base: String,
    old_tip: String,
    new_base: String,
    new_tip: String,
}

impl Revisions {
    fn from_inputs(inputs: RangeDiffInputs) -> Self {
        Self { old_base: inputs.old_base, old_tip: inputs.old_tip, new_base: inputs.new_base, new_tip: inputs.new_tip }
    }

    /// Whether all four fields hold text, so a comparison can be asked for.
    fn complete(&self) -> bool {
        [&self.old_base, &self.old_tip, &self.new_base, &self.new_tip].iter().all(|value| !value.trim().is_empty())
    }
}

pub struct RangeDiffWindow {
    start: Start,
    revisions: Revisions,
    /// The revisions `start` names, read in the background once the window opens.
    prefill: Option<Task<Result<Option<RangeDiffInputs>>>>,
    started: bool,
    /// Why `start` named no revisions, such as a branch with no earlier version.
    note: Option<String>,
    /// The comparison, read in the background when Compare is chosen.
    running: Option<Task<Result<RangeDiff>>>,
    result: Option<Result<RangeDiff>>,
    /// The index of the changed entry whose comparison is shown.
    expanded: Option<usize>,
}

impl RangeDiffWindow {
    pub fn new(start: Start) -> Self {
        Self {
            start,
            revisions: Revisions::default(),
            prefill: None,
            started: false,
            note: None,
            running: None,
            result: None,
            expanded: None,
        }
    }

    /// Reads the revisions `start` names. A task that finishes after the window closes is dropped
    /// with it, and its result goes nowhere.
    fn begin_prefill(&mut self, ctx: &egui::Context, repo: &Path) {
        self.started = true;
        self.prefill = match self.start.clone() {
            Start::Branch(branch) => Some(query(ctx, repo, move |client, directory| client.default_range_diff_inputs(&branch, directory))),
            Start::Tips { old, new } => {
                Some(query(ctx, repo, move |client, directory| client.tip_range_diff_inputs(&old, &new, directory)))
            }
            Start::Blank => None,
        };
    }

    fn poll_prefill(&mut self) {
        let Some(task) = self.prefill.as_mut() else { return };
        let Some(result) = task.get().cloned() else { return };
        self.prefill = None;
        match result {
            Ok(Some(inputs)) => self.revisions = Revisions::from_inputs(inputs),
            Ok(None) => self.note = Some(self.no_revisions_note()),
            Err(error) => self.result = Some(Err(error)),
        }
    }

    fn no_revisions_note(&self) -> String {
        match &self.start {
            Start::Branch(branch) => format!("{branch} has no earlier version in its reflog to compare with."),
            _ => "These two versions share no history, so they cannot be compared.".to_string(),
        }
    }

    fn poll_comparison(&mut self) {
        let Some(task) = self.running.as_mut() else { return };
        if let Some(result) = task.get().cloned() {
            self.result = Some(result);
            self.running = None;
        }
    }

    fn compare(&mut self, ctx: &egui::Context, repo: &Path) {
        let revisions = self.revisions.clone();
        self.result = None;
        self.expanded = None;
        self.running = Some(query(ctx, repo, move |client, directory| {
            client.range_diff(&revisions.old_base, &revisions.old_tip, &revisions.new_base, &revisions.new_tip, directory)
        }));
    }

    /// The four revision fields and the Compare button. Returns whether Compare was chosen.
    fn fields(&mut self, ui: &mut Ui, can_compare: bool) -> bool {
        egui::Grid::new("range-diff-revisions").num_columns(2).spacing([8.0, 6.0]).show(ui, |ui| {
            revision_row(ui, "Old base", &mut self.revisions.old_base);
            revision_row(ui, "Old tip", &mut self.revisions.old_tip);
            revision_row(ui, "New base", &mut self.revisions.new_base);
            revision_row(ui, "New tip", &mut self.revisions.new_tip);
        });
        ui.add_space(6.0);
        widgets::primary_button(ui, "Compare", can_compare).clicked()
    }

    /// The counts of each status, in words.
    fn summary(diff: &RangeDiff) -> String {
        format!(
            "{} unchanged, {} changed, {} removed, {} added",
            diff.count(RangeDiffStatus::Same),
            diff.count(RangeDiffStatus::Changed),
            diff.count(RangeDiffStatus::Removed),
            diff.count(RangeDiffStatus::Added),
        )
    }

    /// The comparison's summary and one row per commit pair. Returns the index of a changed pair
    /// whose row was clicked, so the caller can open or close its comparison.
    fn results(ui: &mut Ui, diff: &RangeDiff, expanded: Option<usize>) -> Option<usize> {
        if diff.entries.is_empty() {
            widgets::empty_state(ui, icon::GIT_DIFF, "Neither version has commits to compare.");
            return None;
        }
        ui.label(RichText::new(Self::summary(diff)).strong());
        ui.add_space(4.0);
        let mut clicked = None;
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            for (index, entry) in diff.entries.iter().enumerate() {
                let open = expanded == Some(index);
                if entry_row(ui, entry, open) {
                    clicked = Some(index);
                }
                if open {
                    details(ui, &entry.diff_lines);
                }
                ui.add_space(2.0);
            }
        });
        clicked
    }
}

impl ToolWindow for RangeDiffWindow {
    fn id(&self) -> String {
        match &self.start {
            Start::Branch(branch) => format!("range-diff:branch:{branch}"),
            Start::Tips { old, new } => format!("range-diff:{old}:{new}"),
            Start::Blank => "range-diff".to_string(),
        }
    }

    fn title(&self) -> String {
        "Compare versions".to_string()
    }

    fn default_size(&self) -> egui::Vec2 {
        egui::vec2(720.0, 560.0)
    }

    fn ui(&mut self, ui: &mut Ui, cx: &mut Ctx) {
        if !self.started {
            self.begin_prefill(ui.ctx(), cx.repo);
        }
        self.poll_prefill();
        self.poll_comparison();

        let can_compare = self.revisions.complete() && self.running.is_none() && self.prefill.is_none();
        if self.fields(ui, can_compare) {
            self.compare(ui.ctx(), cx.repo);
        }
        if self.prefill.is_some() {
            widgets::loading(ui, "Reading the versions…");
        }
        if let Some(note) = &self.note {
            ui.label(RichText::new(note).small().color(theme::of(ui).muted));
        }
        ui.separator();

        if self.running.is_some() {
            widgets::loading(ui, "Comparing the versions…");
            return;
        }
        match &self.result {
            None => {
                let c = theme::of(ui);
                ui.label(RichText::new("Compare two versions to see which commits changed.").color(c.muted));
            }
            Some(Err(error)) => widgets::error(ui, &message(error)),
            Some(Ok(diff)) => {
                if let Some(index) = Self::results(ui, diff, self.expanded) {
                    self.expanded = if self.expanded == Some(index) { None } else { Some(index) };
                }
            }
        }
    }
}

/// One labelled revision field, in monospace, which keeps a long object ID readable.
fn revision_row(ui: &mut Ui, label: &str, value: &mut String) {
    ui.label(label);
    ui.add(egui::TextEdit::singleline(value).font(TextStyle::Monospace).desired_width(320.0));
    ui.end_row();
}

/// The colour for a status: unchanged is quiet, changed stands out, and removed and added follow
/// the diff colours.
fn status_color(c: &theme::Colors, status: RangeDiffStatus) -> Color32 {
    match status {
        RangeDiffStatus::Same => c.muted,
        RangeDiffStatus::Changed => c.warning,
        RangeDiffStatus::Removed => c.removed,
        RangeDiffStatus::Added => c.added,
    }
}

/// A commit's position and abbreviated hash, or a dash where the series has none.
fn side_text(commit: &Option<nicegit_core::range_diff::RangeDiffCommit>) -> String {
    match commit {
        Some(commit) => format!("{}: {}", commit.index, commit.short),
        None => "-".to_string(),
    }
}

/// One pair's row. A changed pair shows a caret and reports a click, so the caller can open its
/// comparison. Returns whether it was clicked.
fn entry_row(ui: &mut Ui, entry: &RangeDiffEntry, open: bool) -> bool {
    let c = theme::of(ui);
    let color = status_color(&c, entry.status);
    let expandable = entry.status == RangeDiffStatus::Changed;
    let response = egui::Frame::new()
        .corner_radius(6.0)
        .inner_margin(Margin::symmetric(8, 5))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_wrapped(|ui| {
                if expandable {
                    let caret = if open { icon::CARET_DOWN } else { icon::CARET_RIGHT };
                    ui.label(RichText::new(caret).small().color(c.muted));
                }
                ui.label(RichText::new(entry.status.symbol()).monospace().strong().color(color));
                ui.label(RichText::new(side_text(&entry.old)).monospace().small().color(c.muted));
                ui.label(RichText::new(icon::ARROW_RIGHT).small().color(c.muted));
                ui.label(RichText::new(side_text(&entry.new)).monospace().small().color(c.muted));
                ui.label(RichText::new(&entry.subject));
            });
        })
        .response;
    if !expandable {
        return false;
    }
    response.interact(Sense::click()).clicked()
}

/// A changed pair's comparison, with added and removed patch lines in their colours.
fn details(ui: &mut Ui, lines: &[String]) {
    let c = theme::of(ui);
    egui::Frame::new().fill(c.subtle_bg).corner_radius(6.0).inner_margin(Margin::symmetric(10, 8)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        if lines.is_empty() {
            ui.label(RichText::new("The patches match; only the message differs.").small().color(c.muted));
            return;
        }
        for line in lines {
            let color = match line.chars().next() {
                Some('+') => c.added,
                Some('-') => c.removed,
                Some('@') => c.muted,
                _ => ui.visuals().text_color(),
            };
            ui.label(RichText::new(line).monospace().small().color(color));
        }
    });
}

fn message(error: &GitError) -> String {
    match error {
        GitError::CommandFailed { message, .. } => message.clone(),
        other => other.to_string(),
    }
}
