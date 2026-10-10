//! Interactive rebase, plus rebase, revert, and cherry-pick started from a chosen commit or
//! branch. Port of `GitInteractiveRebase.swift` and the related parts of `GitClient.swift`.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::client::GitClient;
use crate::models::{Branch, Commit, GitError, Operation, Result, StatusKind};
use crate::parsers::parse_log;
use crate::runner::{self, RunOptions};

const REBASE_COMMAND: &str = "interactive rebase";

/// Commit fields in the order `parse_log` reads them. The decoration field is left empty.
const PLAN_FORMAT: &str = "%H%x1f%h%x1f%P%x1f%x1f%s%x1f%an%x1f%ae%x1f%cr%x1f%ct%x1e";

/// What one commit becomes in an interactive rebase.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RebaseAction {
    Pick,
    /// Keep the commit with a new message.
    Reword(String),
    /// Fold into the kept commit applied before it, adding this commit's message to that one.
    Squash,
    /// Fold into the kept commit applied before it, discarding this commit's message.
    Fixup,
    Drop,
}

/// One commit and what to do with it. Steps are given oldest first, as Git applies them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RebaseStep {
    pub commit: Commit,
    pub action: RebaseAction,
}

/// The commits an interactive rebase would rewrite.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RebasePlan {
    /// Oldest first, as Git applies them.
    pub commits: Vec<Commit>,
    /// The commit the rewritten commits are replayed onto, or `None` to rewrite from the root.
    pub base: Option<String>,
    /// Commits already on a remote-tracking branch. Rewriting them diverges from the remote.
    pub published_commits: BTreeSet<String>,
    /// Full messages by commit, used to combine squashed messages.
    pub messages: BTreeMap<String, String>,
}

/// What an operation was chosen for. Each field that is set must still match the checkout,
/// so a stale dialog cannot act on a different state.
#[derive(Clone, Copy, Debug, Default)]
pub struct Expectation<'a> {
    pub branch: Option<&'a str>,
    pub head: Option<&'a str>,
    /// The source branch whose tip was shown, for a rebase or merge onto a branch.
    pub source_branch: Option<&'a Branch>,
}

struct Group {
    message: String,
    /// Whether the message differs from what Git would keep, so it must be stored and applied.
    changed: bool,
}

impl GitClient {
    /// Lists the commits after `base` through HEAD, oldest first, for editing. `None` lists the
    /// whole history, for rewriting from the root commit.
    pub fn interactive_rebase_plan(&self, base: Option<&str>, directory: &Path) -> Result<RebasePlan> {
        let head = self.run_trimmed(&["rev-parse", "--verify", "HEAD"], directory)?;
        let base = base.map(|revision| self.resolve_commit(revision, directory)).transpose()?;
        if let Some(base) = &base {
            self.run(&["merge-base", "--is-ancestor", base, &head], directory)
                .map_err(|_| GitError::failed(REBASE_COMMAND, "This commit is not part of the current branch's history."))?;
        }
        // The range is built from resolved object IDs, so it cannot be read as an option.
        let range = match &base {
            Some(base) => format!("{base}..{head}"),
            None => head.clone(),
        };
        if !self.run(&["rev-list", "--merges", "--end-of-options", &range, "--"], directory)?.trim().is_empty() {
            return Err(GitError::failed(
                REBASE_COMMAND,
                "These commits include a merge. Interactive rebase here keeps history linear, so choose a commit after the latest merge.",
            ));
        }
        let format = format!("--pretty=format:{PLAN_FORMAT}");
        let commits = parse_log(&self.run(&["log", "--reverse", "--no-color", &format, "--end-of-options", &range, "--"], directory)?);
        if commits.is_empty() {
            return Err(GitError::failed(REBASE_COMMAND, "There are no commits after this one to rewrite."));
        }
        let unpublished: BTreeSet<String> =
            self.run(&["rev-list", &range, "--not", "--remotes", "--"], directory)?.lines().map(str::to_string).collect();
        let published_commits = commits.iter().map(|commit| commit.hash.clone()).filter(|hash| !unpublished.contains(hash)).collect();
        let mut messages = BTreeMap::new();
        for commit in &commits {
            let message = self.run_trimmed(&["show", "--no-patch", "--format=%B", "--no-color", &commit.hash, "--"], directory)?;
            messages.insert(commit.hash.clone(), message);
        }
        Ok(RebasePlan { commits, base, published_commits, messages })
    }

    /// Rewrites the current branch from `plan` using `steps`, oldest first. The steps must cover
    /// exactly the planned commits, and may be reordered. Messages for rewords and squashes are
    /// stored as Git objects, applied by `exec` lines, so a rebase that stops for a conflict can
    /// still finish with them.
    pub fn interactive_rebase(
        &self,
        steps: &[RebaseStep],
        plan: &RebasePlan,
        expected_branch: &str,
        expected_head: &str,
        directory: &Path,
    ) -> Result<()> {
        let state = self.require_checkout(expected_branch, Some(expected_head), REBASE_COMMAND, directory)?;
        if state.operation.is_some() {
            return Err(GitError::failed(REBASE_COMMAND, "Finish or abort the current Git operation first."));
        }
        if self.load_status(directory)?.iter().any(|entry| entry.kind != StatusKind::Untracked) {
            return Err(GitError::failed(REBASE_COMMAND, "Commit or stash your changes before rewriting commits."));
        }
        let mut planned: Vec<&str> = plan.commits.iter().map(|commit| commit.hash.as_str()).collect();
        let mut requested: Vec<&str> = steps.iter().map(|step| step.commit.hash.as_str()).collect();
        planned.sort_unstable();
        requested.sort_unstable();
        if planned != requested || plan.commits.last().map(|commit| commit.hash.as_str()) != Some(expected_head) {
            return Err(GitError::failed(REBASE_COMMAND, "The commits to rewrite changed. Refresh and start again."));
        }
        if let Some(first) = steps.iter().find(|step| step.action != RebaseAction::Drop) {
            if matches!(first.action, RebaseAction::Squash | RebaseAction::Fixup) {
                return Err(GitError::failed(
                    REBASE_COMMAND,
                    "The oldest kept commit has nothing to squash into. Pick or reword it instead.",
                ));
            }
        }

        let scratch = Scratch::new(REBASE_COMMAND)?;
        let mut todo: Vec<String> = Vec::new();
        let mut group: Option<Group> = None;
        for step in steps {
            let hash = step.commit.hash.as_str();
            let original = plan.messages.get(hash).cloned().unwrap_or_else(|| step.commit.subject.clone());
            match &step.action {
                RebaseAction::Pick | RebaseAction::Reword(_) => {
                    let kept = match &step.action {
                        RebaseAction::Reword(message) => {
                            if message.trim().is_empty() {
                                return Err(GitError::EmptyCommitMessage);
                            }
                            Group { message: message.clone(), changed: true }
                        }
                        _ => Group { message: original, changed: false },
                    };
                    self.finish_group(&mut todo, group.replace(kept), &scratch, directory)?;
                    todo.push(format!("pick {hash}"));
                }
                RebaseAction::Squash => {
                    todo.push(format!("fixup {hash}"));
                    let current = group.get_or_insert_with(|| Group { message: String::new(), changed: false });
                    current.message = format!("{}\n\n{original}", current.message);
                    current.changed = true;
                }
                RebaseAction::Fixup => todo.push(format!("fixup {hash}")),
                RebaseAction::Drop => todo.push(format!("drop {hash}")),
            }
        }
        self.finish_group(&mut todo, group.take(), &scratch, directory)?;

        let todo_file = scratch.write(REBASE_COMMAND, "todo", format!("{}\n", todo.join("\n")).as_bytes())?;
        // Git runs the sequence editor through the shell with the todo path appended.
        let editor = format!("cp {}", shell_quote(&sequence_editor_path(&todo_file)));
        let mut arguments = vec![
            "-c",
            "rebase.updateRefs=false",
            "-c",
            "rebase.autoStash=false",
            "-c",
            "rebase.autoSquash=false",
            "-c",
            "rebase.missingCommitsCheck=ignore",
            "-c",
            "rebase.abbreviateCommands=false",
            "rebase",
            "--interactive",
            "--empty=drop",
            "--no-autosquash",
            "--no-update-refs",
        ];
        match &plan.base {
            Some(base) => arguments.extend(["--end-of-options", base.as_str()]),
            None => arguments.push("--root"),
        }
        let options = RunOptions { env: &[("GIT_SEQUENCE_EDITOR", editor.as_str())], ..self.options() };
        if let Err(error) = runner::run(&arguments, directory, &options) {
            if matches!(self.current_operation(directory), Ok(Some(Operation::Rebase))) {
                return Err(GitError::failed(
                    REBASE_COMMAND,
                    "The rebase stopped, usually for a conflict. Resolve and stage the files, then continue, or abort to return to the original commits.",
                ));
            }
            return Err(error);
        }
        Ok(())
    }

    /// Ends a group of kept commits: stores its message and amends the commit with it, when the
    /// message was changed by a reword or a squash.
    fn finish_group(&self, todo: &mut Vec<String>, group: Option<Group>, scratch: &Scratch, directory: &Path) -> Result<()> {
        let Some(group) = group.filter(|group| group.changed) else { return Ok(()) };
        let file = scratch.write(REBASE_COMMAND, &format!("message-{}", todo.len()), group.message.as_bytes())?;
        let blob = self.run_trimmed(&["hash-object", "--no-filters", "-w", "--", &path_text(&file)], directory)?;
        todo.push(format!("exec git cat-file blob {blob} | git commit --amend --only --no-verify --allow-empty --cleanup=whitespace -F -"));
        Ok(())
    }

    /// Starts a rebase onto `target`, a merge, a revert, or a cherry-pick of one commit. A revert
    /// or cherry-pick of a merge needs `mainline`, the parent to keep (1 for the first parent).
    /// Refuses unless the checkout matches `expected`, nothing is in progress, and the working
    /// tree is clean.
    pub fn start(
        &self,
        operation: Operation,
        target: &str,
        mainline: Option<usize>,
        expected: Expectation,
        directory: &Path,
    ) -> Result<()> {
        let command = operation.name();
        let state = self.checkout_state(directory)?;
        let moved = expected.head.is_some_and(|head| state.head_hash.as_deref() != Some(head))
            || expected.branch.is_some_and(|branch| state.current_branch != branch);
        if moved {
            return Err(GitError::failed(
                command,
                "The current branch or HEAD changed since this action was selected. Refresh and review the operation again.",
            ));
        }
        if state.operation.is_some() || !self.load_status(directory)?.is_empty() {
            return Err(GitError::failed(command, "Commit or stash changes and finish the current operation first."));
        }
        let hash = self.resolve_commit(target, directory)?;
        if let Some(source) = expected.source_branch {
            self.require_source_tip(source, command, directory)?;
            if hash != source.tip {
                return Err(GitError::failed(
                    command,
                    "The selected branch changed since this action was selected. Refresh and review it again.",
                ));
            }
        }
        let mut arguments = vec![command];
        if operation == Operation::Merge {
            if state.head_hash.is_some() {
                self.require_no_ignored_merge_collisions(&hash, directory)?;
            }
            arguments.push("--no-overwrite-ignore");
        }
        if matches!(operation, Operation::Merge | Operation::Revert) {
            arguments.push("--no-edit");
        }
        let mainline_text = mainline.map(|parent| parent.to_string());
        if let Some(parent) = mainline_text.as_deref() {
            if matches!(operation, Operation::Revert | Operation::CherryPick) {
                arguments.extend(["--mainline", parent]);
            }
        }
        arguments.push(&hash);
        self.run(&arguments, directory).map(drop)
    }

    /// Cherry-picks several commits, oldest first by committer time, as one sequence. Conflicts
    /// stop the sequence for Continue or Abort, like a single cherry-pick.
    pub fn cherry_pick(&self, commits: &[String], expected_head: Option<&str>, expected_branch: &str, directory: &Path) -> Result<()> {
        const COMMAND: &str = "cherry-pick";
        let state = self.checkout_state(directory)?;
        if state.head_hash.as_deref() != expected_head || state.current_branch != expected_branch {
            return Err(GitError::failed(
                COMMAND,
                "The current branch or HEAD changed since this action was selected. Refresh and review the operation again.",
            ));
        }
        if state.operation.is_some() || !self.load_status(directory)?.is_empty() {
            return Err(GitError::failed(COMMAND, "Commit or stash changes and finish the current operation first."));
        }
        let unique: BTreeSet<&str> = commits.iter().map(String::as_str).collect();
        let mut resolved: Vec<(i64, String)> = Vec::new();
        let mut has_merge = false;
        for commit in unique {
            let hash = self.resolve_commit(commit, directory)?;
            let output = self.run_trimmed(&["show", "--no-patch", "--format=%H %ct %P", &hash, "--"], directory)?;
            let fields: Vec<&str> = output.split_whitespace().collect();
            // Fields: hash, committer time, then at most one parent. Two parents mark a merge.
            has_merge |= fields.len() > 3;
            resolved.push((fields.get(1).and_then(|time| time.parse().ok()).unwrap_or(0), fields[0].to_string()));
        }
        if has_merge && resolved.len() > 1 {
            return Err(GitError::failed(
                COMMAND,
                "Merge commits cannot be cherry-picked together with others. Pick a merge on its own and choose its parent.",
            ));
        }
        if resolved.is_empty() {
            return Ok(());
        }
        resolved.sort();
        let mut arguments = vec!["cherry-pick"];
        arguments.extend(resolved.iter().map(|(_, hash)| hash.as_str()));
        self.run(&arguments, directory).map(drop)
    }

    /// Whether a remote-tracking branch already contains `commit`.
    pub fn is_published(&self, commit: &str, directory: &Path) -> Result<bool> {
        let id = self.resolve_commit(commit, directory)?;
        Ok(!self.run_trimmed(&["branch", "--remotes", "--contains", &id], directory)?.is_empty())
    }

    /// Confirms a branch, local or remote, still points to the tip that was shown.
    fn require_source_tip(&self, branch: &Branch, command: &str, directory: &Path) -> Result<()> {
        let reference = if branch.is_remote { format!("refs/{}", branch.name) } else { format!("refs/heads/{}", branch.name) };
        let current = self.run_trimmed(&["rev-parse", "--verify", "--end-of-options", &reference], directory)?;
        if current != branch.tip {
            return Err(GitError::failed(
                command,
                "The selected branch changed since this action was selected. Refresh and review it again.",
            ));
        }
        Ok(())
    }
}

/// Quotes text for the shell that runs the sequence editor.
fn shell_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
}

/// The todo path as the shell sees it. Git runs the editor through its bundled shell on
/// Windows, where a path with backslashes would be read as escapes.
fn sequence_editor_path(path: &Path) -> String {
    let text = path_text(path);
    if cfg!(windows) {
        text.replace('\\', "/")
    } else {
        text
    }
}

pub(crate) fn path_text(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

/// A failure to prepare a file in the operating system's temporary folder.
pub(crate) fn io_failure(command: &str, error: io::Error) -> GitError {
    GitError::failed(command, format!("Could not prepare a temporary file: {error}"))
}

/// A private folder in the system's temporary location, removed when dropped. It holds the
/// files Git reads during an operation, such as a todo list or a stored message.
pub(crate) struct Scratch {
    path: PathBuf,
}

impl Scratch {
    pub(crate) fn new(command: &str) -> Result<Self> {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map(|elapsed| elapsed.as_nanos()).unwrap_or(0);
        let name = format!("nicegit-{}-{nanos}-{}", std::process::id(), COUNTER.fetch_add(1, Ordering::Relaxed));
        let path = std::env::temp_dir().join(name);
        fs::create_dir_all(&path).map_err(|error| io_failure(command, error))?;
        Ok(Scratch { path })
    }

    /// Writes a file in this folder and returns its path.
    pub(crate) fn write(&self, command: &str, name: &str, contents: &[u8]) -> Result<PathBuf> {
        let file = self.path.join(name);
        fs::write(&file, contents).map_err(|error| io_failure(command, error))?;
        Ok(file)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}
