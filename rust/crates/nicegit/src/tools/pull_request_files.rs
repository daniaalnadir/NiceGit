//! The Files tab of a pull request's detail: the changed files, the diff of the file selected, and
//! the review threads on its lines. A person can reply to a thread, start a comment on a line, and
//! resolve or reopen a thread. Each of those opens a confirmation that names the repository, the
//! pull request, and the place, and nothing is posted until the person confirms it there. The
//! detail window runs a confirmed write; this module only draws and collects what was asked.

use std::collections::HashSet;

use egui::{Color32, RichText, Stroke, Ui};
use egui_phosphor::regular as icon;
use nicegit_core::diff::{DiffLine, DiffLineKind};
use nicegit_core::github::{relative_time, PullRequestFile};
use nicegit_core::github_review::{
    anchor_index, comment_body, comment_target, FileDiff, NewLineComment, ReviewSide, ReviewThread, ThreadComment,
};

use crate::i18n::{tr, tr_fmt};
use crate::theme;
use crate::tools::github::message;
use crate::tools::pull_request_detail::footer;
use crate::tools::widgets;

/// Diff rows drawn for one file. A longer file shows its first rows and says so.
const ROW_LIMIT: usize = 5_000;

/// The longest part of a comment a confirmation quotes, in characters.
const EXCERPT_LIMIT: usize = 80;

/// The pull request a write is for, as the window shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pull {
    /// The repository as `host/owner/name`.
    pub repository: String,
    pub number: u64,
    pub title: String,
    /// The head commit a new line comment is written against.
    pub head_oid: String,
}

/// What a confirmed write does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Write {
    Reply { thread_id: String, body: String },
    Line(NewLineComment),
    Resolve { thread_id: String, resolved: bool },
}

/// A write waiting for the person's confirmation, with what the confirmation names.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Confirm {
    pub pull: Pull,
    /// The file and line, or the file alone for a thread with no line.
    pub location: String,
    /// The start of the text to post, for a reply or a comment.
    pub excerpt: Option<String>,
    pub write: Write,
}

/// Where the comment being written will go.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Target {
    /// A reply to the thread with this ID.
    Reply(String),
    /// A new comment on a line of the diff.
    Line { path: String, side: ReviewSide, line: u32 },
}

/// A comment being written, and its text so far.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Composer {
    target: Target,
    text: String,
}

/// What the Files tab keeps between frames.
#[derive(Default)]
pub struct FilesState {
    /// The path of the file whose diff is shown.
    selected: Option<String>,
    /// Resolved threads shown in full, by thread ID.
    expanded: HashSet<String>,
    /// The comment being written, if any. Only one is open at a time.
    composer: Option<Composer>,
}

/// What the Files tab shows this frame.
pub struct FilesView<'a> {
    pub pull: &'a Pull,
    pub files: &'a [PullRequestFile],
    /// The diff, as far as it has been read. `None` while it loads.
    pub diff: Option<&'a nicegit_core::Result<Vec<FileDiff>>>,
    /// The review threads, as far as they have been read. `None` while they load.
    pub threads: Option<&'a nicegit_core::Result<Vec<ReviewThread>>>,
    /// Why replies, comments, and resolve changes are refused, or `None` when they can be confirmed.
    pub refusal: Option<&'static str>,
    /// The current time in Unix seconds, for relative times.
    pub now: i64,
}

/// Draws the Files tab and returns the write the person asked to confirm, if they asked for one.
pub fn show(ui: &mut Ui, state: &mut FilesState, view: &FilesView) -> Option<Confirm> {
    let mut files = Files { state, view, confirm: None };
    files.draw(ui);
    files.confirm
}

/// The tab's drawing state for one frame.
struct Files<'s, 'v, 'a> {
    state: &'s mut FilesState,
    view: &'v FilesView<'a>,
    confirm: Option<Confirm>,
}

impl FilesState {
    /// Closes the comment being written, once it has been posted.
    pub fn clear_composer(&mut self) {
        self.composer = None;
    }
}

impl Files<'_, '_, '_> {
    fn draw(&mut self, ui: &mut Ui) {
        let c = theme::of(ui);
        if self.view.files.is_empty() {
            ui.label(RichText::new(tr("No files changed.")).color(c.muted));
            return;
        }
        for file in self.view.files {
            self.file_row(ui, file);
        }
        ui.add_space(8.0);
        let Some(path) = self.state.selected.clone() else {
            ui.label(RichText::new(tr("Choose a file to see its changes and review comments.")).color(c.muted));
            return;
        };
        ui.add(egui::Label::new(RichText::new(clean(&path)).monospace().strong()).wrap());
        ui.add_space(4.0);
        let lines: &[DiffLine] = match self.view.diff {
            None => {
                widgets::loading(ui, tr("Loading the diff"));
                &[]
            }
            Some(Err(error)) => {
                widgets::callout(ui, &message(error), true);
                &[]
            }
            Some(Ok(diffs)) => match diffs.iter().find(|file| file.path == path) {
                Some(file) => file.lines.as_slice(),
                None => {
                    ui.label(RichText::new(tr("The diff has no lines to show for this file.")).small().color(c.muted));
                    &[]
                }
            },
        };
        match self.view.threads {
            None => widgets::loading(ui, tr("Loading review comments")),
            Some(Err(error)) => widgets::callout(ui, &message(error), true),
            Some(Ok(threads)) => {
                let on_file: Vec<&ReviewThread> = threads.iter().filter(|thread| thread.path == path).collect();
                self.lines_and_threads(ui, &path, lines, &on_file);
            }
        }
    }

    /// The file's threads with no line, its diff with the threads on each line under it, and the
    /// threads on lines the diff does not show.
    fn lines_and_threads(&mut self, ui: &mut Ui, path: &str, lines: &[DiffLine], threads: &[&ReviewThread]) {
        let mut anchored: Vec<Vec<&ReviewThread>> = vec![Vec::new(); lines.len()];
        let mut outdated = Vec::new();
        let mut elsewhere = Vec::new();
        for thread in threads {
            match thread.line {
                None => outdated.push(*thread),
                Some(line) => match anchor_index(lines, thread.side, line) {
                    Some(index) => anchored[index].push(*thread),
                    None => elsewhere.push(*thread),
                },
            }
        }
        if !outdated.is_empty() {
            widgets::section(ui, tr("Outdated comments"));
            ui.add_space(2.0);
            for thread in outdated {
                self.card(ui, path, thread);
            }
        }

        // A new comment on a line is drawn under that line, wherever the diff shows it.
        let composer_row = match self.state.composer.as_ref().map(|composer| &composer.target) {
            Some(Target::Line { path: at, side, line }) if at == path => anchor_index(lines, *side, *line),
            _ => None,
        };
        for (index, line) in lines.iter().take(ROW_LIMIT).enumerate() {
            self.diff_row(ui, path, index, line);
            for thread in &anchored[index] {
                self.card(ui, path, thread);
            }
            if composer_row == Some(index) {
                self.new_comment(ui, path);
            }
        }
        if lines.len() > ROW_LIMIT {
            ui.label(
                RichText::new(tr_fmt!("Showing the first {count} lines of this file.", count = ROW_LIMIT))
                    .small()
                    .color(theme::of(ui).muted),
            );
        }

        if !elsewhere.is_empty() {
            ui.add_space(6.0);
            widgets::section(ui, tr("Comments on lines not in the diff"));
            ui.add_space(2.0);
            for thread in elsewhere {
                self.card(ui, path, thread);
            }
        }
    }

    /// A row of the file list. Choosing a file shows its diff and closes any comment being written.
    fn file_row(&mut self, ui: &mut Ui, file: &PullRequestFile) {
        let c = theme::of(ui);
        let selected = self.state.selected.as_deref() == Some(file.path.as_str());
        ui.horizontal(|ui| {
            if ui.add(egui::Button::selectable(selected, RichText::new(clean(&file.path)).monospace())).clicked() && !selected {
                self.state.selected = Some(file.path.clone());
                self.state.composer = None;
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(RichText::new(format!("\u{2212}{}", file.deletions)).small().color(c.removed));
                ui.label(RichText::new(format!("+{}", file.additions)).small().color(c.added));
            });
        });
    }

    /// One line of the diff, with its old and new numbers. While the pointer is over it, and a
    /// comment can be written, a "+" button starts a comment on the line.
    fn diff_row(&mut self, ui: &mut Ui, path: &str, index: usize, line: &DiffLine) {
        let c = theme::of(ui);
        let (text_color, background) = match line.kind {
            DiffLineKind::Addition => (c.added, Some(c.added_bg)),
            DiffLineKind::Deletion => (c.removed, Some(c.removed_bg)),
            DiffLineKind::Hunk => (c.muted, Some(c.subtle_bg)),
            DiffLineKind::Context => (ui.visuals().text_color(), None),
            DiffLineKind::Metadata => (c.muted, None),
        };
        // The pointer is read from the last frame, so the button is drawn once the row is under it.
        let hover_id = ui.scope_id().with(("pull request diff row", index));
        let hovered = ui.data(|data| data.get_temp::<bool>(hover_id)).unwrap_or(false);
        let target = comment_target(line);
        let can_comment = self.view.refusal.is_none() && target.is_some();
        let frame = egui::Frame::new().fill(background.unwrap_or(Color32::TRANSPARENT));
        let response = frame.show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                match target {
                    Some((side, number)) if hovered && can_comment => {
                        let button = ui.add_sized([22.0, 16.0], egui::Button::new(RichText::new(icon::PLUS).small()));
                        button.widget_info(|| {
                            egui::WidgetInfo::labeled(
                                egui::accesskit::Role::Button,
                                true,
                                tr_fmt!("Comment on line {number}", number = number),
                            )
                        });
                        if button.on_hover_text(tr("Comment on this line")).clicked() {
                            self.state.composer =
                                Some(Composer { target: Target::Line { path: path.to_string(), side, line: number }, text: String::new() });
                        }
                    }
                    _ => {
                        ui.add_space(26.0);
                    }
                }
                number_cell(ui, line.old_number, c.muted);
                number_cell(ui, line.new_number, c.muted);
                ui.add(egui::Label::new(RichText::new(display_text(&line.text)).monospace().color(text_color)).wrap());
            });
        });
        let now_hovered = ui.rect_contains_pointer(response.response.rect);
        ui.data_mut(|data| data.insert_temp(hover_id, now_hovered));
    }

    /// A review thread: its comments, its state, and the buttons that reply to it or resolve it. A
    /// resolved thread is one line until it is shown.
    fn card(&mut self, ui: &mut Ui, path: &str, thread: &ReviewThread) {
        let c = theme::of(ui);
        let now = self.view.now;
        let location = location_of(path, thread.line);
        egui::Frame::new().fill(c.card_bg).stroke(Stroke::new(1.0, c.border)).corner_radius(6.0).inner_margin(egui::Margin::same(8)).show(
            ui,
            |ui| {
                ui.set_width(ui.available_width());
                if thread.resolved && !self.state.expanded.contains(&thread.id) {
                    let author = thread.comments.first().and_then(|comment| comment.author.as_deref()).unwrap_or(tr("someone"));
                    ui.horizontal_wrapped(|ui| {
                        ui.label(RichText::new(tr_fmt!("Resolved thread by {author}", author = author)).small().color(c.muted));
                        if ui.link(tr("Show")).clicked() {
                            self.state.expanded.insert(thread.id.clone());
                        }
                    });
                    return;
                }
                for comment in &thread.comments {
                    comment_block(ui, comment, now);
                }
                if thread.resolved {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(tr("Resolved")).small().color(c.muted));
                        if ui.link(tr("Hide")).clicked() {
                            self.state.expanded.remove(&thread.id);
                        }
                    });
                }
                self.thread_actions(ui, thread, &location);
            },
        );
        ui.add_space(6.0);
    }

    /// The Reply and Resolve buttons of a thread, and the reply box when it is the one being answered.
    fn thread_actions(&mut self, ui: &mut Ui, thread: &ReviewThread, location: &str) {
        let refusal = self.view.refusal;
        let hover = refusal.unwrap_or(tr("Reply to this thread."));
        ui.horizontal_wrapped(|ui| {
            let reply = widgets::labeled_button(ui, icon::ARROW_BEND_UP_LEFT, tr("Reply"), refusal.is_none())
                .on_hover_text(hover)
                .on_disabled_hover_text(hover);
            if reply.clicked() {
                self.state.composer = Some(Composer { target: Target::Reply(thread.id.clone()), text: String::new() });
            }
            let (word, glyph) =
                if thread.resolved { (tr("Unresolve"), icon::ARROW_COUNTER_CLOCKWISE) } else { (tr("Resolve"), icon::CHECK_CIRCLE) };
            let resolve = widgets::labeled_button(ui, glyph, word, refusal.is_none()).on_hover_text(hover).on_disabled_hover_text(hover);
            if resolve.clicked() {
                self.confirm = Some(Confirm {
                    pull: self.view.pull.clone(),
                    location: location.to_string(),
                    excerpt: None,
                    write: Write::Resolve { thread_id: thread.id.clone(), resolved: !thread.resolved },
                });
            }
        });
        let replying = matches!(self.state.composer.as_ref().map(|composer| &composer.target), Some(Target::Reply(id)) if *id == thread.id);
        if replying {
            ui.add_space(4.0);
            if self.composer_box(ui, tr("Reply text"), tr("Write a reply"), tr("Post reply")) {
                let body = self.composer_body();
                self.confirm = Some(self.confirm_for(location, Write::Reply { thread_id: thread.id.clone(), body }));
            }
        }
    }

    /// The box under a line for a new comment on it.
    fn new_comment(&mut self, ui: &mut Ui, path: &str) {
        let (side, line) = match self.state.composer.as_ref().map(|composer| &composer.target) {
            Some(Target::Line { side, line, .. }) => (*side, *line),
            _ => return,
        };
        let location = location_of(path, Some(line));
        ui.add_space(4.0);
        egui::Frame::new().fill(theme::of(ui).card_bg).inner_margin(egui::Margin::same(8)).corner_radius(6.0).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(tr_fmt!("Comment on {location}", location = location)).small().color(theme::of(ui).muted));
            if self.composer_box(ui, tr("Comment text"), tr("Write a comment"), tr("Comment")) {
                let body = self.composer_body();
                let comment = NewLineComment { head_oid: self.view.pull.head_oid.clone(), path: path.to_string(), line, side, body };
                self.confirm = Some(self.confirm_for(&location, Write::Line(comment)));
            }
        });
        ui.add_space(6.0);
    }

    /// The text of the open composer, which is checked before it is posted.
    fn composer_body(&self) -> String {
        self.state.composer.as_ref().and_then(|composer| comment_body(&composer.text).ok()).unwrap_or_default()
    }

    /// The text box and its buttons for the open composer. Returns true when the person asked to
    /// post it, which needs a non-empty comment and a pull request that can take one. The text
    /// stays open until the write succeeds, so a cancelled confirmation keeps it.
    fn composer_box(&mut self, ui: &mut Ui, label: &str, hint: &str, submit: &str) -> bool {
        let Some(composer) = self.state.composer.as_mut() else { return false };
        let edit = ui.add(egui::TextEdit::multiline(&mut composer.text).hint_text(hint).desired_rows(3).desired_width(f32::INFINITY));
        edit.widget_info(|| egui::WidgetInfo::labeled(egui::accesskit::Role::MultilineTextInput, true, label));
        let ready =
            self.view.refusal.is_none() && self.state.composer.as_ref().is_some_and(|composer| comment_body(&composer.text).is_ok());
        let mut posted = false;
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            if widgets::primary_button(ui, submit, ready).clicked() {
                posted = true;
            }
            if ui.button(tr("Cancel")).clicked() {
                self.state.composer = None;
            }
        });
        posted
    }

    /// The confirmation for `write`, naming the pull request, the place, and the start of the text.
    fn confirm_for(&self, location: &str, write: Write) -> Confirm {
        let excerpt = match &write {
            Write::Reply { body, .. } => Some(excerpt(body)),
            Write::Line(comment) => Some(excerpt(&comment.body)),
            Write::Resolve { .. } => None,
        };
        Confirm { pull: self.view.pull.clone(), location: location.to_string(), excerpt, write }
    }
}

/// One comment of a thread: its author, how long ago it was written, and its text.
fn comment_block(ui: &mut Ui, comment: &ThreadComment, now: i64) {
    let c = theme::of(ui);
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new(comment.author.as_deref().unwrap_or(tr("Unknown author"))).strong());
        if let Some(ago) = relative_time(&comment.created_at, now) {
            ui.label(RichText::new(ago).small().color(c.muted));
        }
    });
    if !comment.body.is_empty() {
        ui.add(egui::Label::new(RichText::new(comment.body.as_str())).wrap().selectable(true));
    }
    ui.add_space(4.0);
}

/// One line number column of a diff row, blank when the line has no number on that side.
fn number_cell(ui: &mut Ui, number: Option<usize>, color: Color32) {
    let text = number.map(|number| number.to_string()).unwrap_or_default();
    ui.add_sized([40.0, 16.0], egui::Label::new(RichText::new(text).monospace().small().color(color)));
}

/// Where a thread is: its file and line, or its file alone when it has no line.
fn location_of(path: &str, line: Option<u32>) -> String {
    match line {
        Some(line) => format!("{}:{line}", clean(path)),
        None => clean(path),
    }
}

/// Draws the confirmation of a write while one waits, and returns the write the person confirmed.
/// Cancel, Escape, and a click outside close it, but not while the write is running. `error` is
/// the failure of the last attempt, shown for the person to read.
pub fn confirmation(ctx: &egui::Context, confirm: &mut Option<Confirm>, busy: bool, error: Option<&str>) -> Option<Write> {
    let pending = confirm.as_ref()?;
    let response = egui::Modal::new(egui::Id::unique(("pull request thread confirmation", pending.pull.number)))
        .show(ctx, |ui| confirm_body(ui, pending, busy, error));
    let closing = response.should_close();
    let (submit, cancel) = response.inner;
    if (cancel || closing) && !busy {
        *confirm = None;
        return None;
    }
    submit
}

/// The body of a confirmation: what it names, the text it posts, and its buttons. Returns the write
/// to run when the person confirmed it, and whether Cancel was clicked.
fn confirm_body(ui: &mut Ui, pending: &Confirm, busy: bool, error: Option<&str>) -> (Option<Write>, bool) {
    let c = theme::of(ui);
    ui.set_width(460.0);
    ui.label(RichText::new(heading(&pending.write)).strong().size(16.0));
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.label(RichText::new(pending.pull.repository.as_str()).monospace().color(c.muted));
        ui.label(RichText::new(format!("#{}", pending.pull.number)).monospace().color(c.muted));
    });
    ui.add(egui::Label::new(RichText::new(pending.pull.title.as_str()).strong()).wrap());
    ui.add_space(6.0);
    ui.label(RichText::new(pending.location.as_str()).monospace().small());
    if let Some(excerpt) = &pending.excerpt {
        ui.add(egui::Label::new(RichText::new(excerpt.as_str()).color(c.muted)).wrap());
    }
    if let Some(error) = error {
        ui.add_space(6.0);
        widgets::callout(ui, error, true);
    }
    let (send, cancel) = footer(ui, submit_text(&pending.write), !busy, busy);
    (send.then(|| pending.write.clone()), cancel)
}

/// The heading of a confirmation, for what the write does.
fn heading(write: &Write) -> &'static str {
    match write {
        Write::Reply { .. } => tr("Reply to review thread"),
        Write::Line(_) => tr("Comment on line"),
        Write::Resolve { resolved: true, .. } => tr("Resolve review thread"),
        Write::Resolve { resolved: false, .. } => tr("Reopen review thread"),
    }
}

/// The text of the confirm button for a write.
fn submit_text(write: &Write) -> &'static str {
    match write {
        Write::Reply { .. } => tr("Post reply"),
        Write::Line(_) => tr("Post comment"),
        Write::Resolve { resolved: true, .. } => tr("Resolve"),
        Write::Resolve { resolved: false, .. } => tr("Unresolve"),
    }
}

/// The first characters of a comment, followed by an ellipsis when it was cut.
fn excerpt(text: &str) -> String {
    let mut shown: String = text.chars().take(EXCERPT_LIMIT).collect();
    if text.chars().count() > EXCERPT_LIMIT {
        shown.push('\u{2026}');
    }
    shown
}

/// A file path or name without control characters, for display only.
fn clean(text: &str) -> String {
    text.chars().map(|c| if c.is_control() { '\u{FFFD}' } else { c }).collect()
}

/// A diff line for display: tabs become four spaces, and other control characters are left out.
fn display_text(text: &str) -> String {
    let mut shown = String::with_capacity(text.len());
    for c in text.chars() {
        if c == '\t' {
            shown.push_str("    ");
        } else if !c.is_control() {
            shown.push(c);
        }
    }
    shown
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn excerpts_keep_eighty_characters_and_mark_a_cut() {
        assert_eq!(excerpt("short"), "short");
        let long = "x".repeat(EXCERPT_LIMIT + 5);
        let shown = excerpt(&long);
        assert_eq!(shown.chars().count(), EXCERPT_LIMIT + 1, "the ellipsis is one character");
        assert!(shown.ends_with('\u{2026}'));
    }

    #[test]
    fn display_text_expands_tabs_and_drops_other_control_characters() {
        assert_eq!(display_text("+\tlet x\u{1b}[0m"), "+    let x[0m");
    }

    #[test]
    fn locations_name_the_line_or_only_the_file() {
        assert_eq!(location_of("src/a.rs", Some(12)), "src/a.rs:12");
        assert_eq!(location_of("src/a.rs", None), "src/a.rs");
    }
}
