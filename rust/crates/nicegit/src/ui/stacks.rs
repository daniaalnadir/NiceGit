//! Stacked branches in the interface: the notice above the graph for branches still based on an
//! earlier tip of the checked-out branch, the restack and push confirmations, and the background
//! read that finds the stacks. The read runs when the branch tips change, never per frame.

use std::collections::BTreeMap;
use std::path::Path;

use egui::{RichText, Ui};
use nicegit_core::stack::{self, OutdatedChild, Stack, StackPush};
use nicegit_core::undo::UndoStep;
use nicegit_core::{GitClient, Snapshot};

use crate::app::{HistoryEntry, NiceGitApp, Recorded};
use crate::i18n::{tr, tr_count, tr_fmt};
use crate::theme;
use crate::tools::Task;
use crate::ui::dialogs::Pending;

/// The branches still based on an earlier tip of `parent`, read while `parent` was at `tip`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Outdated {
    pub parent: String,
    pub tip: String,
    pub children: Vec<OutdatedChild>,
}

/// What the background read finds: the stacks, and the branches outdated by the checked-out branch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StackRead {
    pub stacks: Vec<Stack>,
    pub outdated: Option<Outdated>,
}

/// The branch tips and checkout a read depends on. A refresh that changes none of them is not read again.
pub type StackKey = (Vec<(String, String)>, String, Option<String>);

/// The key for `snapshot`: each local branch's tip, and the checked-out branch and its HEAD.
pub fn stack_key(snapshot: &Snapshot) -> StackKey {
    let tips =
        snapshot.branches.iter().filter(|branch| !branch.is_remote).map(|branch| (branch.name.clone(), branch.tip.clone())).collect();
    (tips, snapshot.current_branch.clone(), snapshot.head_hash.clone())
}

/// Reads the stacks, and the branches outdated by the checked-out branch, from the repository.
fn read_stacks(path: &Path, snapshot: &Snapshot) -> StackRead {
    let client = GitClient::new();
    let stacks = stack::find_stacks(&client, path).unwrap_or_default();
    let outdated = match (snapshot.is_on_branch(), snapshot.head_hash.as_ref()) {
        (true, Some(tip)) => {
            let children = stack::outdated_children(&client, &snapshot.current_branch, path).unwrap_or_default();
            (!children.is_empty()).then(|| Outdated { parent: snapshot.current_branch.clone(), tip: tip.clone(), children })
        }
        _ => None,
    };
    StackRead { stacks, outdated }
}

impl NiceGitApp {
    /// Starts the stack read for repository `index` when its branch tips have changed since the last one.
    pub(crate) fn start_stack_read(&mut self, index: usize) {
        let Some(snapshot) = self.repos[index].snapshot.clone() else { return };
        let key = stack_key(&snapshot);
        if self.repos[index].stack_key.as_ref() == Some(&key) {
            return;
        }
        self.repos[index].stack_key = Some(key);
        let path = self.repos[index].path.clone();
        let context = self.worker.context.clone();
        self.repos[index].stack_read = Some(Task::spawn(&context, move || read_stacks(&path, &snapshot)));
    }

    /// Keeps the stacks and outdated branches once their background read arrives.
    pub(crate) fn receive_stack_reads(&mut self) {
        for repo in &mut self.repos {
            let Some(task) = repo.stack_read.as_mut() else { continue };
            if let Some(read) = task.get().cloned() {
                repo.stacks = read.stacks;
                repo.outdated = read.outdated;
                repo.stack_read = None;
            }
        }
    }

    /// The stack that holds the local branch `name`, if it is in one.
    pub(crate) fn stack_of(&self, name: &str) -> Option<Stack> {
        self.repo()?.stacks.iter().find(|stack| stack.branches.iter().any(|branch| branch.name == name)).cloned()
    }

    /// The branch `name` is stacked on, for showing beside it.
    pub(crate) fn stack_parent_of(&self, name: &str) -> Option<String> {
        self.repo()?.stacks.iter().flat_map(|stack| stack.branches.iter()).find(|branch| branch.name == name)?.parent.clone()
    }

    /// The branches still based on an earlier tip of `name`, when they are known.
    pub(crate) fn outdated_for(&self, name: &str) -> Option<Outdated> {
        self.repo()?.outdated.clone().filter(|outdated| outdated.parent == name)
    }

    /// The notice above the graph when branches are still based on an earlier tip of the checked-out
    /// branch. Dismissing it hides it until that branch's tip changes again.
    pub(crate) fn outdated_notice(&mut self, ui: &mut Ui, idle: bool) {
        let Some(outdated) = self.repo().and_then(|repo| repo.outdated.clone()) else { return };
        let dismissed = (outdated.parent.clone(), outdated.tip.clone());
        if self.repo().and_then(|repo| repo.outdated_dismissed.clone()) == Some(dismissed.clone()) {
            return;
        }
        // Restacking waits for any Git operation in progress, as the other actions do.
        let idle = idle && self.snapshot().is_some_and(|snapshot| snapshot.operation.is_none());
        let c = theme::of(ui);
        let names: Vec<&str> = outdated.children.iter().map(|child| child.name.as_str()).collect();
        let sentence = tr_count(
            "{count} branch is still based on the old {parent}: {names}.",
            "{count} branches are still based on the old {parent}: {names}.",
            outdated.children.len(),
        )
        .replace("{parent}", &outdated.parent)
        .replace("{names}", &names.join(", "));
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(sentence).small().color(c.muted));
            if ui.add_enabled(idle, egui::Button::new(tr("Restack"))).clicked() {
                self.request_restack(outdated.clone());
            }
            if ui.button(tr("Dismiss")).clicked() {
                if let Some(repo) = self.repo_mut() {
                    repo.outdated_dismissed = Some(dismissed);
                }
            }
        });
    }

    /// Asks to restack the branches still based on an earlier tip of their parent. The confirmation
    /// lists each branch with its commit count, and the checkout is checked again when it runs.
    pub(crate) fn request_restack(&mut self, outdated: Outdated) {
        let lines: Vec<String> = outdated
            .children
            .iter()
            .map(|child| tr_count("{name}: {count} commit", "{name}: {count} commits", child.commits).replace("{name}", &child.name))
            .collect();
        let intro = tr_fmt!(
            "Each branch's own commits are replayed onto the new tip of {parent}. Branches built on them move too. The working tree changes only if the checked-out branch moves.",
            parent = outdated.parent
        );
        let message = format!("{intro}\n\n{}", lines.join("\n"));
        let title = tr_fmt!("Restack branches on {parent}?", parent = outdated.parent);
        self.confirm(title, message, tr("Restack"), Pending::Restack { outdated });
    }

    /// Restacks the confirmed branches, as one undoable action. The checkout must still be the one
    /// the confirmation showed.
    pub(crate) fn run_restack(&mut self, outdated: Outdated) {
        let detail = outdated.parent.clone();
        self.act_with_record("Restack", move |client, path| {
            client.require_checkout(&outdated.parent, Some(&outdated.tip), "restack", path)?;
            let moved = stack::restack(client, &outdated.parent, &outdated.tip, &outdated.children, path)?;
            if moved.is_empty() {
                return Ok((Some(tr("Nothing needed restacking.").to_string()), None));
            }
            let count = moved.len();
            let message = tr_count("Restacked 1 branch.", "Restacked {count} branches.", count);
            let step = UndoStep::BranchMoves(moved);
            Ok((Some(message), Some(Recorded::Step(HistoryEntry { title: "Restack".into(), detail: Some(detail), step }))))
        });
    }

    /// Asks to push the branches of a stack. The plan, with each branch's remote branch and whether
    /// it is replaced under a lease, is read now and shown in the confirmation.
    pub(crate) fn request_push_stack(&mut self, names: Vec<String>) {
        let Some((path, snapshot)) = self.repo().map(|repo| (repo.path.clone(), repo.snapshot.clone())) else { return };
        let Some(snapshot) = snapshot else { return };
        let pushes = match stack::plan_stack_push(&GitClient::new(), &names, &path) {
            Ok(pushes) => pushes,
            Err(error) => {
                self.notify(error.to_string(), true);
                return;
            }
        };
        if pushes.is_empty() {
            self.notify(tr("None of these branches has a remote branch to push to. Set an upstream first."), true);
            return;
        }
        let lines: Vec<String> = pushes
            .iter()
            .map(|push| {
                let name = push.destination.strip_prefix("refs/heads/").unwrap_or(&push.destination);
                if push.forced {
                    tr_fmt!(
                        "{branch} to {remote}/{name}, replacing the remote branch",
                        branch = push.branch,
                        remote = push.remote,
                        name = name
                    )
                } else {
                    tr_fmt!("{branch} to {remote}/{name}", branch = push.branch, remote = push.remote, name = name)
                }
            })
            .collect();
        let intro = tr("Each branch is pushed to its own remote branch in one atomic push. Tags are not pushed.");
        let title = tr_count("Push this branch?", "Push {count} branches?", pushes.len());
        let message = format!("{intro}\n\n{}", lines.join("\n"));
        self.confirm(title, message, tr("Push"), Pending::PushStack { pushes, addresses: snapshot.remote_push_addresses.clone() });
    }

    /// Pushes the confirmed stack in one atomic push per remote.
    pub(crate) fn run_push_stack(&mut self, pushes: Vec<StackPush>, addresses: BTreeMap<String, Vec<String>>) {
        let count = pushes.len();
        self.act("Push stack", move |client, path| {
            stack::push_stack(client, &pushes, &addresses, path)?;
            Ok(Some(tr_count("Pushed 1 branch.", "Pushed {count} branches.", count)))
        });
    }
}
