//! Interactive rebase editor: choose what happens to each commit after a base, reorder them, and
//! reword, squash, fix up, or drop them. Commits are listed oldest first, as Git applies them.

#![allow(dead_code)]

use std::collections::BTreeSet;

use egui::{RichText, TextStyle};
use egui_phosphor::regular as icon;
use nicegit_core::models::Commit;
use nicegit_core::rebase::{RebaseAction, RebasePlan, RebaseStep};
use nicegit_core::Snapshot;

use super::{query, widgets, Ctx, Task, ToolWindow};
use crate::theme;

/// What happens to one commit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Choice {
    Pick,
    Reword,
    Squash,
    Fixup,
    Drop,
}

impl Choice {
    const ALL: [Choice; 5] = [Choice::Pick, Choice::Reword, Choice::Squash, Choice::Fixup, Choice::Drop];

    fn title(self) -> &'static str {
        match self {
            Choice::Pick => "Pick",
            Choice::Reword => "Reword",
            Choice::Squash => "Squash",
            Choice::Fixup => "Fixup",
            Choice::Drop => "Drop",
        }
    }

    fn help(self) -> &'static str {
        match self {
            Choice::Pick => "Keep this commit as it is",
            Choice::Reword => "Keep this commit with a new message",
            Choice::Squash => "Combine into the commit above, keeping both messages",
            Choice::Fixup => "Combine into the commit above, discarding this message",
            Choice::Drop => "Remove this commit and its changes",
        }
    }
}

/// One commit in the editor, with its chosen action and the message it would keep.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Entry {
    commit: Commit,
    choice: Choice,
    message: String,
}

impl Entry {
    fn action(&self) -> RebaseAction {
        match self.choice {
            Choice::Pick => RebaseAction::Pick,
            Choice::Reword => RebaseAction::Reword(self.message.clone()),
            Choice::Squash => RebaseAction::Squash,
            Choice::Fixup => RebaseAction::Fixup,
            Choice::Drop => RebaseAction::Drop,
        }
    }
}

/// Edits the commits after `base` through HEAD. The branch and HEAD are captured when the window
/// opens, so the rewrite refuses if the checkout changed while the editor was open.
pub struct InteractiveRebaseWindow {
    base: String,
    branch: String,
    head: Option<String>,
    load: Option<Task<nicegit_core::Result<RebasePlan>>>,
    plan: Option<RebasePlan>,
    original: Vec<Entry>,
    entries: Vec<Entry>,
    error: Option<String>,
    closing: bool,
}

impl InteractiveRebaseWindow {
    /// `base` is the commit to keep; the commits after it up to HEAD are listed. An empty `base`
    /// rewrites the whole history, from the root commit.
    pub fn new(base: String, snapshot: &Snapshot) -> Self {
        Self {
            base,
            branch: snapshot.current_branch.clone(),
            head: snapshot.head_hash.clone(),
            load: None,
            plan: None,
            original: Vec::new(),
            entries: Vec::new(),
            error: None,
            closing: false,
        }
    }

    /// Takes the loaded plan once it arrives, and starts the editor from it.
    fn poll_load(&mut self) {
        if self.plan.is_some() || self.error.is_some() {
            return;
        }
        let Some(task) = self.load.as_mut() else { return };
        match task.get() {
            Some(Ok(plan)) => {
                let entries: Vec<Entry> = plan
                    .commits
                    .iter()
                    .map(|commit| Entry {
                        commit: commit.clone(),
                        choice: Choice::Pick,
                        message: plan.messages.get(&commit.hash).cloned().unwrap_or_else(|| commit.subject.clone()),
                    })
                    .collect();
                self.original = entries.clone();
                self.entries = entries;
                self.plan = Some(plan.clone());
            }
            Some(Err(error)) => self.error = Some(error.to_string()),
            None => {}
        }
    }

    fn published_count(&self) -> usize {
        match &self.plan {
            Some(plan) => self.entries.iter().filter(|entry| plan.published_commits.contains(&entry.commit.hash)).count(),
            None => 0,
        }
    }

    /// A reason the rewrite cannot run yet, if any.
    fn problem(&self) -> Option<&'static str> {
        // Squash and fixup combine into the commit applied before them, so the first kept commit
        // cannot be one of them.
        if let Some(first) = self.entries.iter().find(|entry| entry.choice != Choice::Drop) {
            if matches!(first.choice, Choice::Squash | Choice::Fixup) {
                return Some("The oldest kept commit has no earlier commit to combine into. Pick or reword it instead.");
            }
        }
        if self.entries.iter().any(|entry| entry.choice == Choice::Reword && entry.message.trim().is_empty()) {
            return Some("A reworded commit needs a message.");
        }
        None
    }

    fn summary(&self) -> String {
        let kept = self.entries.iter().filter(|entry| matches!(entry.choice, Choice::Pick | Choice::Reword)).count();
        let dropped = self.entries.iter().filter(|entry| entry.choice == Choice::Drop).count();
        let commits = if kept == 1 { "commit" } else { "commits" };
        if dropped > 0 {
            format!("Result: {kept} {commits}, {dropped} dropped")
        } else {
            format!("Result: {kept} {commits}")
        }
    }

    fn changed(&self) -> bool {
        self.entries != self.original
    }

    /// Moves the commit at `index` by `offset` places in the list.
    fn move_entry(&mut self, index: usize, offset: isize) {
        let Some(target) = index.checked_add_signed(offset) else { return };
        if index < self.entries.len() && target < self.entries.len() {
            self.entries.swap(index, target);
        }
    }

    fn row(
        entries: &mut [Entry],
        index: usize,
        published: &BTreeSet<String>,
        idle: bool,
        moved: &mut Option<(usize, isize)>,
        ui: &mut egui::Ui,
    ) {
        let c = theme::of(ui);
        let count = entries.len();
        let entry = &mut entries[index];
        egui::Frame::new()
            .fill(c.card_bg)
            .stroke(egui::Stroke::new(1.0, c.border))
            .corner_radius(6.0)
            .inner_margin(egui::Margin::symmetric(10, 6))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.label(RichText::new(format!("{}", index + 1)).monospace().color(c.muted));
                    if widgets::icon_button(ui, icon::ARROW_UP, "Move earlier", idle && index > 0).clicked() {
                        *moved = Some((index, -1));
                    }
                    if widgets::icon_button(ui, icon::ARROW_DOWN, "Move later", idle && index + 1 < count).clicked() {
                        *moved = Some((index, 1));
                    }
                    egui::ComboBox::from_id_salt(("interactive-rebase-choice", entry.commit.hash.as_str()))
                        .selected_text(entry.choice.title())
                        .width(96.0)
                        .show_ui(ui, |ui| {
                            for option in Choice::ALL {
                                ui.selectable_value(&mut entry.choice, option, option.title()).on_hover_text(option.help());
                            }
                        });
                    widgets::hash_label(ui, &entry.commit.hash);
                    let subject = RichText::new(&entry.commit.subject);
                    let subject = if entry.choice == Choice::Drop { subject.strikethrough().color(c.muted) } else { subject };
                    ui.add(egui::Label::new(subject).truncate());
                    if matches!(entry.choice, Choice::Squash | Choice::Fixup) {
                        ui.label(RichText::new(icon::ARROW_BEND_DOWN_RIGHT).color(c.muted)).on_hover_text("Combined into the commit above");
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if published.contains(&entry.commit.hash) {
                            ui.label(RichText::new(icon::NETWORK).color(c.warning)).on_hover_text("Already on a remote");
                        }
                        ui.label(RichText::new(&entry.commit.author_name).small().color(c.muted));
                    });
                });
                if entry.choice == Choice::Reword {
                    ui.add_space(4.0);
                    ui.add(
                        egui::TextEdit::multiline(&mut entry.message)
                            .font(TextStyle::Monospace)
                            .desired_rows(4)
                            .desired_width(f32::INFINITY)
                            .hint_text("New commit message"),
                    );
                }
            });
        ui.add_space(4.0);
    }
}

impl ToolWindow for InteractiveRebaseWindow {
    fn id(&self) -> String {
        format!("interactive-rebase:{}", self.base)
    }

    fn title(&self) -> String {
        "Interactive rebase".to_string()
    }

    fn default_size(&self) -> egui::Vec2 {
        egui::vec2(820.0, 580.0)
    }

    fn ui(&mut self, ui: &mut egui::Ui, cx: &mut Ctx) {
        if self.load.is_none() && self.plan.is_none() && self.error.is_none() {
            let base = (!self.base.is_empty()).then(|| self.base.clone());
            self.load = Some(query(ui.ctx(), cx.repo, move |client, repo| client.interactive_rebase_plan(base.as_deref(), repo)));
        }
        self.poll_load();

        let c = theme::of(ui);
        ui.add_space(4.0);
        ui.label(RichText::new("Interactive rebase").size(16.0).strong());
        let count = self.entries.len();
        let commits = if count == 1 { "commit" } else { "commits" };
        ui.label(
            RichText::new(format!(
                "Rewrite {count} {commits} on {}. Listed oldest first, as Git applies them. Squash and fixup combine a commit into the one above it.",
                self.branch
            ))
            .small()
            .color(c.muted),
        );
        ui.add_space(8.0);

        let published = self.published_count();
        if published > 0 {
            let commits = if published == 1 { "commit is" } else { "commits are" };
            widgets::callout(
                ui,
                &format!(
                    "{published} of these {commits} already on a remote. Rewriting them means you will need to force-push, and anyone who has them must reconcile their copies."
                ),
                true,
            );
            ui.add_space(8.0);
        }

        if let Some(error) = self.error.clone() {
            widgets::error(ui, &error);
        } else if self.plan.is_none() {
            widgets::loading(ui, "Reading commits...");
        } else {
            let idle = cx.idle;
            let mut moved: Option<(usize, isize)> = None;
            let plan = self.plan.as_ref().map(|plan| plan.published_commits.clone()).unwrap_or_default();
            // Leave room below the list for the summary and buttons.
            let list_height = (ui.available_height() - 110.0).max(120.0);
            egui::ScrollArea::vertical().auto_shrink([false, false]).max_height(list_height).show(ui, |ui| {
                for index in 0..self.entries.len() {
                    Self::row(&mut self.entries, index, &plan, idle, &mut moved, ui);
                }
            });
            if let Some((index, offset)) = moved {
                self.move_entry(index, offset);
            }
        }

        ui.add_space(8.0);
        ui.separator();
        ui.add_space(6.0);
        let problem = self.problem();
        if let Some(problem) = problem {
            widgets::error(ui, problem);
        } else if self.plan.is_some() {
            ui.label(RichText::new(self.summary()).small().color(c.muted));
        }

        let mut start = false;
        let mut cancel = false;
        let mut reset = false;
        ui.horizontal(|ui| {
            let ready = cx.idle && self.plan.is_some() && self.head.is_some() && problem.is_none() && self.changed();
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                start = widgets::primary_button(ui, "Rewrite commits", ready).clicked();
                if ui.button("Cancel").clicked() {
                    cancel = true;
                }
                if ui.add_enabled(cx.idle && self.changed(), egui::Button::new("Reset edits")).clicked() {
                    reset = true;
                }
            });
        });

        if cancel {
            self.closing = true;
        }
        if reset {
            self.entries = self.original.clone();
        }
        if start {
            if let (Some(plan), Some(head)) = (self.plan.clone(), self.head.clone()) {
                let steps: Vec<RebaseStep> =
                    self.entries.iter().map(|entry| RebaseStep { commit: entry.commit.clone(), action: entry.action() }).collect();
                let branch = self.branch.clone();
                let rewritten = steps.len();
                cx.act_recording(
                    format!("Rewrite commits on {branch}"),
                    "Interactive rebase",
                    nicegit_core::undo::UndoMode::Keep,
                    move |client, repo| {
                        client.interactive_rebase(&steps, &plan, &branch, &head, repo)?;
                        Ok(Some(format!("Rewrote {rewritten} commits on {branch}.")))
                    },
                );
                self.closing = true;
            }
        }
    }

    fn wants_close(&self) -> bool {
        self.closing
    }
}
