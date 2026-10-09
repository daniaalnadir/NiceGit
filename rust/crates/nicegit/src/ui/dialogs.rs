//! Confirmations and short forms. Each captures what the user saw, so the action it runs can
//! refuse if the repository changed in the meantime.

use egui::{Key, RichText};
use egui_phosphor::regular as icon;
use nicegit_core::{Branch, Operation, Stash, StatusEntry};

use crate::app::NiceGitApp;
use crate::theme;
use crate::tools::widgets;

pub enum Pending {
    Discard(StatusEntry),
    DiscardAll(Vec<StatusEntry>),
    DeleteBranch { branch: Branch, force: bool },
    DeleteTag { name: String, tip: String },
    DropStash(Stash),
    Merge { source: Branch, branch: String, head: Option<String> },
    Rebase { onto: Branch, branch: String, head: Option<String> },
    CherryPick { commits: Vec<String>, branch: String, head: Option<String> },
    Revert { commit: String, branch: String, head: Option<String> },
    Abort(Operation),
    UndoCommit { branch: String, head: String },
    PushBranch { branch: Branch, remote: String, addresses: std::collections::BTreeMap<String, Vec<String>> },
    ApplyPatch { file: std::path::PathBuf, branch: String, head: Option<String> },
    Undo,
    Redo,
    RestoreFile { path: String, source: String, branch: String, head: Option<String> },
}

pub enum InputKind {
    CreateBranch { branch: String, head: Option<String> },
    CreateBranchFrom(Branch),
    RenameBranch(Branch),
    CreateTag { target: String },
    SaveStash,
    Clone,
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
        Self::with_second("Clone repository", "Repository URL or local path", "Destination folder", InputKind::Clone)
    }
}

impl Pending {
    fn destructive(&self) -> bool {
        matches!(
            self,
            Pending::Discard(_)
                | Pending::DiscardAll(_)
                | Pending::DeleteBranch { .. }
                | Pending::DeleteTag { .. }
                | Pending::DropStash(_)
                | Pending::Abort(_)
        )
    }
}

impl NiceGitApp {
    pub fn dialogs(&mut self, ctx: &egui::Context) {
        let Some(mut dialog) = self.dialog.take() else { return };
        let mut close = false;
        let mut run = false;
        let mut integrate: Option<bool> = None;
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
                    let first = widgets::text_field(ui, value, "");
                    let first_label = label.clone();
                    first.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, &first_label));
                    if ui.memory(|m| m.focused().is_none()) {
                        first.request_focus();
                    }
                    if let Some(second_label) = second_label {
                        ui.add_space(6.0);
                        ui.label(RichText::new(second_label.as_str()).color(c.muted));
                        ui.horizontal(|ui| {
                            let browse = matches!(kind, InputKind::Clone);
                            let width = ui.available_width() - if browse { 90.0 } else { 0.0 };
                            if matches!(kind, InputKind::CreateTag { .. }) {
                                ui.add(egui::TextEdit::multiline(second).desired_rows(3).desired_width(width));
                            } else {
                                let second_name = second_label.clone();
                                ui.add(egui::TextEdit::singleline(second).desired_width(width))
                                    .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, &second_name));
                            }
                            if browse && ui.button("Choose…").clicked() {
                                if let Some(folder) = rfd::FileDialog::new().set_title("Clone into").pick_folder() {
                                    let name = value
                                        .trim_end_matches('/')
                                        .rsplit(['/', ':'])
                                        .next()
                                        .unwrap_or("repository")
                                        .trim_end_matches(".git")
                                        .to_string();
                                    *second = folder.join(if name.is_empty() { "repository".into() } else { name }).display().to_string();
                                }
                            }
                        });
                    }
                    ui.add_space(16.0);
                    let enter = ui.input(|i| i.key_pressed(Key::Enter) && !i.modifiers.shift);
                    ui.horizontal(|ui| {
                        if widgets::primary_button(ui, "OK", !value.trim().is_empty()).clicked() || (enter && !value.trim().is_empty()) {
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
                        ui.radio_value(selected, index, format!("Parent {} · {} {subject}", index + 1, nicegit_core::models::short(hash)));
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
        if modal.should_close() {
            close = true;
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
                    self.act_recording(title, title, move |client, path| {
                        let expected =
                            nicegit_core::rebase::Expectation { branch: Some(&branch), head: head.as_deref(), source_branch: None };
                        client.start(operation, &commit, Some(selected + 1), expected, path).map(|_| None)
                    });
                }
                Dialog::EditMessage { message, branch, head } => {
                    self.act_recording("Edit message", "Edit message", move |client, path| {
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
                    let entry = crate::app::HistoryEntry { title: format!("Delete {name}"), step };
                    Ok((Some(format!("Deleted branch {name}.")), Some(crate::app::Recorded::Step(entry))))
                })
            }
            Pending::DeleteTag { name, tip } => self
                .act("Delete tag", move |client, path| client.delete_tag(&name, &tip, path).map(|_| Some(format!("Deleted tag {name}.")))),
            Pending::DropStash(stash) => {
                self.act("Delete stash", move |client, path| client.drop_stash(&stash, path).map(|_| Some("Deleted the stash.".into())))
            }
            Pending::Merge { source, branch, head } => {
                let name = source.display_name();
                self.act_recording("Merge", "Merge", move |client, path| {
                    client.merge(&source, &branch, head.as_deref(), path).map(|_| Some(format!("Merged {name}.")))
                })
            }
            Pending::Rebase { onto, branch, head } => {
                let name = onto.display_name();
                self.act_recording("Rebase", "Rebase", move |client, path| {
                    crate::git_ext::rebase_onto(client, &onto, &branch, head.as_deref(), path)
                        .map(|_| Some(format!("Rebased onto {name}.")))
                })
            }
            Pending::CherryPick { commits, branch, head } => {
                let count = commits.len();
                self.act_recording("Cherry-pick", "Cherry-pick", move |client, path| {
                    client.cherry_pick(&commits, head.as_deref(), &branch, path).map(|_| {
                        Some(if count == 1 { "Cherry-picked the commit.".to_string() } else { format!("Cherry-picked {count} commits.") })
                    })
                })
            }
            Pending::Revert { commit, branch, head } => self.act_recording("Revert", "Revert", move |client, path| {
                crate::git_ext::revert(client, &commit, &branch, head.as_deref(), path).map(|_| Some("Reverted the commit.".into()))
            }),
            Pending::Abort(operation) => self.act("Abort", move |client, path| {
                client.abort_operation(operation, path).map(|_| Some(format!("Aborted the {}.", operation.name())))
            }),
            Pending::PushBranch { branch, remote, addresses } => {
                let name = branch.name.clone();
                self.act("Push branch", move |client, path| {
                    client.push_branch(&branch, &remote, &addresses, path).map(|_| Some(format!("Pushed {name} to {remote}.")))
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
            Pending::Undo => self.undo(),
            Pending::Redo => self.redo(),
            Pending::UndoCommit { branch, head } => self.act_recording("Undo commit", "Undo commit", move |client, path| {
                client.undo_last_commit(&branch, &head, path).map(|_| Some("Undid the last commit. Its changes are staged.".into()))
            }),
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
            InputKind::RenameBranch(branch) => {
                self.act("Rename branch", move |client, path| client.rename_branch(&branch, &value, path).map(|_| None))
            }
            InputKind::CreateTag { target } => self.act("Create tag", move |client, path| {
                let message = (!second.trim().is_empty()).then_some(second.as_str());
                client.create_tag(&value, &target, message, path).map(|_| Some(format!("Created tag {}.", value.trim())))
            }),
            InputKind::SaveStash => self
                .act("Stash", move |client, path| client.save_stash(&value, path).map(|_| Some("Saved your changes in a stash.".into()))),
            InputKind::Clone => {
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
