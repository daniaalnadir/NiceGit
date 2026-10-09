//! Interactive rebase editor: choose what happens to each commit after a base, reorder them, and
//! reword, squash, fix up, or drop them. Commits are listed newest first, the way history reads;
//! the steps given to Git are built oldest first, as Git applies them.

#![allow(dead_code)]

use std::collections::BTreeSet;

use egui::{Id, RichText, Stroke, TextStyle};
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
            Choice::Squash => "Combine into the nearest kept commit below, keeping both messages",
            Choice::Fixup => "Combine into the nearest kept commit below, discarding this message",
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

/// The payload of a row being dragged: its index in the list when the drag started.
#[derive(Clone, Copy)]
struct DraggedRow(usize);

/// Edits the commits after `base` through HEAD. The branch and HEAD are captured when the window
/// opens, so the rewrite refuses if the checkout changed while the editor was open.
pub struct InteractiveRebaseWindow {
    base: String,
    branch: String,
    head: Option<String>,
    load: Option<Task<nicegit_core::Result<RebasePlan>>>,
    plan: Option<RebasePlan>,
    /// The entries as first loaded, newest first, for Reset edits and change detection.
    original: Vec<Entry>,
    /// The entries as shown, newest first.
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
                // The plan is oldest first; the editor shows newest first.
                let entries: Vec<Entry> = plan
                    .commits
                    .iter()
                    .rev()
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
        if oldest_kept_combines(&self.entries) {
            return Some("The oldest kept commit has no earlier commit to combine into. Pick or reword it instead.");
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

    /// The steps for Git, oldest first.
    fn steps(&self) -> Vec<RebaseStep> {
        oldest_first_steps(&self.entries)
    }

    /// Draws the row at `index`. A drag over it shows an insertion line; a release records the
    /// move in `moved` as (from, gap), applied after the list is drawn.
    fn row(
        entries: &mut [Entry],
        index: usize,
        published: &BTreeSet<String>,
        idle: bool,
        moved: &mut Option<(usize, usize)>,
        ui: &mut egui::Ui,
    ) {
        let c = theme::of(ui);
        let count = entries.len();
        let entry = &mut entries[index];
        let response = egui::Frame::new()
            .fill(c.card_bg)
            .stroke(egui::Stroke::new(1.0, c.border))
            .corner_radius(6.0)
            .inner_margin(egui::Margin::symmetric(10, 6))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    if idle {
                        ui.dnd_drag_source(Id::new(("interactive-rebase-row", entry.commit.hash.as_str())), DraggedRow(index), |ui| {
                            ui.label(RichText::new(icon::DOTS_SIX_VERTICAL).color(c.muted));
                        })
                        .response
                        .on_hover_text("Drag to reorder");
                    } else {
                        ui.label(RichText::new(icon::DOTS_SIX_VERTICAL).color(c.muted));
                    }
                    ui.label(RichText::new(format!("{}", index + 1)).monospace().color(c.muted));
                    if move_button(ui, icon::ARROW_UP, "Move up", idle && index > 0).clicked() {
                        // The slot above the row before this one.
                        *moved = Some((index, index - 1));
                    }
                    if move_button(ui, icon::ARROW_DOWN, "Move down", idle && index + 1 < count).clicked() {
                        // The slot below the row after this one.
                        *moved = Some((index, index + 2));
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
                        ui.label(RichText::new(icon::ARROW_BEND_DOWN_RIGHT).color(c.muted)).on_hover_text("Combined into the commit below");
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if published.contains(&entry.commit.hash) {
                            ui.add(egui::Label::new(RichText::new(icon::NETWORK).color(c.warning)).sense(egui::Sense::hover()))
                                .on_hover_text("Already on a remote");
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
            })
            .response;

        // A dragged row shows where it would land: a line above or below this row, by pointer half.
        if let Some(from) = response.dnd_hover_payload::<DraggedRow>().map(|row| row.0) {
            if let Some(pointer) = ui.ctx().input(|input| input.pointer.interact_pos()) {
                let gap = gap_for_pointer(index, response.rect.top(), response.rect.bottom(), pointer.y);
                if !is_noop_gap(from, gap) {
                    let y = if gap == index { response.rect.top() } else { response.rect.bottom() };
                    ui.painter().hline(response.rect.x_range(), y, Stroke::new(2.0, c.accent));
                }
                if response.dnd_release_payload::<DraggedRow>().is_some() {
                    *moved = Some((from, gap));
                }
            }
        }
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
        let count = self.entries.len();
        let commits = if count == 1 { "commit" } else { "commits" };
        ui.label(
            RichText::new(format!(
                "Rewrite {count} {commits} on {}. Listed newest first; drag a row by its grip or use the arrows to reorder. Squash and fixup combine a commit into the nearest kept commit below it.",
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
            let mut moved: Option<(usize, usize)> = None;
            let published = self.plan.as_ref().map(|plan| plan.published_commits.clone()).unwrap_or_default();
            // Leave room below the list for the summary and buttons.
            let list_height = (ui.available_height() - 110.0).max(120.0);
            egui::ScrollArea::vertical().auto_shrink([false, false]).max_height(list_height).show(ui, |ui| {
                for index in 0..self.entries.len() {
                    Self::row(&mut self.entries, index, &published, idle, &mut moved, ui);
                }
            });
            if self.published_count() > 0 {
                ui.label(RichText::new(format!("{}  Already on a remote", icon::NETWORK)).small().color(c.warning));
            }
            if let Some((from, gap)) = moved.filter(|_| idle) {
                move_to_gap(&mut self.entries, from, gap);
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
                let steps = self.steps();
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

/// A small icon button for moving a row. Enabled buttons use the normal text colour, so they stay
/// legible; disabled ones are muted.
fn move_button(ui: &mut egui::Ui, glyph: &str, tooltip: &str, enabled: bool) -> egui::Response {
    let color = if enabled { ui.visuals().text_color() } else { theme::of(ui).muted };
    ui.add_enabled(enabled, egui::Button::new(RichText::new(glyph).size(15.0).color(color)).frame(false).min_size(egui::vec2(24.0, 24.0)))
        .on_hover_text(tooltip)
        .on_disabled_hover_text(tooltip)
}

/// Whether dropping the row at `from` into insertion slot `gap` leaves the order unchanged.
/// Slot `i` lies above row `i`, and slot `len` lies below the last row.
fn is_noop_gap(from: usize, gap: usize) -> bool {
    gap == from || gap == from + 1
}

/// Moves the item at `from` into insertion slot `gap` (see [`is_noop_gap`]). Returns false when
/// nothing changes, including when either index is out of range.
fn move_to_gap<T>(items: &mut Vec<T>, from: usize, gap: usize) -> bool {
    if from >= items.len() || gap > items.len() || is_noop_gap(from, gap) {
        return false;
    }
    let item = items.remove(from);
    // Removing the item shifts every later slot down by one.
    let to = if gap > from { gap - 1 } else { gap };
    items.insert(to, item);
    true
}

/// The insertion slot above or below row `index`, for a pointer at `pointer_y` over that row:
/// the upper half of the row means above it, the lower half means below it.
fn gap_for_pointer(index: usize, top: f32, bottom: f32, pointer_y: f32) -> usize {
    if pointer_y < (top + bottom) / 2.0 {
        index
    } else {
        index + 1
    }
}

/// The steps for Git, oldest first. The entries are shown newest first, so they are reversed.
fn oldest_first_steps(entries: &[Entry]) -> Vec<RebaseStep> {
    entries.iter().rev().map(|entry| RebaseStep { commit: entry.commit.clone(), action: entry.action() }).collect()
}

/// Whether the oldest commit that is kept would be squashed or fixed up. Such a commit has no
/// earlier kept commit to combine into. `entries` are newest first.
fn oldest_kept_combines(entries: &[Entry]) -> bool {
    entries
        .iter()
        .rev()
        .find(|entry| entry.choice != Choice::Drop)
        .is_some_and(|entry| matches!(entry.choice, Choice::Squash | Choice::Fixup))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn commit(hash: &str) -> Commit {
        Commit {
            hash: hash.to_string(),
            short_hash: hash.chars().take(7).collect(),
            parents: Vec::new(),
            refs: Vec::new(),
            subject: format!("Subject {hash}"),
            author_name: "Author".to_string(),
            author_email: "author@example.com".to_string(),
            relative_date: "now".to_string(),
            commit_time: None,
        }
    }

    fn entry(hash: &str, choice: Choice) -> Entry {
        Entry { commit: commit(hash), choice, message: format!("Subject {hash}") }
    }

    fn hashes(items: &[&str]) -> Vec<String> {
        items.iter().map(|item| item.to_string()).collect()
    }

    #[test]
    fn moving_up_and_down_swaps_neighbours() {
        let mut items = hashes(&["a", "b", "c", "d"]);
        // Move "c" up: into the slot above "b".
        assert!(move_to_gap(&mut items, 2, 1));
        assert_eq!(items, hashes(&["a", "c", "b", "d"]));
        // Move "c" down: into the slot below "b" (above "d").
        assert!(move_to_gap(&mut items, 1, 3));
        assert_eq!(items, hashes(&["a", "b", "c", "d"]));
    }

    #[test]
    fn dropping_moves_across_several_rows_in_both_directions() {
        let mut items = hashes(&["a", "b", "c", "d", "e"]);
        // Drag "a" to the bottom of the list.
        assert!(move_to_gap(&mut items, 0, 5));
        assert_eq!(items, hashes(&["b", "c", "d", "e", "a"]));
        // Drag "a" to the top of the list.
        assert!(move_to_gap(&mut items, 4, 0));
        assert_eq!(items, hashes(&["a", "b", "c", "d", "e"]));
        // Drag "b" to just above "d".
        assert!(move_to_gap(&mut items, 1, 3));
        assert_eq!(items, hashes(&["a", "c", "b", "d", "e"]));
    }

    #[test]
    fn drops_onto_their_own_place_or_out_of_range_change_nothing() {
        let mut items = hashes(&["a", "b", "c"]);
        assert!(!move_to_gap(&mut items, 1, 1));
        assert!(!move_to_gap(&mut items, 1, 2));
        assert!(!move_to_gap(&mut items, 0, 4));
        assert!(!move_to_gap(&mut items, 3, 0));
        assert_eq!(items, hashes(&["a", "b", "c"]));
    }

    #[test]
    fn pointer_in_upper_half_inserts_above_and_lower_half_below() {
        assert_eq!(gap_for_pointer(2, 100.0, 140.0, 105.0), 2);
        assert_eq!(gap_for_pointer(2, 100.0, 140.0, 125.0), 3);
        assert_eq!(gap_for_pointer(0, 100.0, 140.0, 120.0), 1);
    }

    #[test]
    fn steps_are_oldest_first_when_entries_are_newest_first() {
        // Newest first: "c" is the tip, "a" the oldest.
        let entries = vec![entry("c", Choice::Pick), entry("b", Choice::Squash), entry("a", Choice::Pick)];
        let hashes_in_order: Vec<String> = oldest_first_steps(&entries).into_iter().map(|step| step.commit.hash).collect();
        assert_eq!(hashes_in_order, hashes(&["a", "b", "c"]));
        let actions: Vec<RebaseAction> = oldest_first_steps(&entries).into_iter().map(|step| step.action).collect();
        assert_eq!(actions, vec![RebaseAction::Pick, RebaseAction::Squash, RebaseAction::Pick]);
    }

    #[test]
    fn oldest_kept_commit_cannot_be_squashed_or_fixed_up() {
        // Oldest is "a"; it is kept and fixed up, so it has nothing to combine into.
        let entries = vec![entry("b", Choice::Pick), entry("a", Choice::Fixup)];
        assert!(oldest_kept_combines(&entries));

        // Dropped commits are skipped: "a" is dropped, so "b" is the oldest kept commit.
        let entries = vec![entry("c", Choice::Squash), entry("b", Choice::Pick), entry("a", Choice::Drop)];
        assert!(!oldest_kept_combines(&entries));

        // Squashing into a dropped commit is not the oldest kept commit either, when a kept one is older.
        let entries = vec![entry("c", Choice::Squash), entry("b", Choice::Drop), entry("a", Choice::Pick)];
        assert!(!oldest_kept_combines(&entries));

        // "b" would fold into the only commit below it, which is dropped, so nothing is kept to combine into.
        let entries = vec![entry("b", Choice::Squash), entry("a", Choice::Drop)];
        assert!(oldest_kept_combines(&entries));
    }
}
