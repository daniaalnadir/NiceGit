//! Compares a branch with the checked-out branch: the commits that only one of them has, and the
//! files that merging the other branch would change.

use egui::{Margin, RichText, Sense, Ui};
use egui_phosphor::regular as icon;
use nicegit_core::branch_compare::{BranchComparison, ComparedCommit};
use nicegit_core::{short, GitError, Snapshot};

use crate::theme;
use crate::tools::compare::CompareWindow;
use crate::tools::{query, widgets, Ctx, Task, ToolWindow};

pub struct BranchCompareWindow {
    /// The other branch's display name.
    other: String,
    /// The other branch's tip as it was displayed when the window opened.
    tip: String,
    /// The checked-out branch, as the snapshot names it.
    current: String,
    /// The comparison, read in the background.
    loading: Option<Task<nicegit_core::Result<BranchComparison>>>,
    /// Set when the repository changed, so the comparison is read again on the next frame.
    needs_load: bool,
    comparison: Option<nicegit_core::Result<BranchComparison>>,
}

impl BranchCompareWindow {
    pub fn new(other: String, tip: String, current: String) -> Self {
        Self { other, tip, current, loading: None, needs_load: true, comparison: None }
    }

    /// The two summary counts, such as "1 commit on feature that main does not have".
    fn summary(&self, comparison: &BranchComparison) -> String {
        let count = |listed: usize, more: bool| {
            let text = if more { format!("{listed}+") } else { listed.to_string() };
            let noun = if listed == 1 && !more { "commit" } else { "commits" };
            format!("{text} {noun}")
        };
        format!(
            "{} on {} that {} does not have · {} on {} that {} does not have",
            count(comparison.only_in_other.len(), comparison.more_in_other),
            self.other,
            self.current,
            count(comparison.only_in_current.len(), comparison.more_in_current),
            self.current,
            self.other,
        )
    }

    /// The summary, the note for unrelated histories, and the changed-files button.
    fn header(&self, ui: &mut Ui, cx: &mut Ctx, comparison: &BranchComparison) {
        let c = theme::of(ui);
        ui.label(RichText::new(self.summary(comparison)).strong());
        if comparison.base.is_none() {
            ui.label(RichText::new("These branches share no history.").small().color(c.muted));
        }
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            let button = widgets::labeled_button(ui, icon::GIT_DIFF, "Changed files…", comparison.base.is_some())
                .on_hover_text(format!("The files that merging {} would change", self.other));
            if button.clicked() {
                if let Some(base) = &comparison.base {
                    cx.open(Box::new(CompareWindow::new(base.clone(), Some(self.tip.clone()))));
                }
            }
        });
    }

    /// One side's commits, under a heading that names the branch each commit is only on.
    fn section(&self, ui: &mut Ui, cx: &mut Ctx, heading: &str, commits: &[ComparedCommit], more: bool) {
        let c = theme::of(ui);
        ui.label(RichText::new(heading).strong());
        if commits.is_empty() {
            ui.label(RichText::new("No commits.").small().color(c.muted));
        }
        for commit in commits {
            commit_row(ui, cx, commit);
        }
        if more {
            ui.label(RichText::new(format!("Only the newest {} are listed.", commits.len())).small().color(c.muted));
        }
        ui.add_space(12.0);
    }
}

impl ToolWindow for BranchCompareWindow {
    fn id(&self) -> String {
        format!("branch-compare:{}:{}", self.other, self.tip)
    }

    fn title(&self) -> String {
        format!("{} and {}", self.other, self.current)
    }

    fn default_size(&self) -> egui::Vec2 {
        egui::vec2(720.0, 560.0)
    }

    fn repository_changed(&mut self, snapshot: &Snapshot) {
        self.current = snapshot.current_branch.clone();
        self.needs_load = true;
    }

    fn ui(&mut self, ui: &mut Ui, cx: &mut Ctx) {
        if self.needs_load && self.loading.is_none() {
            self.needs_load = false;
            let tip = self.tip.clone();
            self.loading = Some(query(ui.ctx(), cx.repo, move |client, directory| client.compare_branch(&tip, directory)));
        }
        if let Some(result) = self.loading.as_mut().and_then(|task| task.get().cloned()) {
            self.loading = None;
            self.comparison = Some(result);
        }

        match &self.comparison {
            None => widgets::loading(ui, "Comparing the branches…"),
            Some(Err(error)) => widgets::error(ui, &message(error)),
            Some(Ok(comparison)) => {
                self.header(ui, cx, comparison);
                ui.separator();
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                    let other_heading = format!("On {} but not on {}", self.other, self.current);
                    self.section(ui, cx, &other_heading, &comparison.only_in_other, comparison.more_in_other);
                    let current_heading = format!("On {} but not on {}", self.current, self.other);
                    self.section(ui, cx, &current_heading, &comparison.only_in_current, comparison.more_in_current);
                });
            }
        }
    }
}

/// A commit that selects itself in the graph when clicked.
fn commit_row(ui: &mut Ui, cx: &mut Ctx, commit: &ComparedCommit) {
    let c = theme::of(ui);
    let response = egui::Frame::new()
        .corner_radius(6.0)
        .inner_margin(Margin::symmetric(8, 5))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(RichText::new(short(&commit.hash)).monospace().small().color(c.muted));
                ui.add(egui::Label::new(RichText::new(&commit.subject)).truncate());
            });
            ui.add(
                egui::Label::new(RichText::new(format!("{} · {}", commit.author, commit.relative_date)).small().color(c.muted)).truncate(),
            );
        })
        .response
        .interact(Sense::click());
    if response.clicked() {
        cx.select_commit(commit.hash.clone());
    }
}

fn message(error: &GitError) -> String {
    match error {
        GitError::CommandFailed { message, .. } => message.clone(),
        other => other.to_string(),
    }
}
