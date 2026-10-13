//! Interactive rebase editor: choose what happens to each commit after a base, reorder them, and
//! reword, squash, fix up, or drop them. Commits are listed newest first, the way history reads;
//! the steps given to Git are built oldest first, as Git applies them.

#![allow(dead_code)]

use std::collections::BTreeSet;

use egui::{Id, RichText, Stroke, TextStyle};
use egui_phosphor::regular as icon;
use nicegit_core::models::Commit;
use nicegit_core::rebase::{RebaseAction, RebaseOutcome, RebasePlan, RebaseStep};
use nicegit_core::Snapshot;

use super::{query, widgets, Ctx, Task, ToolWindow};
use crate::theme;

/// What happens to one commit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Choice {
    Pick,
    Reword,
    Edit,
    Squash,
    Fixup,
    Drop,
}

impl Choice {
    const ALL: [Choice; 6] = [Choice::Pick, Choice::Reword, Choice::Edit, Choice::Squash, Choice::Fixup, Choice::Drop];

    fn title(self) -> &'static str {
        match self {
            Choice::Pick => "Pick",
            Choice::Reword => "Reword",
            Choice::Edit => "Edit",
            Choice::Squash => "Squash",
            Choice::Fixup => "Fixup",
            Choice::Drop => "Drop",
        }
    }

    fn help(self) -> &'static str {
        match self {
            Choice::Pick => "Keep this commit as it is",
            Choice::Reword => "Keep this commit with a new message",
            Choice::Edit => "Stop after this commit so you can change its files",
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
            Choice::Edit => RebaseAction::Edit,
            Choice::Squash => RebaseAction::Squash,
            Choice::Fixup => RebaseAction::Fixup,
            Choice::Drop => RebaseAction::Drop,
        }
    }
}

/// The payload of a row being dragged: its index in the list when the drag started.
#[derive(Clone, Copy)]
struct DraggedRow(usize);

/// How the plan starts. It is applied when the plan loads, so Reset returns to it.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Preset {
    /// Edit the message of one commit.
    Reword(String),
    /// Combine the commits with these hashes into one, at the oldest of them.
    Squash(Vec<String>),
    /// Remove the commits with these hashes.
    Drop(Vec<String>),
}

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
    /// The starting arrangement, applied when the plan loads.
    preset: Option<Preset>,
    /// Whether applying the preset moved commits, so the window warns that they may conflict.
    moved: bool,
    /// Whether the reworded commit's message should take keyboard focus on its first frame.
    focus_pending: bool,
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
            preset: None,
            moved: false,
            focus_pending: false,
        }
    }

    /// Opens the editor to change one commit's message: the commits after its parent are listed,
    /// with that commit set to Reword. A root commit rewrites from the root.
    pub fn reword(commit: &Commit, snapshot: &Snapshot) -> Self {
        let base = commit.parents.first().cloned().unwrap_or_default();
        Self { preset: Some(Preset::Reword(commit.hash.clone())), ..Self::new(base, snapshot) }
    }

    /// Opens the editor to combine the commits in `hashes` into one. The window lists the commits
    /// after the oldest one's parent, with the others set to Squash and moved next to the oldest.
    pub fn squash(hashes: &BTreeSet<String>, snapshot: &Snapshot) -> Self {
        Self::selected(hashes, snapshot, Preset::Squash)
    }

    /// Opens the editor to remove the commits in `hashes`, with their changes.
    pub fn drop_commits(hashes: &BTreeSet<String>, snapshot: &Snapshot) -> Self {
        Self::selected(hashes, snapshot, Preset::Drop)
    }

    /// A window for the commits in `hashes`, with `preset` applied. When they cannot be rewritten
    /// together (not all in the loaded history, or one is a merge), the window shows that instead.
    fn selected(hashes: &BTreeSet<String>, snapshot: &Snapshot, preset: fn(Vec<String>) -> Preset) -> Self {
        match selection_base(hashes, snapshot) {
            Some(base) => Self { preset: Some(preset(hashes.iter().cloned().collect())), ..Self::new(base, snapshot) },
            None => Self {
                error: Some("The selected commits must be in the loaded history, and none can be a merge.".to_string()),
                ..Self::new(String::new(), snapshot)
            },
        }
    }

    /// The hash of the commit whose message is being edited, if this window is a reword.
    fn reworded_hash(&self) -> Option<String> {
        match &self.preset {
            Some(Preset::Reword(hash)) => Some(hash.clone()),
            _ => None,
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
                let mut entries: Vec<Entry> = plan
                    .commits
                    .iter()
                    .rev()
                    .map(|commit| Entry {
                        commit: commit.clone(),
                        choice: Choice::Pick,
                        message: plan.messages.get(&commit.hash).cloned().unwrap_or_else(|| commit.subject.clone()),
                    })
                    .collect();
                // Preset before `original` is taken, so Reset returns to the preset and nothing
                // counts as changed until the message is edited.
                if let Some(preset) = self.preset.clone() {
                    match apply_preset(&mut entries, &preset, &self.branch) {
                        Ok(moved) => self.moved = moved,
                        Err(error) => {
                            self.error = Some(error);
                            return;
                        }
                    }
                    self.focus_pending = matches!(preset, Preset::Reword(_));
                }
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
        let kept = self.entries.iter().filter(|entry| matches!(entry.choice, Choice::Pick | Choice::Reword | Choice::Edit)).count();
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

    /// Whether Rewrite commits has something to apply. A squash or drop is itself the rewrite, so
    /// it can run straight away; a reword needs an edited message.
    fn has_pending_rewrite(&self) -> bool {
        self.changed() || matches!(self.preset, Some(Preset::Squash(_) | Preset::Drop(_)))
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
        focus: Option<&str>,
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
                        })
                        .response
                        .widget_info(|| {
                            egui::WidgetInfo::labeled(egui::WidgetType::ComboBox, idle, format!("Action for {}", entry.commit.subject))
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
                // What the choice does, on one truncated line under the row. For Reword it sits
                // above the message editor, which then reads as the thing it describes.
                if entry.choice != Choice::Pick {
                    ui.add(egui::Label::new(RichText::new(entry.choice.help()).small().color(c.muted)).truncate());
                }
                if entry.choice == Choice::Reword {
                    ui.add_space(4.0);
                    let message = ui.add(
                        egui::TextEdit::multiline(&mut entry.message)
                            .font(TextStyle::Monospace)
                            .desired_rows(4)
                            .desired_width(f32::INFINITY)
                            .hint_text("New commit message"),
                    );
                    message.widget_info(|| {
                        egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, format!("Message for {}", entry.commit.subject))
                    });
                    if focus == Some(entry.commit.hash.as_str()) {
                        message.request_focus();
                        // An older commit can be far down the list.
                        message.scroll_to_me(Some(egui::Align::Center));
                    }
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
        // Each kind of edit is its own window, so it never replaces a rebase from the same base.
        match &self.preset {
            Some(Preset::Reword(hash)) => format!("interactive-rebase-reword:{hash}"),
            Some(Preset::Squash(_)) => format!("interactive-rebase-squash:{}", self.base),
            Some(Preset::Drop(_)) => format!("interactive-rebase-drop:{}", self.base),
            None => format!("interactive-rebase:{}", self.base),
        }
    }

    fn title(&self) -> String {
        match &self.preset {
            Some(Preset::Reword(_)) => "Edit commit message",
            Some(Preset::Squash(_)) => "Squash commits",
            Some(Preset::Drop(_)) => "Drop commits",
            None => "Interactive rebase",
        }
        .to_string()
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
        if self.moved {
            ui.label(
                RichText::new("Selected commits were moved next to each other; moving commits can cause conflicts.").small().color(c.muted),
            );
            ui.add_space(8.0);
        }

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
            let focus = if self.focus_pending { self.reworded_hash() } else { None };
            egui::ScrollArea::vertical().auto_shrink([false, false]).max_height(list_height).show(ui, |ui| {
                for index in 0..self.entries.len() {
                    Self::row(&mut self.entries, index, &published, idle, focus.as_deref(), &mut moved, ui);
                }
            });
            // The first frame with the list is the only one that focuses the message.
            self.focus_pending = false;
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
            let ready = cx.idle && self.plan.is_some() && self.head.is_some() && problem.is_none() && self.has_pending_rewrite();
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
                // Git stops at the oldest Edit first, so its subject is the one the notice names.
                let stop_subject = steps.iter().find(|step| step.action == RebaseAction::Edit).map(|step| step.commit.subject.clone());
                let detail = format!("{rewritten} commit{}", if rewritten == 1 { "" } else { "s" });
                cx.act_recording(
                    format!("Rewrite commits on {branch}"),
                    "Interactive rebase",
                    Some(detail),
                    nicegit_core::undo::UndoMode::Keep,
                    move |client, repo| match client.interactive_rebase(&steps, &plan, &branch, &head, repo)? {
                        RebaseOutcome::Finished => Ok(Some(format!("Rewrote {rewritten} commits on {branch}."))),
                        RebaseOutcome::Stopped => {
                            let stopped_at = match stop_subject {
                                Some(subject) => format!("Stopped at “{subject}”"),
                                None => "Stopped".to_string(),
                            };
                            Ok(Some(format!(
                                "{stopped_at} so you can change it. Edit and stage files, then click Continue to fold them in; Abort undoes the whole rebase."
                            )))
                        }
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

/// Sets the entry for `hash` to Reword. Returns false when no entry has that hash.
fn preset_reword(entries: &mut [Entry], hash: &str) -> bool {
    match entries.iter_mut().find(|entry| entry.commit.hash == hash) {
        Some(entry) => {
            entry.choice = Choice::Reword;
            true
        }
        None => false,
    }
}

/// The base for rewriting the commits in `hashes`: the first parent of the oldest of them. Returns
/// `None` when one is not in the loaded history or is a merge, and an empty base when the oldest
/// is a root commit, so the rewrite starts from the root.
pub fn selection_base(hashes: &BTreeSet<String>, snapshot: &Snapshot) -> Option<String> {
    // The loaded history is in topological order, newest first, so the oldest has the highest index.
    let mut oldest: Option<usize> = None;
    for hash in hashes {
        let index = snapshot.commits.iter().position(|commit| &commit.hash == hash)?;
        if snapshot.commits[index].parents.len() > 1 {
            return None;
        }
        oldest = oldest.max(Some(index));
    }
    let commit = &snapshot.commits[oldest?];
    Some(commit.parents.first().cloned().unwrap_or_default())
}

/// Applies `preset` to `entries`, newest first. Returns whether any commit moved, or the message
/// to show when a commit the preset names is not in the list.
fn apply_preset(entries: &mut Vec<Entry>, preset: &Preset, branch: &str) -> Result<bool, String> {
    let missing = || format!("Some selected commits are not part of {branch}'s history.");
    match preset {
        Preset::Reword(hash) => {
            if preset_reword(entries, hash) {
                Ok(false)
            } else {
                // The commit is on another branch whose parent is in this one's history.
                Err(format!("This commit is not part of {branch}'s history."))
            }
        }
        Preset::Squash(hashes) => squash_together(entries, hashes).ok_or_else(missing),
        Preset::Drop(hashes) => drop_together(entries, hashes).ok_or_else(missing),
    }
}

/// Moves the commits in `hashes` next to each other, with the oldest kept and the rest set to
/// Squash. The others go directly above the oldest, in their original order. Commits that were
/// between them keep their order and end up above the group. Returns whether the order changed,
/// or `None`, leaving `entries` as it was, when a hash is not listed.
fn squash_together(entries: &mut Vec<Entry>, hashes: &[String]) -> Option<bool> {
    let mut marked: Vec<usize> =
        hashes.iter().map(|hash| entries.iter().position(|entry| &entry.commit.hash == hash)).collect::<Option<_>>()?;
    marked.sort_unstable();
    marked.dedup();
    // Entries are newest first, so the oldest marked commit has the highest index.
    let Some((&oldest, others)) = marked.split_last() else { return Some(false) };
    let original: Vec<&str> = entries.iter().map(|entry| entry.commit.hash.as_str()).collect();

    let mut arranged = Vec::with_capacity(entries.len());
    for (index, entry) in entries.iter().enumerate().take(oldest) {
        if marked.binary_search(&index).is_err() {
            arranged.push(entry.clone());
        }
    }
    // The others are listed newest first, as they appear above the oldest in the list.
    for &index in others {
        arranged.push(Entry { choice: Choice::Squash, ..entries[index].clone() });
    }
    arranged.push(Entry { choice: Choice::Pick, ..entries[oldest].clone() });
    arranged.extend(entries.iter().skip(oldest + 1).cloned());

    let moved = arranged.iter().map(|entry| entry.commit.hash.as_str()).ne(original);
    *entries = arranged;
    Some(moved)
}

/// Sets the commits in `hashes` to Drop. Returns `None`, leaving `entries` as it was, when a hash
/// is not listed. Dropping does not move commits.
fn drop_together(entries: &mut [Entry], hashes: &[String]) -> Option<bool> {
    let indices: Vec<usize> =
        hashes.iter().map(|hash| entries.iter().position(|entry| &entry.commit.hash == hash)).collect::<Option<_>>()?;
    for index in indices {
        entries[index].choice = Choice::Drop;
    }
    Some(false)
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
    fn preset_marks_only_the_named_commit_as_reword() {
        let mut entries = vec![entry("c", Choice::Pick), entry("b", Choice::Pick), entry("a", Choice::Pick)];
        assert!(preset_reword(&mut entries, "b"));
        let choices: Vec<Choice> = entries.iter().map(|entry| entry.choice).collect();
        assert_eq!(choices, vec![Choice::Pick, Choice::Reword, Choice::Pick]);
        // A commit that is not listed changes nothing.
        assert!(!preset_reword(&mut entries, "z"));
        assert_eq!(entries.iter().filter(|entry| entry.choice == Choice::Reword).count(), 1);
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

    #[test]
    fn edit_is_a_kept_commit_that_maps_to_an_edit_step() {
        assert_eq!(entry("a", Choice::Edit).action(), RebaseAction::Edit);
        // The edited commit is kept, so it is the oldest kept commit here and combines into nothing.
        let entries = vec![entry("b", Choice::Squash), entry("a", Choice::Edit)];
        assert!(!oldest_kept_combines(&entries));
        let steps: Vec<RebaseAction> = oldest_first_steps(&entries).into_iter().map(|step| step.action).collect();
        assert_eq!(steps, vec![RebaseAction::Edit, RebaseAction::Squash]);
    }

    /// The hash and action of each entry, newest first.
    fn shape(entries: &[Entry]) -> Vec<(&str, Choice)> {
        entries.iter().map(|entry| (entry.commit.hash.as_str(), entry.choice)).collect()
    }

    /// The entries "d", "c", "b", "a" in that order, newest first, all picked.
    fn four_picks() -> Vec<Entry> {
        vec![entry("d", Choice::Pick), entry("c", Choice::Pick), entry("b", Choice::Pick), entry("a", Choice::Pick)]
    }

    #[test]
    fn squash_of_two_adjacent_commits_keeps_their_order() {
        let mut entries = vec![entry("c", Choice::Pick), entry("b", Choice::Pick), entry("a", Choice::Pick)];
        let moved = apply_preset(&mut entries, &Preset::Squash(hashes(&["b", "c"])), "main");
        assert_eq!(moved, Ok(false), "adjacent commits do not move");
        assert_eq!(shape(&entries), [("c", Choice::Squash), ("b", Choice::Pick), ("a", Choice::Pick)]);
    }

    #[test]
    fn squash_of_two_non_adjacent_commits_moves_the_between_commit_above_the_group() {
        // "b" and "d" are squashed; "c" lies between them in history and moves above the group.
        let mut entries = four_picks();
        let moved = apply_preset(&mut entries, &Preset::Squash(hashes(&["b", "d"])), "main");
        assert_eq!(moved, Ok(true));
        assert_eq!(shape(&entries), [("c", Choice::Pick), ("d", Choice::Squash), ("b", Choice::Pick), ("a", Choice::Pick)]);
        // Oldest first, as Git applies them: "a", then "b" with "d" squashed into it, then "c".
        let order: Vec<(String, RebaseAction)> =
            oldest_first_steps(&entries).into_iter().map(|step| (step.commit.hash, step.action)).collect();
        assert_eq!(
            order,
            [
                ("a".to_string(), RebaseAction::Pick),
                ("b".to_string(), RebaseAction::Pick),
                ("d".to_string(), RebaseAction::Squash),
                ("c".to_string(), RebaseAction::Pick),
            ]
        );
    }

    #[test]
    fn squash_of_three_commits_keeps_their_relative_order() {
        // Newest first: "e", "d", "c", "b", "a". "a", "c", and "e" are squashed; "d" and "b" lie
        // between them and move above the group, keeping their order.
        let mut entries = vec![
            entry("e", Choice::Pick),
            entry("d", Choice::Pick),
            entry("c", Choice::Pick),
            entry("b", Choice::Pick),
            entry("a", Choice::Pick),
        ];
        let moved = apply_preset(&mut entries, &Preset::Squash(hashes(&["c", "a", "e"])), "main");
        assert_eq!(moved, Ok(true));
        assert_eq!(
            shape(&entries),
            [("d", Choice::Pick), ("b", Choice::Pick), ("e", Choice::Squash), ("c", Choice::Squash), ("a", Choice::Pick)]
        );
    }

    #[test]
    fn drop_of_two_commits_marks_both_and_moves_nothing() {
        let mut entries = vec![entry("c", Choice::Pick), entry("b", Choice::Pick), entry("a", Choice::Pick)];
        let moved = apply_preset(&mut entries, &Preset::Drop(hashes(&["a", "c"])), "main");
        assert_eq!(moved, Ok(false));
        assert_eq!(shape(&entries), [("c", Choice::Drop), ("b", Choice::Pick), ("a", Choice::Drop)]);
    }

    #[test]
    fn squash_or_drop_of_a_commit_outside_the_list_is_an_error() {
        let expected = Err("Some selected commits are not part of main's history.".to_string());

        let mut entries = four_picks();
        assert_eq!(apply_preset(&mut entries, &Preset::Squash(hashes(&["b", "z"])), "main"), expected);
        assert_eq!(entries, four_picks(), "a failed squash changes nothing");

        let mut entries = four_picks();
        assert_eq!(apply_preset(&mut entries, &Preset::Drop(hashes(&["z"])), "main"), expected);
        assert_eq!(entries, four_picks(), "a failed drop changes nothing");
    }

    #[test]
    fn reword_preset_reports_a_commit_outside_the_list_as_not_part_of_the_branch() {
        let mut entries = four_picks();
        assert_eq!(
            apply_preset(&mut entries, &Preset::Reword("z".to_string()), "main"),
            Err("This commit is not part of main's history.".to_string())
        );
        assert_eq!(apply_preset(&mut entries, &Preset::Reword("b".to_string()), "main"), Ok(false));
        assert_eq!(shape(&entries)[2], ("b", Choice::Reword));
    }
}
