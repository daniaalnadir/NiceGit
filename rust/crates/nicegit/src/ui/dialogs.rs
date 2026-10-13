//! Confirmations and short forms. Each captures what the user saw, so the action it runs can
//! refuse if the repository changed in the meantime.

use egui::{Key, RichText};
use egui_phosphor::regular as icon;
use nicegit_core::models::short;
use nicegit_core::{Branch, GitError, MergeStyle, Operation, Snapshot, Stash, StashOptions, StatusEntry};

use crate::app::NiceGitApp;
use crate::theme;
use crate::tools::widgets;
use crate::ui::clone_picker::{ClonePicker, Picked};

pub enum Pending {
    Discard(StatusEntry),
    DiscardAll(Vec<StatusEntry>),
    /// Discarding some of a file's unstaged lines, as indices into the review's lines.
    DiscardLines {
        lines: std::collections::BTreeSet<usize>,
        review: nicegit_core::staging::FileReview,
    },
    DeleteBranch {
        branch: Branch,
        force: bool,
    },
    DeleteTag {
        name: String,
        tip: String,
    },
    DropStash(Stash),
    Merge {
        source: Branch,
        branch: String,
        head: Option<String>,
        /// Chosen in the confirmation, which starts as the default. Not remembered between dialogs.
        style: MergeStyle,
    },
    Rebase {
        onto: Branch,
        branch: String,
        head: Option<String>,
    },
    CherryPick {
        commits: Vec<String>,
        branch: String,
        head: Option<String>,
    },
    Revert {
        commit: String,
        branch: String,
        head: Option<String>,
    },
    /// Reverting several marked commits as one sequence, newest first.
    RevertMany {
        commits: Vec<String>,
        branch: String,
        head: Option<String>,
    },
    /// Folding the staged changes into an older commit. The subject names it in the result.
    FixupInto {
        target: String,
        subject: String,
        branch: String,
        head: Option<String>,
    },
    Abort(Operation),
    /// Taking one side of every listed conflicted file. The paths are those shown when the
    /// confirmation opened; `side_name` is the side's name as the menus show it, in lower case.
    ResolveConflictsSide {
        paths: Vec<String>,
        incoming: bool,
        side_name: String,
    },
    UndoCommit {
        branch: String,
        head: String,
    },
    PushBranch {
        branch: Branch,
        remote: String,
        addresses: std::collections::BTreeMap<String, Vec<String>>,
    },
    /// Replacing the upstream branch with the current one. Everything shown is captured: the
    /// branch and HEAD, the upstream, the remote-tracking tip the user reviewed (the lease), and
    /// the remote's push addresses.
    ForcePush {
        branch: String,
        head: String,
        upstream: String,
        expected_remote_tip: String,
        addresses: std::collections::BTreeMap<String, Vec<String>>,
    },
    ApplyPatch {
        file: std::path::PathBuf,
        branch: String,
        head: Option<String>,
    },
    Undo,
    Redo,
    RestoreFile {
        path: String,
        source: String,
        branch: String,
        head: Option<String>,
    },
    /// Restoring one file to its version in a stash, captured with the checkout it was shown on.
    RestoreFromStash {
        path: String,
        stash: Stash,
        untracked: bool,
        branch: String,
        head: Option<String>,
    },
    DeleteRemoteTag {
        tag: String,
        tip: String,
        remote: String,
        addresses: std::collections::BTreeMap<String, Vec<String>>,
    },
    /// Deleting `branch` (named without its remote) from `remote`. `tip` is the remote-tracking
    /// tip shown when the branch was selected.
    DeleteRemoteBranch {
        remote: String,
        branch: String,
        tip: String,
        addresses: std::collections::BTreeMap<String, Vec<String>>,
    },
    PopStash(Stash),
}

pub enum InputKind {
    CreateBranch {
        branch: String,
        head: Option<String>,
    },
    CreateBranchFrom(Branch),
    /// A branch at the commit a stash was made on. The branch and HEAD are captured when the
    /// dialog opens, as Create branch's are.
    BranchFromStash {
        stash: Stash,
        branch: String,
        head: Option<String>,
    },
    RenameBranch(Branch),
    /// A lightweight tag, or an annotated one with a message. `push` is the remote to publish
    /// the tag to once it exists, or `None` to keep it local. The remotes and their push
    /// addresses are captured when the dialog opens, so the push checks what was shown.
    CreateTag {
        target: String,
        annotated: bool,
        /// Whether an annotated tag is signed with the user's signing key. It is kept while the tag is
        /// lightweight, but only an annotated tag is created signed.
        sign: bool,
        push: Option<String>,
        remotes: Vec<String>,
        push_addresses: std::collections::BTreeMap<String, Vec<String>>,
        /// The remote that is chosen when "Push to remote" is ticked.
        default_remote: Option<String>,
    },
    SaveStash,
    /// Cloning. The destination follows the address until the person edits it or chooses one.
    Clone(CloneDestination),
}

/// The Clone dialog's destination. While it is automatic it is the default folder joined with
/// the repository's name, and it follows the address as that is typed or picked.
#[derive(Clone, Default)]
pub struct CloneDestination {
    /// The destination last written from the default folder and the address.
    automatic: String,
    /// Set once the destination is typed or chosen; from then on the dialog leaves it alone.
    by_hand: bool,
}

impl CloneDestination {
    /// Writes the automatic destination for `url` into `destination`, unless it is the person's.
    fn follow(&mut self, folder: &std::path::Path, url: &str, destination: &mut String) {
        if self.by_hand || !(destination.is_empty() || *destination == self.automatic) {
            return;
        }
        let url = url.trim();
        let wanted = if url.is_empty() { String::new() } else { folder.join(repository_folder_name(url)).display().to_string() };
        *destination = wanted.clone();
        self.automatic = wanted;
    }
}

/// Names commits in a history detail: one commit by its short hash, several by their count.
fn commits_detail(commits: &[String]) -> String {
    match commits {
        [one] => short(one).to_string(),
        _ => format!("{} commits", commits.len()),
    }
}

/// The folder a clone of `url` is named after: its last path segment without `.git`, such as
/// `app` for `https://github.com/acme/app.git` or `git@github.com:acme/app.git`.
pub fn repository_folder_name(url: &str) -> String {
    let name = url.trim().trim_end_matches(['/', '\\']).rsplit(['/', ':', '\\']).next().unwrap_or_default();
    let name = name.strip_suffix(".git").unwrap_or(name);
    // `.` and `..` would name the default folder itself or its parent, not a new folder in it.
    if name.is_empty() || name == "." || name == ".." {
        "repository".into()
    } else {
        name.into()
    }
}

/// Whether `path` is taken, so a clone into it would fail: it exists and is not an empty folder.
fn is_taken(path: &std::path::Path) -> bool {
    match std::fs::read_dir(path) {
        Ok(mut entries) => entries.next().is_some(),
        Err(_) => path.exists(),
    }
}

pub enum Dialog {
    Confirm {
        title: String,
        message: String,
        button: String,
        pending: Pending,
    },
    Input {
        title: String,
        label: String,
        value: String,
        second_label: Option<String>,
        second: String,
        kind: InputKind,
    },
    Publish {
        remote: String,
    },
    /// A branch dropped onto the current branch: merge it in, or rebase onto it.
    Integrate {
        source: Branch,
    },
    /// Cherry-picking or reverting a merge needs the parent to compare against.
    ChooseParent {
        commit: String,
        subject: String,
        parents: Vec<(String, String)>,
        selected: usize,
        revert: bool,
        branch: String,
        head: Option<String>,
    },
    /// Rewrite the HEAD commit's message.
    EditMessage {
        message: String,
        branch: String,
        head: String,
    },
}

impl Dialog {
    pub fn input(title: &str, label: &str, value: &str, kind: InputKind) -> Self {
        Dialog::Input { title: title.into(), label: label.into(), value: value.into(), second_label: None, second: String::new(), kind }
    }

    pub fn with_second(title: &str, label: &str, second_label: &str, kind: InputKind) -> Self {
        Dialog::Input {
            title: title.into(),
            label: label.into(),
            value: String::new(),
            second_label: Some(second_label.into()),
            second: String::new(),
            kind,
        }
    }

    pub fn clone_repository() -> Self {
        Self::with_second(
            "Clone repository",
            "Repository URL or local path",
            "Destination folder",
            InputKind::Clone(CloneDestination::default()),
        )
    }

    /// The New tag dialog for `target`, capturing the snapshot's remotes and push addresses now.
    pub fn new_tag(target: String, snapshot: &Snapshot) -> Self {
        let remotes = snapshot.remotes.clone();
        let default_remote = default_push_remote(&remotes, snapshot.upstream.as_deref());
        Self::with_second(
            "New tag",
            "Tag name",
            "Tag message",
            InputKind::CreateTag {
                target,
                annotated: false,
                sign: false,
                push: None,
                remotes,
                push_addresses: snapshot.remote_push_addresses.clone(),
                default_remote,
            },
        )
    }
}

/// The remote a new tag is pushed to by default: the current branch's upstream remote, else
/// `origin`, else the first remote. `upstream` is a short name such as `origin/main`.
fn default_push_remote(remotes: &[String], upstream: Option<&str>) -> Option<String> {
    let upstream_remote = upstream.and_then(|upstream| {
        remotes
            .iter()
            .filter(|remote| upstream.strip_prefix(remote.as_str()).is_some_and(|rest| rest.starts_with('/')))
            .max_by_key(|remote| remote.len())
    });
    upstream_remote.or_else(|| remotes.iter().find(|remote| remote.as_str() == "origin")).or_else(|| remotes.first()).cloned()
}

impl Pending {
    fn destructive(&self) -> bool {
        matches!(
            self,
            Pending::Discard(_)
                | Pending::DiscardAll(_)
                | Pending::DeleteBranch { .. }
                | Pending::DeleteTag { .. }
                | Pending::DeleteRemoteBranch { .. }
                | Pending::DropStash(_)
                | Pending::RestoreFromStash { .. }
                | Pending::Abort(_)
                | Pending::ForcePush { .. }
                | Pending::ResolveConflictsSide { .. }
        )
    }
}

/// The three ways a merge can combine the branch, with a note under the chosen one.
fn merge_style_choices(ui: &mut egui::Ui, style: &mut MergeStyle) {
    let choices = [
        (MergeStyle::Default, "Merge (fast-forward when possible)", "Git follows this repository's merge.ff setting."),
        (
            MergeStyle::NoFastForward,
            "Always create a merge commit",
            "Records the merge with a merge commit even when the branch could move forward.",
        ),
        (
            MergeStyle::Squash,
            "Squash into staged changes",
            "Stages the branch's changes without committing, so you can commit them as one new commit. The branch is not recorded as merged.",
        ),
    ];
    for (choice, title, note) in choices {
        if widgets::radio(ui, *style == choice, title).clicked() {
            *style = choice;
        }
        if *style == choice {
            ui.indent(title, |ui| {
                ui.label(RichText::new(note).small().color(theme::of(ui).muted));
            });
        }
    }
}

/// The most conflicted files the confirmation lists by name.
const LISTED_CONFLICTS: usize = 15;

/// The confirmation for taking one side of every conflicted file in `paths`.
pub fn resolve_conflicts_confirmation(paths: &[String], side: &str) -> (String, String, String) {
    let title = format!("Take the {side} version for all conflicts?");
    // A side that deleted a file takes the deletion, so the file is removed rather than replaced.
    let mut message = format!(
        "Each file is replaced with its {side} version and marked resolved. Changes from the other side in these files are discarded. A file that the {side} side deleted is removed instead."
    );
    for path in paths.iter().take(LISTED_CONFLICTS) {
        message.push_str("\n• ");
        message.push_str(path);
    }
    if paths.len() > LISTED_CONFLICTS {
        message.push_str(&format!("\n…and {} more", paths.len() - LISTED_CONFLICTS));
    }
    (title, message, format!("Take {side} versions"))
}

/// The notice for a bulk resolution: how many files took the side, and which could not be
/// resolved, so the person can open them in the conflict editor.
fn resolve_conflicts_notice(report: &nicegit_core::conflict::ResolveReport, side: &str) -> String {
    let resolved = report.resolved.len();
    let mut text = format!("Resolved {resolved} {} with the {side} version.", if resolved == 1 { "file" } else { "files" });
    if !report.failed.is_empty() {
        let names: Vec<&str> = report.failed.iter().map(|(path, _)| path.as_str()).collect();
        text.push_str(&format!(" {} could not be resolved: {} (open them in the conflict editor).", report.failed.len(), names.join(", ")));
    }
    text
}

impl NiceGitApp {
    pub fn dialogs(&mut self, ctx: &egui::Context) {
        let Some(mut dialog) = self.dialog.take() else {
            // The GitHub picker belongs to the Clone dialog, so it closes with it.
            self.clone_picker = None;
            return;
        };
        let mut close = false;
        let mut run = false;
        let mut integrate: Option<bool> = None;
        // The Clone dialog's "From GitHub…" button asks for the picker, which is drawn over the dialog.
        let mut open_picker = false;
        let picker_open = self.clone_picker.is_some();
        let published = self.head_published();
        let mut snapshot_commits: Vec<(String, String)> = Vec::new();
        if let Some(snapshot) = self.snapshot() {
            snapshot_commits = snapshot.commits.iter().map(|c| (c.hash.clone(), c.subject.clone())).collect();
        }
        let modal = egui::Modal::new(egui::Id::new("dialog")).show(ctx, |ui| {
            ui.set_width(440.0);
            let c = theme::of(ui);
            match &mut dialog {
                Dialog::Confirm { title, message, button, pending } => {
                    ui.horizontal(|ui| {
                        let (glyph, color) = if pending.destructive() { (icon::WARNING, c.danger) } else { (icon::QUESTION, c.accent) };
                        ui.label(RichText::new(glyph).size(22.0).color(color));
                        ui.heading(title.as_str());
                    });
                    ui.add_space(8.0);
                    ui.label(message.as_str());
                    // A merge can be made in three ways, and the confirm button says what the chosen one does.
                    if let Pending::Merge { style, .. } = pending {
                        ui.add_space(12.0);
                        merge_style_choices(ui, style);
                        *button = if *style == MergeStyle::Squash { "Squash" } else { "Merge" }.to_string();
                    }
                    ui.add_space(16.0);
                    ui.horizontal(|ui| {
                        let confirm = if pending.destructive() {
                            widgets::danger_button(ui, button, true)
                        } else {
                            widgets::primary_button(ui, button, true)
                        };
                        if confirm.clicked() {
                            run = true;
                        }
                        if ui.button("Cancel").clicked() {
                            close = true;
                        }
                    });
                }
                Dialog::Input { title, label, value, second_label, second, kind } => {
                    ui.heading(title.as_str());
                    ui.add_space(8.0);
                    ui.label(RichText::new(label.as_str()).color(c.muted));
                    // The Clone dialog's address field has a "From GitHub…" button beside it.
                    let first = if matches!(kind, InputKind::Clone(_)) {
                        ui.horizontal(|ui| {
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                let pick = ui
                                    .button(format!("{}  From GitHub…", icon::GITHUB_LOGO))
                                    .on_hover_text("Choose one of your GitHub repositories");
                                pick.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Choose a GitHub repository"));
                                if pick.clicked() {
                                    open_picker = true;
                                }
                                widgets::text_field(ui, value, "")
                            })
                            .inner
                        })
                        .inner
                    } else {
                        widgets::text_field(ui, value, "")
                    };
                    let first_label = label.clone();
                    first.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, &first_label));
                    if ui.memory(|m| m.focused().is_none()) {
                        first.request_focus();
                    }
                    // A branch or tag name is checked as it is typed, so a refused name is
                    // explained before OK rather than after.
                    let names_a_ref = matches!(
                        kind,
                        InputKind::CreateBranch { .. }
                            | InputKind::CreateBranchFrom(_)
                            | InputKind::BranchFromStash { .. }
                            | InputKind::RenameBranch(_)
                            | InputKind::CreateTag { .. }
                    );
                    let name_problem = names_a_ref.then(|| crate::git_ext::ref_name_problem(value.trim())).flatten();
                    if let Some(problem) = name_problem {
                        ui.label(RichText::new(problem).small().color(c.warning));
                    }
                    if matches!(kind, InputKind::BranchFromStash { .. }) {
                        ui.add_space(4.0);
                        ui.label(
                            RichText::new(
                                "Creates the branch at the commit this stash was made on, switches to it, applies the stash, and removes the stash once it applies.",
                            )
                            .small()
                            .color(c.muted),
                        );
                    }
                    // A stash's options sit under its message, and are remembered in Settings.
                    if matches!(kind, InputKind::SaveStash) {
                        ui.add_space(6.0);
                        widgets::checkbox(ui, true, &mut self.settings.stash_include_untracked, "Include untracked files").on_hover_text(
                            "New files that Git does not track yet are saved in the stash and removed from the folder.",
                        );
                        widgets::checkbox(ui, true, &mut self.settings.stash_keep_index, "Keep staged changes").on_hover_text(
                            "Staged changes stay staged and in your files, and only unstaged changes leave your files. \
                             The stash still holds a copy of the staged changes, as Git's --keep-index does.",
                        );
                    }
                    let mut show_second = second_label.is_some();
                    if let InputKind::CreateTag { annotated, sign, push, remotes, default_remote, .. } = kind {
                        ui.add_space(6.0);
                        ui.horizontal(|ui| {
                            if widgets::radio(ui, !*annotated, "Lightweight").clicked() {
                                *annotated = false;
                            }
                            if widgets::radio(ui, *annotated, "Annotated, with a message").clicked() {
                                *annotated = true;
                            }
                        });
                        show_second = *annotated;
                        // Only an annotated tag can be signed. A lightweight tag shows the box unticked, and
                        // the choice is kept for when the tag is annotated again.
                        let mut signing = *annotated && *sign;
                        let signing_changed = widgets::checkbox(ui, *annotated, &mut signing, "Sign with your signing key")
                            .on_hover_text(
                                "Git signs the tag with your configured signing key (user.signingKey and gpg.format), as `git tag --sign` does.",
                            )
                            .changed();
                        if signing_changed {
                            *sign = signing;
                        }
                        if !remotes.is_empty() {
                            ui.add_space(6.0);
                            ui.horizontal(|ui| {
                                let mut ticked = push.is_some();
                                widgets::checkbox(ui, true, &mut ticked, "Push to")
                                    .widget_info(move || egui::WidgetInfo::selected(egui::WidgetType::Checkbox, true, ticked, "Push to remote"));
                                // Ticking the box chooses the default remote; the picker only changes it while ticked.
                                let mut chosen = push.clone().or_else(|| default_remote.clone()).unwrap_or_default();
                                if let [only] = remotes.as_slice() {
                                    let text = RichText::new(only.as_str());
                                    ui.label(if ticked { text } else { text.color(c.muted) });
                                    chosen = only.clone();
                                } else {
                                    ui.add_enabled_ui(ticked, |ui| {
                                        egui::ComboBox::from_id_salt("tag-push-remote").selected_text(chosen.as_str()).show_ui(ui, |ui| {
                                            for remote in remotes.iter() {
                                                ui.selectable_value(&mut chosen, remote.clone(), remote.as_str());
                                            }
                                        });
                                    });
                                }
                                *push = ticked.then_some(chosen);
                            });
                        }
                    }
                    let tag_needs_message = matches!(kind, InputKind::CreateTag { annotated: true, .. }) && second.trim().is_empty();
                    // While the destination is still automatic, the default folder and the address give it.
                    if let (InputKind::Clone(destination), Some(folder)) = (&mut *kind, self.settings.clone_folder.as_deref()) {
                        destination.follow(folder, value.as_str(), second);
                    }
                    let mut edited_destination = false;
                    let mut chose_destination = false;
                    if let Some(second_label) = second_label.as_ref().filter(|_| show_second) {
                        ui.add_space(6.0);
                        ui.label(RichText::new(second_label.as_str()).color(c.muted));
                        ui.horizontal(|ui| {
                            let browse = matches!(kind, InputKind::Clone(_));
                            let width = ui.available_width() - if browse { 90.0 } else { 0.0 };
                            if matches!(kind, InputKind::CreateTag { .. }) {
                                ui.add(egui::TextEdit::multiline(second).desired_rows(3).desired_width(width))
                                    .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, "Tag message"));
                            } else {
                                let second_name = second_label.clone();
                                let response = ui.add(egui::TextEdit::singleline(second).desired_width(width));
                                response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, &second_name));
                                edited_destination = response.changed();
                            }
                            if browse && ui.button("Choose…").clicked() {
                                if let Some(folder) = crate::file_dialog::pick_folder("Clone into") {
                                    *second = folder.join(repository_folder_name(value.as_str())).display().to_string();
                                    chose_destination = true;
                                }
                            }
                        });
                        if edited_destination || chose_destination {
                            if let InputKind::Clone(destination) = &mut *kind {
                                destination.by_hand = true;
                            }
                        }
                        if let InputKind::Clone(destination) = &*kind {
                            let path = std::path::PathBuf::from(second.trim());
                            if !destination.by_hand && !second.trim().is_empty() && is_taken(&path) {
                                ui.label(RichText::new(format!("{} already exists; choose another folder.", path.display())).color(c.muted));
                            }
                        }
                    }
                    ui.add_space(16.0);
                    // While the GitHub picker is open, Enter belongs to it, not to this dialog.
                    let enter = !picker_open && ui.input(|i| i.key_pressed(Key::Enter) && !i.modifiers.shift);
                    ui.horizontal(|ui| {
                        // A stash's message is optional: Git names one after the branch when it is empty.
                        let optional = matches!(kind, InputKind::SaveStash);
                        let ready = (optional || !value.trim().is_empty()) && !tag_needs_message && name_problem.is_none();
                        let ok_label = match kind {
                            InputKind::CreateTag { push: Some(_), .. } => "Create and push",
                            InputKind::BranchFromStash { .. } => "Create branch",
                            _ => "OK",
                        };
                        if widgets::primary_button(ui, ok_label, ready).clicked() || (enter && ready) {
                            run = true;
                        }
                        if ui.button("Cancel").clicked() {
                            close = true;
                        }
                    });
                }
                Dialog::Integrate { source } => {
                    let current = self.snapshot().map(|s| s.current_branch.clone()).unwrap_or_default();
                    ui.heading(format!("Integrate {}", source.display_name()));
                    ui.add_space(8.0);
                    ui.label(format!("Merge {} into {current}, or replay {current}'s commits on top of it?", source.display_name()));
                    ui.add_space(16.0);
                    ui.horizontal(|ui| {
                        if widgets::primary_button(ui, &format!("{}  Merge…", icon::GIT_MERGE), true).clicked() {
                            integrate = Some(false);
                        }
                        if ui.button(format!("{}  Rebase…", icon::GIT_PULL_REQUEST)).clicked() {
                            integrate = Some(true);
                        }
                        if ui.button("Cancel").clicked() {
                            close = true;
                        }
                    });
                }
                Dialog::ChooseParent { subject, parents, selected, revert, .. } => {
                    ui.heading(if *revert { "Revert a merge" } else { "Cherry-pick a merge" });
                    ui.add_space(8.0);
                    ui.label(format!("“{subject}” has {} parents. Choose the one its changes are measured against; usually the first, the branch it was merged into.", parents.len()));
                    ui.add_space(6.0);
                    for (index, (hash, parent_subject)) in parents.iter().enumerate() {
                        let subject = if parent_subject.is_empty() {
                            snapshot_commits.iter().find(|(h, _)| h == hash).map(|(_, s)| s.clone()).unwrap_or_default()
                        } else {
                            parent_subject.clone()
                        };
                        let label = format!("Parent {} · {} {subject}", index + 1, nicegit_core::models::short(hash));
                        if widgets::radio(ui, *selected == index, label).clicked() {
                            *selected = index;
                        }
                    }
                    ui.add_space(16.0);
                    ui.horizontal(|ui| {
                        if widgets::primary_button(ui, if *revert { "Revert" } else { "Cherry-pick" }, true).clicked() {
                            run = true;
                        }
                        if ui.button("Cancel").clicked() {
                            close = true;
                        }
                    });
                }
                Dialog::EditMessage { message, .. } => {
                    ui.heading("Edit commit message");
                    ui.add_space(8.0);
                    ui.label(RichText::new("Only the message changes; staged and unstaged edits are left out. The commit gets a new ID.").color(c.muted));
                    if published {
                        widgets::callout(
                            ui,
                            "This commit is already on a remote. Changing its message rewrites it, so others who pulled it will need to reconcile.",
                            true,
                        );
                    }
                    ui.add_space(6.0);
                    ui.add(egui::TextEdit::multiline(message).desired_rows(6).desired_width(f32::INFINITY));
                    ui.add_space(16.0);
                    ui.horizontal(|ui| {
                        if widgets::primary_button(ui, "Save message", !message.trim().is_empty()).clicked() {
                            run = true;
                        }
                        if ui.button("Cancel").clicked() {
                            close = true;
                        }
                    });
                }
                Dialog::Publish { remote } => {
                    ui.heading("Publish branch");
                    ui.add_space(8.0);
                    let branch = self.snapshot().map(|s| s.current_branch.clone()).unwrap_or_default();
                    ui.label(format!("Push {branch} to a remote and track it there."));
                    ui.add_space(6.0);
                    let remotes = self.snapshot().map(|s| s.remotes.clone()).unwrap_or_default();
                    egui::ComboBox::from_label("Remote").selected_text(remote.as_str()).show_ui(ui, |ui| {
                        for name in remotes {
                            ui.selectable_value(remote, name.clone(), name);
                        }
                    });
                    ui.add_space(16.0);
                    ui.horizontal(|ui| {
                        if widgets::primary_button(ui, "Publish", true).clicked() {
                            run = true;
                        }
                        if ui.button("Cancel").clicked() {
                            close = true;
                        }
                    });
                }
            }
        });
        if modal.should_close() && !picker_open {
            close = true;
        }
        if open_picker {
            self.clone_picker = Some(ClonePicker::new(&self.settings.github_enterprise_hosts));
        }
        if let Some(picker) = self.clone_picker.as_mut() {
            match picker.show(ctx) {
                Some(Picked::Url(url)) => {
                    if let Dialog::Input { value, .. } = &mut dialog {
                        *value = url;
                    }
                    self.clone_picker = None;
                }
                Some(Picked::Close) => self.clone_picker = None,
                None => {}
            }
        }
        if let (Some(rebase), Dialog::Integrate { source }) = (integrate, &dialog) {
            let source = source.clone();
            if rebase {
                self.request_rebase(source);
            } else {
                self.request_merge(source);
            }
            return;
        }
        if run {
            match dialog {
                Dialog::Confirm { pending, .. } => self.run_pending(pending),
                Dialog::Input { kind, value, second, .. } => self.run_input(kind, value, second),
                Dialog::Integrate { .. } => {}
                Dialog::ChooseParent { commit, selected, revert, branch, head, .. } => {
                    let operation = if revert { Operation::Revert } else { Operation::CherryPick };
                    let title = if revert { "Revert" } else { "Cherry-pick" };
                    self.act_recording_detail(title, title, Some(short(&commit).to_string()), move |client, path| {
                        let expected =
                            nicegit_core::rebase::Expectation { branch: Some(&branch), head: head.as_deref(), source_branch: None };
                        client.start(operation, &commit, Some(selected + 1), expected, path).map(|_| None)
                    });
                }
                Dialog::EditMessage { message, branch, head } => {
                    self.act_recording_detail("Edit message", "Edit message", crate::app::summary_of(&message), move |client, path| {
                        client.amend_message(&message, &branch, &head, path).map(|_| Some("Updated the commit message.".into()))
                    });
                }
                Dialog::Publish { remote } => {
                    if let Some(snapshot) = self.snapshot().cloned() {
                        self.act("Publish", move |client, path| {
                            client
                                .publish(&remote, &snapshot, path)
                                .map(|_| Some(format!("Published {} to {remote}.", snapshot.current_branch)))
                        });
                    }
                }
            }
        } else if !close {
            self.dialog = Some(dialog);
        }
    }

    pub fn run_pending(&mut self, pending: Pending) {
        match pending {
            Pending::Discard(entry) => {
                let name = entry.path.clone();
                self.act_with_record("Discard", move |client, path| {
                    let undo = client.discard_keeping_undo(&entry, path)?;
                    Ok((Some(format!("Discarded changes to {name}.")), undo.map(crate::app::Recorded::Discard)))
                })
            }
            Pending::DiscardLines { lines, review } => {
                let name = review.path.clone();
                let count = lines.len();
                self.act_with_record("Discard lines", move |client, path| {
                    let undo = client.discard_lines_keeping_undo(&lines, &review, path)?;
                    let message = format!("Discarded {count} changed line{} in {name}.", if count == 1 { "" } else { "s" });
                    Ok((Some(message), undo.map(crate::app::Recorded::Discard)))
                })
            }
            Pending::DiscardAll(entries) => self.act("Discard", move |client, path| {
                for entry in &entries {
                    client.discard(entry, path)?;
                }
                Ok(Some(format!("Discarded {} changed files.", entries.len())))
            }),
            Pending::DeleteBranch { branch, force } => {
                let name = branch.name.clone();
                self.act_with_record("Delete branch", move |client, path| {
                    let deletion = client.delete_branch_keeping_undo(&branch, force, path)?;
                    let step = nicegit_core::undo::UndoStep::BranchDeletions(vec![deletion]);
                    let entry = crate::app::HistoryEntry { title: "Delete branch".into(), detail: Some(name.clone()), step };
                    Ok((Some(format!("Deleted branch {name}.")), Some(crate::app::Recorded::Step(entry))))
                })
            }
            Pending::DeleteTag { name, tip } => self
                .act("Delete tag", move |client, path| client.delete_tag(&name, &tip, path).map(|_| Some(format!("Deleted tag {name}.")))),
            Pending::DropStash(stash) => {
                self.act("Delete stash", move |client, path| client.drop_stash(&stash, path).map(|_| Some("Deleted the stash.".into())))
            }
            Pending::Merge { source, branch, head, style } => {
                let name = source.display_name();
                match style {
                    // A squash leaves HEAD where it is, so there is no merge to undo.
                    MergeStyle::Squash => self.act_squash(name.clone(), move |client, path| {
                        client
                            .merge(&source, &branch, head.as_deref(), MergeStyle::Squash, path)
                            .map(|_| Some(format!("Staged {name}'s changes as a squash. Review and commit them.")))
                    }),
                    MergeStyle::Default | MergeStyle::NoFastForward => {
                        self.act_recording_detail("Merge", "Merge", Some(name.clone()), move |client, path| {
                            client.merge(&source, &branch, head.as_deref(), style, path).map(|_| Some(format!("Merged {name}.")))
                        })
                    }
                }
            }
            Pending::Rebase { onto, branch, head } => {
                let name = onto.display_name();
                self.act_recording_detail("Rebase", "Rebase", Some(format!("onto {name}")), move |client, path| {
                    crate::git_ext::rebase_onto(client, &onto, &branch, head.as_deref(), path)
                        .map(|_| Some(format!("Rebased onto {name}.")))
                })
            }
            Pending::CherryPick { commits, branch, head } => {
                let count = commits.len();
                self.act_recording_detail("Cherry-pick", "Cherry-pick", Some(commits_detail(&commits)), move |client, path| {
                    client.cherry_pick(&commits, head.as_deref(), &branch, path).map(|_| {
                        Some(if count == 1 { "Cherry-picked the commit.".to_string() } else { format!("Cherry-picked {count} commits.") })
                    })
                })
            }
            Pending::Revert { commit, branch, head } => {
                self.act_recording_detail("Revert", "Revert", Some(short(&commit).to_string()), move |client, path| {
                    crate::git_ext::revert(client, &commit, &branch, head.as_deref(), path).map(|_| Some("Reverted the commit.".into()))
                })
            }
            Pending::RevertMany { commits, branch, head } => {
                let count = commits.len();
                self.act_recording_detail("Revert", "Revert", Some(commits_detail(&commits)), move |client, path| {
                    client.revert_commits(&commits, head.as_deref(), &branch, path).map(|_| Some(format!("Reverted {count} commits.")))
                })
            }
            Pending::FixupInto { target, subject, branch, head } => {
                let Some(head) = head else { return };
                // Soft mode: Undo moves the branch back to the old commits and keeps the folded
                // changes staged, which is what they were before the fix up. A rebase that stops
                // for a conflict is not recorded, since the rebase is still in progress.
                self.act_recording_mode_detail(
                    "Fix up",
                    "Fix up",
                    Some(subject.clone()),
                    nicegit_core::undo::UndoMode::Soft,
                    move |client, path| {
                        client.fixup_into(&target, &branch, &head, path)?;
                        Ok(Some(format!("Folded the staged changes into “{subject}”.")))
                    },
                )
            }
            Pending::Abort(operation) => self.act("Abort", move |client, path| {
                client.abort_operation(operation, path).map(|_| Some(format!("Aborted the {}.", operation.name())))
            }),
            Pending::ResolveConflictsSide { paths, incoming, side_name } => self.act("Resolve conflicts", move |client, repo| {
                let report = client.resolve_conflicts_side(&paths, incoming, repo)?;
                let text = resolve_conflicts_notice(&report, &side_name);
                // Nothing resolved is an error. The app shows the message itself, without a prefix.
                if report.resolved.is_empty() && !report.failed.is_empty() {
                    Err(GitError::failed("resolve conflicts", text))
                } else {
                    Ok(Some(text))
                }
            }),
            Pending::PushBranch { branch, remote, addresses } => {
                let name = branch.name.clone();
                self.act("Push branch", move |client, path| {
                    client.push_branch(&branch, &remote, &addresses, path).map(|_| Some(format!("Pushed {name} to {remote}.")))
                })
            }
            Pending::ForcePush { branch, head, upstream, expected_remote_tip, addresses } => {
                self.act("Force push", move |client, path| {
                    // The push reads only these fields, so it runs against what was shown.
                    let snapshot = nicegit_core::Snapshot {
                        current_branch: branch.clone(),
                        head_hash: Some(head),
                        upstream: Some(upstream.clone()),
                        remote_push_addresses: addresses,
                        ..Default::default()
                    };
                    client.force_push(&snapshot, &expected_remote_tip, path).map(|_| Some(format!("Force pushed {branch} to {upstream}.")))
                })
            }
            Pending::ApplyPatch { file, branch, head } => self.act("Apply patch", move |client, path| {
                client.require_checkout(&branch, head.as_deref(), "apply", path)?;
                let contents = std::fs::read(&file).map_err(|e| nicegit_core::GitError::failed("apply", e.to_string()))?;
                client.apply_patch(&contents, path).map(|_| Some("Applied the patch. Its changes are unstaged.".into()))
            }),
            Pending::RestoreFile { path, source, branch, head } => self.act("Restore file", move |client, repo| {
                client
                    .restore(&path, &source, &branch, head.as_deref(), repo)
                    .map(|_| Some(format!("Restored {path}. Review and commit the staged change.")))
            }),
            Pending::RestoreFromStash { path, stash, untracked, branch, head } => {
                let reference = stash.reference.clone();
                self.act("Restore file", move |client, repo| {
                    client
                        .restore_from_stash(&path, &stash, untracked, &branch, head.as_deref(), repo)
                        .map(|_| Some(format!("Restored {path} from {reference}.")))
                })
            }
            Pending::DeleteRemoteTag { tag, tip, remote, addresses } => self.act("Delete remote tag", move |client, path| {
                let expected = addresses.get(&remote).map(Vec::as_slice).unwrap_or_default();
                client.delete_remote_tag(&tag, &remote, &tip, expected, path).map(|_| Some(format!("Deleted {tag} from {remote}.")))
            }),
            Pending::DeleteRemoteBranch { remote, branch, tip, addresses } => self.act("Delete remote branch", move |client, path| {
                let expected = addresses.get(&remote).map(Vec::as_slice).unwrap_or_default();
                client
                    .delete_remote_branch(&remote, &branch, &tip, expected, path)
                    .map(|_| Some(format!("Deleted {branch} from {remote}.")))
            }),
            Pending::PopStash(stash) => self.act("Pop stash", move |client, path| {
                client.pop_stash(&stash, path).map(|_| Some("Applied and deleted the stash.".into()))
            }),
            Pending::Undo => self.undo(),
            Pending::Redo => self.redo(),
            Pending::UndoCommit { branch, head } => {
                self.act_recording_detail("Undo commit", "Undo commit", Some(short(&head).to_string()), move |client, path| {
                    client.undo_last_commit(&branch, &head, path).map(|_| Some("Undid the last commit. Its changes are staged.".into()))
                })
            }
        }
    }

    pub fn run_input(&mut self, kind: InputKind, value: String, second: String) {
        match kind {
            InputKind::CreateBranch { branch, head } => self.act("Create branch", move |client, path| {
                client
                    .create_branch(&value, &branch, head.as_deref(), path)
                    .map(|_| Some(format!("Created and switched to {}.", value.trim())))
            }),
            InputKind::CreateBranchFrom(source) => self.act("Create branch", move |client, path| {
                client.create_branch_from(&value, &source, path).map(|_| Some(format!("Created branch {}.", value.trim())))
            }),
            InputKind::BranchFromStash { stash, branch, head } => self.act("Create branch from stash", move |client, path| {
                client
                    .branch_from_stash(&value, &stash, &branch, head.as_deref(), path)
                    .map(|_| Some(format!("Created {} from the stash and switched to it.", value.trim())))
            }),
            InputKind::RenameBranch(branch) => {
                self.act("Rename branch", move |client, path| client.rename_branch(&branch, &value, path).map(|_| None))
            }
            InputKind::CreateTag { target, annotated, sign, push, push_addresses, .. } => self.act("Create tag", move |client, path| {
                let name = value.trim().to_string();
                let message = annotated.then_some(second.as_str());
                let signed = annotated && sign;
                client.create_tag(&value, &target, message, signed, path)?;
                let created = if signed { "signed tag" } else { "tag" };
                let Some(remote) = push else {
                    return Ok(Some(format!("Created {created} {name}.")));
                };
                // The tag exists now, so a failed push leaves it in place and says so.
                let addresses = push_addresses.get(&remote).map(Vec::as_slice).unwrap_or_default();
                let pushed = client.tag_tip(&name, path).and_then(|tip| client.push_tag(&name, &remote, &tip, addresses, path));
                match pushed {
                    Ok(()) => Ok(Some(format!("Created {created} {name} and pushed it to {remote}."))),
                    Err(error) => {
                        let reason = match error {
                            GitError::CommandFailed { message, .. } => message,
                            other => other.to_string(),
                        };
                        Err(GitError::failed(
                            "create tag",
                            format!("Created {created} {name}, but could not push it to {remote}: {reason}"),
                        ))
                    }
                }
            }),
            InputKind::SaveStash => {
                let options =
                    StashOptions { keep_index: self.settings.stash_keep_index, include_untracked: self.settings.stash_include_untracked };
                self.act("Stash", move |client, path| {
                    client.save_stash(&value, options, path).map(|_| Some("Saved your changes in a stash.".into()))
                })
            }
            InputKind::Clone(_) => {
                let destination = std::path::PathBuf::from(second.trim());
                let source = value.trim().to_string();
                if destination.as_os_str().is_empty() {
                    self.notify("Choose a destination folder.", true);
                    return;
                }
                if self.busy.is_some() {
                    return;
                }
                self.busy = Some("Clone".into());
                let context = self.worker.context.clone();
                let task = crate::tools::Task::spawn(&context, move || {
                    nicegit_core::GitClient::new().clone_repository(&source, &destination).map(|_| destination)
                });
                self.pending_clone = Some(task);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{default_push_remote, repository_folder_name, CloneDestination};

    #[test]
    fn a_new_tag_is_pushed_to_the_upstream_remote_then_origin_then_the_first() {
        let remotes = |names: &[&str]| names.iter().map(|name| name.to_string()).collect::<Vec<_>>();
        let both = remotes(&["fork", "origin"]);
        assert_eq!(default_push_remote(&both, Some("fork/main")).as_deref(), Some("fork"));
        assert_eq!(default_push_remote(&both, None).as_deref(), Some("origin"));
        assert_eq!(default_push_remote(&remotes(&["fork", "upstream"]), None).as_deref(), Some("fork"));
        // Remote names can contain slashes; the longest one that prefixes the upstream wins.
        let nested = remotes(&["team", "team/fork"]);
        assert_eq!(default_push_remote(&nested, Some("team/fork/main")).as_deref(), Some("team/fork"));
        assert_eq!(default_push_remote(&remotes(&["origin"]), Some("originals/main")).as_deref(), Some("origin"));
        assert_eq!(default_push_remote(&[], None), None);
    }

    #[test]
    fn repository_folder_name_is_the_last_path_segment_without_git() {
        assert_eq!(repository_folder_name("https://github.com/acme/app.git"), "app");
        assert_eq!(repository_folder_name("https://host/acme/.."), "repository");
        assert_eq!(repository_folder_name("."), "repository");
        assert_eq!(repository_folder_name("git@github.com:acme/app.git"), "app");
        assert_eq!(repository_folder_name("https://host/path/app/"), "app");
        assert_eq!(repository_folder_name("ssh://git@host:22/acme/app"), "app");
        assert_eq!(repository_folder_name("/local/path/repo"), "repo");
        assert_eq!(repository_folder_name("  https://github.com/acme/app  "), "app");
        // A Windows path uses backslashes, and a drive's colon is not a separator.
        assert_eq!(repository_folder_name(r"C:\repos\x"), "x");
        assert_eq!(repository_folder_name(""), "repository");
        assert_eq!(repository_folder_name(".git"), "repository");
    }

    #[test]
    fn clone_folder_destination_follows_the_address_until_the_person_edits_it() {
        let folder = std::path::Path::new("/work/clones");
        // Joined as the app joins them, so the separator is right on every platform.
        let inside = |name: &str| folder.join(name).display().to_string();
        let mut state = CloneDestination::default();
        let mut destination = String::new();
        state.follow(folder, "https://github.com/acme/app.git", &mut destination);
        assert_eq!(destination, inside("app"));
        state.follow(folder, "https://github.com/acme/tool", &mut destination);
        assert_eq!(destination, inside("tool"), "still automatic, so it follows the address");
        state.follow(folder, "", &mut destination);
        assert_eq!(destination, "", "an empty address leaves no automatic destination");

        state.follow(folder, "https://github.com/acme/app.git", &mut destination);
        destination.push_str("-copy");
        state.by_hand = true;
        state.follow(folder, "https://github.com/acme/other.git", &mut destination);
        assert_eq!(destination, format!("{}-copy", inside("app")), "an edited destination is not overwritten");
    }
}
