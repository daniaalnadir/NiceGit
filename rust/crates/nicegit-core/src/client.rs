use std::collections::{BTreeMap, HashSet};
use std::io::Read;
use std::path::{Path, PathBuf};

use crate::diff::DiffRequest;
use crate::models::*;
use crate::parsers::*;
use crate::rebase::edit_stop_in;
use crate::runner::{self, RunOptions};

const CHANGED_SINCE_SELECTED: &str = "This file changed since it was selected. Refresh and review it again.";
const CHECKOUT_CHANGED: &str = "The current checkout changed since this action was selected. Refresh and review it again.";
const BRANCH_CHANGED: &str = "This branch changed since it was selected. Refresh and review it again.";
/// The largest `commit.template` file read. A commit message is never this long.
const COMMIT_TEMPLATE_LIMIT: u64 = 64 * 1024;

/// What a force push would change on the remote, from `GitClient::force_push_preview`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ForcePushPreview {
    /// The remote-tracking branch's tip: the commit the lease expects the remote to still have.
    pub remote_tip: String,
    /// Commits on the remote that the branch lacks, newest first, at most 50 of them.
    pub removed: Vec<Commit>,
    /// How many commits on the remote the branch lacks, which a force push would remove.
    pub removed_count: usize,
    /// How many commits the branch has that the remote lacks.
    pub added_count: usize,
}

/// Whether the status left after `git stash push` holds only what `options` intentionally keeps.
/// Untracked files can remain only without `include_untracked`, and only if they were untracked
/// before. With `keep_index`, a staged entry can remain only if its staged side is unchanged and
/// its working-tree side is clean. Anything else, such as a dirty submodule, is unexpected.
fn stash_left_expected(before: &[StatusEntry], after: &[StatusEntry], options: StashOptions) -> bool {
    after.iter().all(|entry| {
        if entry.kind == StatusKind::Untracked {
            return !options.include_untracked && before.iter().any(|old| old.kind == StatusKind::Untracked && old.path == entry.path);
        }
        options.keep_index
            && entry.work_tree_status == ' '
            && before
                .iter()
                .any(|old| old.index_status == entry.index_status && old.path == entry.path && old.original_path == entry.original_path)
    })
}

/// Whether `value` is a full object ID, as Git prints them: 40 hex digits, or 64 for SHA-256.
fn is_object_id(value: &str) -> bool {
    matches!(value.len(), 40 | 64) && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

/// Refuses an archive folder name that is not a single safe path component. The name becomes
/// the top folder of the archive, so it must not add directories, climb out of the archive, or
/// read as an option.
fn validate_archive_prefix(prefix: &str) -> Result<()> {
    let safe = !prefix.is_empty()
        && prefix != "."
        && !prefix.contains("..")
        && !prefix.starts_with('-')
        && !prefix.chars().any(|c| matches!(c, '/' | '\\' | '\0') || c.is_control());
    if safe {
        Ok(())
    } else {
        Err(GitError::failed("archive", "The archive folder name must be a single name without slashes, \"..\", or a leading dash."))
    }
}

/// Runs Git commands for one or more repositories. Every method takes the checkout it acts on,
/// and every action that changes files or refs first confirms the checkout it was chosen for.
#[derive(Clone, Debug, Default)]
pub struct GitClient {
    /// Let `git status` refresh the index. Only deliberate loads (opening, explicit refresh)
    /// set this; automatic refreshes leave the index untouched.
    pub status_updates_index: bool,
}

/// Full details of one commit, for the inspector.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommitDetails {
    pub hash: String,
    pub parents: Vec<String>,
    pub author: String,
    pub author_email: String,
    pub author_date: String,
    pub committer: String,
    pub committer_date: String,
    pub message: String,
}

/// A selected file's change, as the action that was chosen for it last saw it.
pub type Selected<'a> = &'a StatusEntry;

impl GitClient {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn deliberate() -> Self {
        Self { status_updates_index: true }
    }

    pub(crate) fn options(&self) -> RunOptions<'static> {
        RunOptions { accepted: &[], status_updates_index: self.status_updates_index, env: &[] }
    }

    pub fn run(&self, arguments: &[&str], directory: &Path) -> Result<String> {
        runner::run(arguments, directory, &self.options())
    }

    pub(crate) fn run_accepting(&self, arguments: &[&str], directory: &Path, accepted: &[i32]) -> Result<String> {
        runner::run(arguments, directory, &RunOptions { accepted, ..self.options() })
    }

    pub(crate) fn run_trimmed(&self, arguments: &[&str], directory: &Path) -> Result<String> {
        Ok(self.run(arguments, directory)?.trim().to_string())
    }

    pub(crate) fn try_trimmed(&self, arguments: &[&str], directory: &Path) -> Option<String> {
        self.run_trimmed(arguments, directory).ok().filter(|value| !value.is_empty())
    }

    // MARK: Repository

    pub fn git_version(&self) -> Result<String> {
        self.run_trimmed(&["--version"], &std::env::temp_dir())
    }

    pub fn repository_root(&self, directory: &Path) -> Result<PathBuf> {
        let output = runner::strip_line_terminator(self.run(&["rev-parse", "--show-toplevel"], directory)?);
        Ok(PathBuf::from(output))
    }

    pub fn initialize(&self, directory: &Path) -> Result<()> {
        self.run(&["init", "--", &directory.to_string_lossy()], directory.parent().unwrap_or(directory))?;
        Ok(())
    }

    pub fn clone_repository(&self, source: &str, destination: &Path) -> Result<()> {
        let parent = destination.parent().ok_or_else(|| GitError::failed("clone", "Choose a folder inside another folder."))?;
        self.run(&["clone", "--", source, &destination.to_string_lossy()], parent)?;
        Ok(())
    }

    /// This checkout's own Git directory (per worktree), as an absolute path.
    pub fn git_directory(&self, directory: &Path) -> Result<PathBuf> {
        Ok(PathBuf::from(runner::strip_line_terminator(self.run(&["rev-parse", "--absolute-git-dir"], directory)?)))
    }

    pub fn current_operation(&self, directory: &Path) -> Result<Option<Operation>> {
        Ok(operation_in(&self.git_directory(directory)?))
    }

    pub(crate) fn current_branch(&self, directory: &Path) -> String {
        if let Some(branch) = self.try_trimmed(&["branch", "--show-current"], directory) {
            return branch;
        }
        match self.try_trimmed(&["rev-parse", "--short", "HEAD"], directory) {
            Some(head) => format!("Detached HEAD {head}"),
            None => "No commits yet".to_string(),
        }
    }

    pub(crate) fn head(&self, directory: &Path) -> Option<String> {
        self.try_trimmed(&["rev-parse", "--verify", "--quiet", "HEAD"], directory)
    }

    pub fn checkout_state(&self, directory: &Path) -> Result<CheckoutState> {
        let commands = vec![
            vec!["branch".to_string(), "--show-current".to_string()],
            vec!["rev-parse".into(), "--verify".into(), "--quiet".into(), "HEAD".into()],
            vec!["rev-parse".into(), "--absolute-git-dir".into()],
        ];
        let mut results = runner::run_concurrently(&commands, directory, &self.options()).into_iter();
        let branch = results.next().unwrap()?.trim().to_string();
        let head = results.next().unwrap().unwrap_or_default().trim().to_string();
        let git_directory = runner::strip_line_terminator(results.next().unwrap()?);
        let current_branch = if branch.is_empty() { self.current_branch(directory) } else { branch };
        Ok(CheckoutState {
            current_branch,
            head_hash: (!head.is_empty()).then_some(head),
            operation: operation_in(Path::new(&git_directory)),
        })
    }

    /// Confirms the checkout still has the branch and HEAD captured when an action was shown.
    pub fn require_checkout(
        &self,
        expected_branch: &str,
        expected_head: Option<&str>,
        command: &str,
        directory: &Path,
    ) -> Result<CheckoutState> {
        let state = self.checkout_state(directory)?;
        if state.current_branch != expected_branch || state.head_hash.as_deref() != expected_head {
            return Err(GitError::failed(command, CHECKOUT_CHANGED));
        }
        Ok(state)
    }

    pub(crate) fn require_finished_operation(&self, command: &str, directory: &Path) -> Result<()> {
        if self.current_operation(directory)?.is_some() {
            return Err(GitError::failed(command, "Finish or abort the current Git operation before changing the working tree."));
        }
        Ok(())
    }

    pub(crate) fn resolve_commit(&self, revision: &str, directory: &Path) -> Result<String> {
        self.run_trimmed(&["rev-parse", "--verify", "--end-of-options", &format!("{revision}^{{commit}}")], directory)
    }

    // MARK: Snapshot

    /// The revisions the history log walks for `scope`, before the log's own options.
    ///
    /// Git's default, HEAD, fails on a repository without commits, so HEAD is named only once
    /// it resolves. An upstream is named by its object ID, never by its name.
    fn history_revisions(&self, scope: HistoryScope, root: &Path) -> Vec<String> {
        match scope {
            HistoryScope::All => vec!["--exclude=refs/stash".to_string(), "--all".to_string()],
            HistoryScope::Local => {
                let mut revisions = vec!["--branches".to_string()];
                if self.head(root).is_some() {
                    revisions.push("HEAD".to_string());
                }
                revisions
            }
            HistoryScope::Current => {
                let mut revisions = Vec::new();
                if self.head(root).is_some() {
                    revisions.push("HEAD".to_string());
                }
                if let Some(upstream) = self.try_trimmed(&["rev-parse", "--verify", "--quiet", "@{upstream}"], root) {
                    if is_object_id(&upstream) {
                        revisions.push(upstream);
                    }
                }
                if revisions.is_empty() {
                    // Without commits there is nothing to show. A pattern that names no ref
                    // (a space cannot appear in a ref name) keeps Git from falling back to HEAD.
                    revisions.push("--branches= ".to_string());
                }
                revisions
            }
        }
    }

    /// Reads the repository's state. Independent reads run at the same time, so a refresh takes
    /// about as long as its slowest command.
    pub fn load_snapshot(&self, selected: &Path, history_limit: usize) -> Result<Snapshot> {
        self.load_snapshot_scoped(selected, history_limit, HistoryScope::All)
    }

    /// Like `load_snapshot`, but the history covers only the branches `scope` names.
    pub fn load_snapshot_scoped(&self, selected: &Path, history_limit: usize, scope: HistoryScope) -> Result<Snapshot> {
        let root = self.repository_root(selected)?;
        let limit = history_limit.max(1);
        let log_format = format!("--pretty=format:{LOG_FORMAT}");
        let branch_format = format!("--format={BRANCH_FORMAT}");
        let stash_format = format!("--format={STASH_FORMAT}");
        let count = (limit + 1).to_string();
        let revisions = self.history_revisions(scope, &root);
        let mut log = vec!["log"];
        log.extend(revisions.iter().map(String::as_str));
        log.extend(["--topo-order", "--decorate=short", "--date=relative", "-n", count.as_str(), log_format.as_str(), "--"]);
        let commands: Vec<Vec<String>> = [
            vec!["status", "--porcelain=v1", "-z", "--untracked-files=all"],
            vec!["branch", "--all", &branch_format],
            log,
            vec!["remote", "-v"],
            vec!["worktree", "list", "--porcelain", "-z"],
            vec!["for-each-ref", "--sort=-version:refname", "--format=%(refname:strip=2)%09%(objectname)", "refs/tags"],
            vec!["rev-parse", "--absolute-git-dir"],
            vec!["rev-list", "--left-right", "--count", "HEAD...@{upstream}", "--"],
            vec!["stash", "list", &stash_format],
        ]
        .into_iter()
        .map(|command| command.into_iter().map(str::to_string).collect())
        .collect();
        let mut results = runner::run_concurrently(&commands, &root, &self.options());
        let counts = std::mem::replace(&mut results[7], Ok(String::new())).unwrap_or_default();
        let outputs: Vec<String> = results.into_iter().collect::<Result<_>>()?;

        let status = parse_status(&outputs[0]);
        let branches = parse_branches(&outputs[1]);
        let current = branches.iter().find(|branch| branch.is_current).cloned();
        let current_branch = match &current {
            Some(branch) if !branch.is_detached() => branch.name.clone(),
            _ => self.current_branch(&root),
        };
        let head_hash = current.as_ref().map(|branch| branch.tip.clone()).or_else(|| self.head(&root));
        let commits = parse_log(&outputs[2]);
        let mut visible: Vec<Commit> = commits.iter().take(limit).cloned().collect();
        // Keep HEAD visible even when other branches fill the first page.
        if let Some(head) = &head_hash {
            if !visible.iter().any(|commit| &commit.hash == head) {
                if let Some(commit) = commits.iter().find(|commit| &commit.hash == head) {
                    visible.push(commit.clone());
                } else if let Ok(output) = self.run(&["log", "-1", "--decorate=short", &log_format, "HEAD", "--"], &root) {
                    visible.extend(parse_log(&output).into_iter().take(1));
                }
            }
        }
        let mut snapshot = Snapshot {
            root_path: root.to_string_lossy().into_owned(),
            name: root.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default(),
            current_branch,
            status,
            branches,
            has_more_commits: commits.len() > limit,
            commits: visible,
            remotes: parse_remotes(&outputs[3]),
            remote_fetch_addresses: parse_remote_addresses(&outputs[3], "fetch"),
            remote_push_addresses: parse_remote_addresses(&outputs[3], "push"),
            worktrees: parse_worktrees(&outputs[4]),
            stashes: parse_stashes(&outputs[8]),
            head_hash,
            ..Snapshot::default()
        };
        for (name, tip) in parse_tags(&outputs[5]) {
            snapshot.tags.push(name.clone());
            snapshot.tag_tips.insert(name, tip);
        }
        if let Some(upstream) = current.as_ref().and_then(|branch| branch.upstream.clone()) {
            let values: Vec<usize> = counts.split_whitespace().filter_map(|value| value.parse().ok()).collect();
            let short =
                upstream.strip_prefix("refs/remotes/").or_else(|| upstream.strip_prefix("refs/heads/")).unwrap_or(&upstream).to_string();
            snapshot.upstream = Some(short);
            if let [ahead, behind] = values[..] {
                snapshot.ahead = Some(ahead);
                snapshot.behind = Some(behind);
            }
        }
        let git_directory = PathBuf::from(runner::strip_line_terminator(outputs[6].clone()));
        snapshot.operation = operation_in(&git_directory);
        snapshot.rebase_stopped_for_edit = edit_stop_in(&git_directory, &snapshot.status);
        snapshot.bisecting = git_directory.join("BISECT_START").exists();
        snapshot.has_gitmodules = root.join(".gitmodules").exists();
        Ok(snapshot)
    }

    pub fn load_status(&self, directory: &Path) -> Result<Vec<StatusEntry>> {
        Ok(parse_status(&self.run(&["status", "--porcelain=v1", "-z", "--untracked-files=all"], directory)?))
    }

    pub(crate) fn load_status_for(&self, directory: &Path, paths: &[&str]) -> Result<Vec<StatusEntry>> {
        let mut arguments = vec!["status", "--porcelain=v1", "-z", "--untracked-files=all", "--"];
        arguments.extend_from_slice(paths);
        Ok(parse_status(&self.run(&arguments, directory)?))
    }

    // MARK: Changes

    pub fn stage(&self, path: &str, directory: &Path) -> Result<()> {
        self.run(&["add", "--", path], directory).map(drop)
    }

    pub fn stage_all(&self, directory: &Path) -> Result<()> {
        self.run(&["add", "--all"], directory).map(drop)
    }

    pub fn unstage(&self, entry: Selected, directory: &Path) -> Result<()> {
        let mut arguments = vec!["restore", "--staged", "--", entry.path.as_str()];
        // A detected copy carries a source path too, but only the copied path is unstaged.
        if entry.kind == StatusKind::Renamed {
            if let Some(original) = entry.original_path.as_deref() {
                arguments.push(original);
            }
        }
        if self.head(directory).is_none() {
            // Before the first commit there is nothing to restore from; keep the working file.
            return self.run(&["rm", "--cached", "--force", "--", &entry.path], directory).map(drop);
        }
        self.run(&arguments, directory).map(drop)
    }

    pub fn unstage_all(&self, directory: &Path) -> Result<()> {
        if self.head(directory).is_none() {
            self.run(&["rm", "--cached", "--force", "-r", "--quiet", "--", "."], directory).map(drop)
        } else {
            self.run(&["restore", "--staged", "."], directory).map(drop)
        }
    }

    /// Discards staged and unstaged changes to one path, or removes it when untracked. Refuses
    /// if the path changed since it was selected, and verifies it is clean afterwards.
    pub fn discard(&self, entry: Selected, directory: &Path) -> Result<()> {
        let rename_source = (entry.kind == StatusKind::Renamed).then_some(entry.original_path.as_deref()).flatten();
        let mut paths = vec![entry.path.as_str()];
        paths.extend(rename_source);
        if !self.load_status_for(directory, &paths)?.contains(entry) {
            return Err(GitError::failed("discard", CHANGED_SINCE_SELECTED));
        }
        if entry.kind == StatusKind::Untracked {
            self.run(&["clean", "--force", "--", &entry.path], directory)?;
        } else if self.head(directory).is_none() {
            self.run(&["rm", "--force", "--", &entry.path], directory)?;
        } else {
            let mut arguments = vec!["restore", "--source=HEAD", "--staged", "--worktree", "--"];
            arguments.extend_from_slice(&paths);
            self.run(&arguments, directory)?;
        }
        // Git can report success while changes remain, for example in a dirty submodule.
        let after = self.load_status(directory)?;
        if after.iter().any(|remaining| paths.contains(&remaining.path.as_str())) {
            let message = if entry.kind == StatusKind::Untracked {
                "Git could not remove this untracked path. Nested repositories require manual removal."
            } else {
                "Changes remain after Git restored this path. If it is a submodule, open it and discard its changes there."
            };
            return Err(GitError::failed("discard", message));
        }
        Ok(())
    }

    /// The diff of one file with Git's default context.
    pub fn diff(&self, entry: Selected, staged: bool, directory: &Path) -> Result<String> {
        self.diff_with(entry, staged, DiffRequest::default(), directory)
    }

    /// The diff of one file, hiding whitespace-only changes and showing the requested context
    /// when asked. Untracked files are shown whole, as new files, and ignore the whitespace option.
    pub fn diff_with(&self, entry: Selected, staged: bool, request: DiffRequest, directory: &Path) -> Result<String> {
        let context = request.context.argument();
        if entry.kind == StatusKind::Untracked {
            let mut arguments = vec!["diff", "--no-index", "--no-ext-diff", "--no-color"];
            arguments.extend(context.as_deref());
            arguments.extend(["--", "/dev/null", entry.path.as_str()]);
            return self.run_accepting(&arguments, directory, &[0, 1]);
        }
        let mut arguments = vec!["diff", "--no-ext-diff", "--no-color"];
        if request.ignore_whitespace {
            arguments.push("-w");
        }
        arguments.extend(context.as_deref());
        if staged {
            arguments.push("--cached");
        }
        arguments.extend(["--", entry.path.as_str()]);
        // Only a rename shows its source; a copy's source is unchanged.
        if entry.kind == StatusKind::Renamed && staged {
            arguments.extend(entry.original_path.as_deref());
        }
        self.run(&arguments, directory)
    }

    // MARK: Commits

    pub fn commit(&self, message: &str, directory: &Path) -> Result<()> {
        let message = message.trim();
        if message.is_empty() {
            return Err(GitError::EmptyCommitMessage);
        }
        self.run(&["commit", "--quiet", "-m", message], directory).map(drop)
    }

    /// The message in the file `commit.template` names, cleaned up as `git commit` cleans a
    /// message: comment lines go, trailing whitespace is trimmed, and blank lines at either end
    /// are dropped. `None` when the setting is unset, the file is missing, unreadable, not a
    /// regular file, or over 64 KB, or nothing is left after the cleanup.
    pub fn commit_template(&self, directory: &Path) -> Result<Option<String>> {
        // Read as a path, so a leading `~` expands as Git expands it.
        let Ok(value) = self.run_accepting(&["config", "--type=path", "--get", "commit.template"], directory, &[0, 1]) else {
            return Ok(None);
        };
        let value = runner::strip_line_terminator(value);
        if value.is_empty() {
            return Ok(None);
        }
        let mut path = PathBuf::from(&value);
        if path.is_relative() {
            // Git resolves a relative template against the top of the working tree.
            let Ok(root) = self.repository_root(directory) else { return Ok(None) };
            path = root.join(path);
        }
        let Ok(metadata) = std::fs::metadata(&path) else { return Ok(None) };
        if !metadata.is_file() || metadata.len() > COMMIT_TEMPLATE_LIMIT {
            return Ok(None);
        }
        let Ok(file) = std::fs::File::open(&path) else { return Ok(None) };
        // Reads one byte past the limit, so a file that grew since it was measured is still refused.
        let mut bytes = Vec::new();
        if file.take(COMMIT_TEMPLATE_LIMIT + 1).read_to_end(&mut bytes).is_err() || bytes.len() as u64 > COMMIT_TEMPLATE_LIMIT {
            return Ok(None);
        }
        // Git's comment character is `core.commentChar`; `auto` picks one, which is taken as `#`.
        let comment = match self.config("core.commentchar", false, directory).ok().flatten() {
            Some(value) if value != "auto" => {
                let mut chars = value.chars();
                match (chars.next(), chars.next()) {
                    (Some(character), None) => character,
                    _ => '#',
                }
            }
            _ => '#',
        };
        let text = String::from_utf8_lossy(&bytes);
        let mut lines: Vec<&str> = Vec::new();
        for line in text.lines() {
            if line.starts_with(comment) {
                continue;
            }
            let line = line.trim_end();
            // Git's cleanup collapses a run of blank lines into one.
            if line.is_empty() && lines.last().is_some_and(|last| last.is_empty()) {
                continue;
            }
            lines.push(line);
        }
        let Some(first) = lines.iter().position(|line| !line.is_empty()) else { return Ok(None) };
        let last = lines.iter().rposition(|line| !line.is_empty()).unwrap_or(first);
        Ok(Some(lines[first..=last].join("\n")))
    }

    /// Replaces the last commit with one that also contains the staged changes.
    pub fn amend(&self, message: &str, expected_branch: &str, expected_head: &str, directory: &Path) -> Result<()> {
        let message = message.trim();
        if message.is_empty() {
            return Err(GitError::EmptyCommitMessage);
        }
        let state = self.require_checkout(expected_branch, Some(expected_head), "amend commit", directory)?;
        if state.operation.is_some() {
            return Err(GitError::failed("amend commit", "Finish or abort the current Git operation before amending."));
        }
        self.run(&["commit", "--quiet", "--amend", "--message", message], directory).map(drop)
    }

    /// The commit HEAD points to, if any.
    pub fn head_commit(&self, directory: &Path) -> Option<String> {
        self.head(directory)
    }

    pub fn commit_details(&self, hash: &str, directory: &Path) -> Result<CommitDetails> {
        let output = self.run(
            &[
                "show",
                "--no-patch",
                "--no-color",
                "--date=format:%e %b %Y, %H:%M",
                "--format=%H%x1f%P%x1f%an%x1f%ae%x1f%ad%x1f%cn%x1f%cd%x1f%B",
                hash,
                "--",
            ],
            directory,
        )?;
        let fields: Vec<&str> = output.splitn(8, '\u{1f}').collect();
        let [hash, parents, author, email, author_date, committer, committer_date, message] = fields[..] else {
            return Err(GitError::failed("show", "Git returned commit details in an unexpected format."));
        };
        Ok(CommitDetails {
            hash: hash.to_string(),
            parents: parents.split_whitespace().map(str::to_string).collect(),
            author: author.to_string(),
            author_email: email.to_string(),
            author_date: author_date.trim().to_string(),
            committer: committer.to_string(),
            committer_date: committer_date.trim().to_string(),
            message: message.trim_end().to_string(),
        })
    }

    pub fn commit_message(&self, hash: &str, directory: &Path) -> Result<String> {
        self.run(&["show", "--no-patch", "--format=%B", "--no-color", hash, "--"], directory)
    }

    /// Undoes the last commit on the current branch, keeping its changes staged.
    pub fn undo_last_commit(&self, expected_branch: &str, expected_head: &str, directory: &Path) -> Result<String> {
        let state = self.require_checkout(expected_branch, Some(expected_head), "undo commit", directory)?;
        if state.operation.is_some() {
            return Err(GitError::failed("undo commit", "Finish or abort the current Git operation before undoing a commit."));
        }
        let parent = self
            .resolve_commit(&format!("{expected_head}^"), directory)
            .map_err(|_| GitError::failed("undo commit", "The first commit has no parent to return to."))?;
        self.run(&["reset", "--soft", &parent, "--"], directory)?;
        Ok(parent)
    }

    /// Moves the current branch to the commit it pointed to before an undo, while the index and
    /// working files still hold that commit's changes.
    pub fn redo_commit(&self, commit: &str, expected_branch: &str, expected_head: &str, directory: &Path) -> Result<()> {
        let state = self.require_checkout(expected_branch, Some(expected_head), "redo commit", directory)?;
        if state.operation.is_some() {
            return Err(GitError::failed("redo commit", "Finish or abort the current Git operation first."));
        }
        let target = self.resolve_commit(commit, directory)?;
        self.run(&["reset", "--soft", &target, "--"], directory).map(drop)
    }

    pub fn reset(&self, target: &str, mode: ResetMode, expected_branch: &str, expected_head: &str, directory: &Path) -> Result<()> {
        let state = self.require_checkout(expected_branch, Some(expected_head), "reset", directory)?;
        if state.operation.is_some() {
            return Err(GitError::failed("reset", "Finish or abort the current Git operation before resetting."));
        }
        let hash = self.resolve_commit(target, directory)?;
        self.run(&["reset", mode.flag(), &hash, "--"], directory).map(drop)
    }

    /// Patch and summary for a commit; merges show changes against their first parent.
    pub fn commit_diff(&self, hash: &str, path: Option<&str>, directory: &Path) -> Result<String> {
        let mut arguments = vec!["show", "--first-parent", "-m", "--format=", "--patch", "--no-ext-diff", "--no-color", hash, "--"];
        arguments.extend(path);
        self.run(&arguments, directory)
    }

    pub fn commit_diff_with(&self, hash: &str, path: Option<&str>, request: DiffRequest, directory: &Path) -> Result<String> {
        let context = request.context.argument();
        let mut arguments = vec!["show", "--first-parent", "-m", "--format=", "--patch", "--no-ext-diff", "--no-color"];
        if request.ignore_whitespace {
            arguments.push("-w");
        }
        arguments.extend(context.as_deref());
        arguments.extend([hash, "--"]);
        arguments.extend(path);
        self.run(&arguments, directory)
    }

    /// Files a commit changed, with Git's change letter, sorted by path.
    pub fn commit_files(&self, hash: &str, directory: &Path) -> Result<Vec<(String, String)>> {
        let output = self.run(
            &["diff-tree", "--root", "--no-commit-id", "--first-parent", "-m", "-r", "--no-renames", "--name-status", "-z", hash, "--"],
            directory,
        )?;
        let fields: Vec<&str> = output.split('\0').collect();
        let mut files: Vec<(String, String)> = fields
            .chunks(2)
            .filter(|pair| pair.len() == 2 && !pair[1].is_empty())
            .map(|pair| (pair[0].to_string(), pair[1].to_string()))
            .collect();
        files.sort_by(|a, b| a.1.cmp(&b.1));
        files.dedup();
        Ok(files)
    }

    // MARK: Branches

    pub(crate) fn require_branch_tip(&self, branch: &str, expected_tip: &str, directory: &Path) -> Result<()> {
        let current = self.run_trimmed(&["rev-parse", "--verify", "--end-of-options", &format!("refs/heads/{branch}")], directory)?;
        if current != expected_tip {
            return Err(GitError::failed("branch", BRANCH_CHANGED));
        }
        Ok(())
    }

    /// Switches to a local branch, carrying uncommitted changes across in a stash when needed.
    /// Returns true when changes were stashed and restored (or kept in a visible stash).
    pub fn checkout(&self, branch: &Branch, expected_branch: &str, expected_head: Option<&str>, directory: &Path) -> Result<bool> {
        self.require_checkout(expected_branch, expected_head, "switch branch", directory)?;
        self.require_branch_tip(&branch.name, &branch.tip, directory)?;
        self.switch_preserving_changes(&["switch", "--no-overwrite-ignore", "--", &branch.name], &branch.name, directory)
    }

    /// Checks out a remote branch: switches to the one local branch tracking it, or creates one.
    pub fn checkout_remote(
        &self,
        branch: &Branch,
        remotes: &[String],
        expected_branch: &str,
        expected_head: Option<&str>,
        directory: &Path,
    ) -> Result<bool> {
        self.require_checkout(expected_branch, expected_head, "switch branch", directory)?;
        let reference = format!("refs/{}", branch.name);
        let current = self.run_trimmed(&["rev-parse", "--verify", "--end-of-options", &reference], directory)?;
        if current != branch.tip {
            return Err(GitError::failed(
                "checkout remote branch",
                "This remote branch changed since it was selected. Refresh and review it again.",
            ));
        }
        let branches = parse_branches(&self.run(&["branch", "--all", &format!("--format={BRANCH_FORMAT}")], directory)?);
        let tracking: Vec<&Branch> =
            branches.iter().filter(|b| !b.is_remote && b.upstream.as_deref() == Some(reference.as_str())).collect();
        if tracking.len() > 1 {
            return Err(GitError::failed(
                "checkout remote branch",
                "Several local branches track this remote branch. Choose the desired branch in Local.",
            ));
        }
        if let Some(existing) = tracking.first() {
            if existing.is_current {
                return Ok(false);
            }
            self.require_branch_tip(&existing.name, &existing.tip, directory)?;
            return self.switch_preserving_changes(&["switch", "--no-overwrite-ignore", "--", &existing.name], &existing.name, directory);
        }
        let remote = branch.remote_name(remotes).ok_or_else(|| {
            GitError::failed("checkout remote branch", "This remote is no longer available. Refresh and select a branch again.")
        })?;
        let remote_path = branch.name.strip_prefix("remotes/").unwrap_or(&branch.name);
        let preferred = &remote_path[remote.len() + 1..];
        let local_names: Vec<&str> = branches.iter().filter(|b| !b.is_remote).map(|b| b.name.as_str()).collect();
        let available = |name: &str| {
            !local_names
                .iter()
                .any(|local| *local == name || local.starts_with(&format!("{name}/")) || name.starts_with(&format!("{local}/")))
                && self.run(&["check-ref-format", "--branch", name], directory).is_ok()
        };
        let mut name = preferred.to_string();
        if !available(&name) {
            // When remotes share a branch name, create a distinct local tracking branch.
            let flattened = remote_path.replace('/', "-");
            let base = if self.run(&["check-ref-format", "--branch", &flattened], directory).is_ok() {
                flattened.clone()
            } else {
                format!("remote-{flattened}")
            };
            name = base.clone();
            let mut suffix = 2;
            while !available(&name) {
                name = format!("{base}-{suffix}");
                suffix += 1;
            }
        }
        self.switch_preserving_changes(
            &["switch", "--no-overwrite-ignore", "--track", "--create", &name, "--", &reference],
            &reference,
            directory,
        )
    }

    pub(crate) fn switch_preserving_changes(&self, arguments: &[&str], target: &str, directory: &Path) -> Result<bool> {
        self.require_finished_operation("switch branch", directory)?;
        if self.load_status(directory)?.is_empty() {
            self.run(arguments, directory)?;
            return Ok(false);
        }
        let source = self.current_branch(directory);
        if source == target {
            return Ok(false);
        }
        let previous = self.try_trimmed(&["rev-parse", "--verify", "--quiet", "refs/stash"], directory);
        let message = format!("NiceGit: changes from {source} before switching to {target}");
        self.run(&["stash", "push", "--include-untracked", "-m", &message], directory)?;
        let saved = self.try_trimmed(&["rev-parse", "--verify", "--quiet", "refs/stash"], directory);
        let Some(stash) = saved.filter(|saved| Some(saved) != previous.as_ref()) else {
            return Err(GitError::failed(
                "switch branch",
                "Git could not save all working changes. The branch was not switched. Check the working tree and Stashes before retrying.",
            ));
        };
        let switched = (|| {
            // A superproject stash does not save dirty submodule files.
            if !self.load_status(directory)?.is_empty() {
                return Err(GitError::failed(
                    "switch branch",
                    "Git could not stash every change, including changes inside submodules. The branch was not switched.",
                ));
            }
            self.run(arguments, directory).map(drop)
        })();
        match switched {
            // The changes stay in the saved stash, listed in the sidebar, for the user to apply
            // on whichever branch they belong to.
            Ok(()) => Ok(true),
            Err(error) => {
                let restored =
                    self.run(&["stash", "apply", "--index", &stash], directory).and_then(|_| self.drop_stash_by_hash(&stash, directory));
                if let Err(restore_error) = restored {
                    return Err(GitError::failed(
                        "switch branch",
                        format!(
                            "Switch failed: {error}\nYour changes are saved in stash {}. Automatic restoration also failed: {restore_error}",
                            &stash[..stash.len().min(12)]
                        ),
                    ));
                }
                Err(error)
            }
        }
    }

    /// Creates a branch at HEAD and checks it out.
    pub fn create_branch(&self, name: &str, expected_branch: &str, expected_head: Option<&str>, directory: &Path) -> Result<()> {
        let name = name.trim();
        if name.is_empty() {
            return Err(GitError::EmptyBranchName);
        }
        let state = self.require_checkout(expected_branch, expected_head, "create branch", directory)?;
        if state.operation.is_some() {
            return Err(GitError::failed(
                "create branch",
                "Finish or abort the current Git operation before creating and checking out a branch.",
            ));
        }
        self.run(&["check-ref-format", "--branch", name], directory)?;
        self.run(&["switch", "--create", name], directory).map(drop)
    }

    /// Creates a branch at a selected branch's displayed tip, without checking it out.
    pub fn create_branch_from(&self, name: &str, source: &Branch, directory: &Path) -> Result<()> {
        let name = name.trim();
        if name.is_empty() {
            return Err(GitError::EmptyBranchName);
        }
        self.run(&["check-ref-format", "--branch", name], directory)?;
        let reference = if source.is_remote { format!("refs/{}", source.name) } else { format!("refs/heads/{}", source.name) };
        let current = self.run_trimmed(&["rev-parse", "--verify", "--end-of-options", &reference], directory)?;
        if current != source.tip {
            return Err(GitError::failed("branch", BRANCH_CHANGED));
        }
        self.run(&["branch", "--no-track", "--", name, &source.tip], directory).map(drop)
    }

    pub fn rename_branch(&self, branch: &Branch, name: &str, directory: &Path) -> Result<()> {
        let name = name.trim();
        self.run(&["check-ref-format", "--branch", name], directory)?;
        self.require_branch_tip(&branch.name, &branch.tip, directory)?;
        self.run(&["branch", "--move", "--", &branch.name, name], directory).map(drop)
    }

    /// Deletes a local branch. Unmerged branches are deleted only with `force`, and then
    /// atomically against the tip that was shown, so a moved branch survives.
    pub fn delete_branch(&self, branch: &Branch, force: bool, directory: &Path) -> Result<()> {
        self.require_branch_tip(&branch.name, &branch.tip, directory)?;
        if force {
            self.run(&["update-ref", "-d", &format!("refs/heads/{}", branch.name), &branch.tip], directory)?;
            let _ = self.run(&["config", "--remove-section", &format!("branch.{}", branch.name)], directory);
            Ok(())
        } else {
            self.run(&["branch", "--delete", "--", &branch.name], directory).map(drop)
        }
    }

    /// Starts a merge of the selected branch, after confirming both checkouts are as displayed.
    pub fn merge(&self, source: &Branch, expected_branch: &str, expected_head: Option<&str>, directory: &Path) -> Result<()> {
        let state = self.require_checkout(expected_branch, expected_head, "merge", directory)?;
        if state.operation.is_some() || !self.load_status(directory)?.is_empty() {
            return Err(GitError::failed("merge", "Commit or stash changes and finish the current operation first."));
        }
        let reference = if source.is_remote { format!("refs/{}", source.name) } else { format!("refs/heads/{}", source.name) };
        let current = self.run_trimmed(&["rev-parse", "--verify", "--end-of-options", &reference], directory)?;
        if current != source.tip {
            return Err(GitError::failed(
                "merge",
                "The selected branch changed since this action was selected. Refresh and review it again.",
            ));
        }
        if state.head_hash.is_some() {
            self.require_no_ignored_merge_collisions(&current, directory)?;
        }
        self.run(&["merge", "--no-overwrite-ignore", "--no-edit", &current], directory).map(drop)
    }

    /// `--no-overwrite-ignore` does not protect divergent merges with the ort strategy, so check
    /// incoming paths against ignored local files first.
    pub(crate) fn require_no_ignored_merge_collisions(&self, target: &str, directory: &Path) -> Result<()> {
        let bases = self.run_accepting(&["merge-base", "--all", "HEAD", target], directory, &[0, 1])?;
        let mut candidates: Vec<String> = Vec::new();
        for base in bases.split_whitespace() {
            let paths = self.run(&["diff", "--name-only", "--no-renames", "--diff-filter=ACMRT", "-z", base, target, "--"], directory)?;
            for path in paths.split('\0').filter(|path| !path.is_empty()) {
                let components: Vec<&str> = path.split('/').collect();
                let mut prefix = PathBuf::new();
                let mut relative = String::new();
                for (index, component) in components.iter().enumerate() {
                    prefix.push(component);
                    if !relative.is_empty() {
                        relative.push('/');
                    }
                    relative.push_str(component);
                    let Ok(metadata) = std::fs::symlink_metadata(directory.join(&prefix)) else { break };
                    if !metadata.is_dir() || index == components.len() - 1 {
                        candidates.push(relative.clone());
                        break;
                    }
                }
            }
        }
        if candidates.is_empty() {
            return Ok(());
        }
        candidates.sort();
        candidates.dedup();
        let mut arguments = vec!["ls-files", "--others", "--ignored", "--exclude-standard", "-z", "--"];
        arguments.extend(candidates.iter().map(String::as_str));
        if !self.run(&arguments, directory)?.is_empty() {
            return Err(GitError::failed(
                "merge",
                "Ignored local files would be overwritten by this merge. Move or back them up before merging.",
            ));
        }
        Ok(())
    }

    pub fn continue_operation(&self, operation: Operation, directory: &Path) -> Result<()> {
        // Git refuses to continue a rebase while any tracked file has unstaged changes, but it
        // reports that on stdout with a misleading message about conflicts, which the app does
        // not read. Check first, so the refusal says what to do.
        if operation == Operation::Rebase
            && self
                .load_status(directory)?
                .iter()
                .any(|entry| !matches!(entry.kind, StatusKind::Untracked | StatusKind::Conflicted) && entry.is_unstaged())
        {
            return Err(GitError::failed("rebase", "cannot rebase: You have unstaged changes. Stage or discard them, then continue."));
        }
        self.run(&[operation.name(), "--continue"], directory).map(drop)
    }

    pub fn abort_operation(&self, operation: Operation, directory: &Path) -> Result<()> {
        self.run(&[operation.name(), "--abort"], directory).map(drop)
    }

    // MARK: Remotes

    pub fn fetch(&self, directory: &Path) -> Result<()> {
        self.run(&["fetch", "--all", "--prune"], directory).map(drop)
    }

    pub(crate) fn require_upstream(&self, expected: &str, command: &str, directory: &Path) -> Result<()> {
        let current = self.try_trimmed(&["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{upstream}"], directory);
        if current.as_deref() != Some(expected) {
            return Err(GitError::failed(
                command,
                "The upstream branch changed since this action was selected. Refresh and review it again.",
            ));
        }
        Ok(())
    }

    pub(crate) fn require_remote_addresses(
        &self,
        expected: &BTreeMap<String, Vec<String>>,
        remote: &str,
        push: bool,
        command: &str,
        directory: &Path,
    ) -> Result<()> {
        let mut arguments = vec!["remote", "get-url"];
        if push {
            arguments.push("--push");
        }
        arguments.extend(["--all", "--", remote]);
        let current: Option<Vec<String>> = self.run(&arguments, directory).ok().map(|output| output.lines().map(str::to_string).collect());
        match (expected.get(remote), current) {
            (Some(displayed), Some(current)) if !displayed.is_empty() && *displayed == current => Ok(()),
            _ => Err(GitError::failed(command, "The remote address changed since this action was selected. Refresh and review it again.")),
        }
    }

    pub(crate) fn config(&self, key: &str, boolean: bool, directory: &Path) -> Result<Option<String>> {
        let mut arguments = vec!["config"];
        if boolean {
            arguments.push("--bool");
        }
        arguments.extend(["--get", key]);
        let value = self.run_accepting(&arguments, directory, &[0, 1])?;
        Ok((!value.is_empty()).then(|| value.trim().to_string()))
    }

    /// Whether `git pull` would auto-stash: `pull.autoStash` overrides the merge or rebase
    /// default, and a branch's rebase setting overrides `pull.rebase`.
    pub(crate) fn pull_auto_stash(&self, directory: &Path) -> Result<bool> {
        if let Some(value) = self.config("pull.autostash", true, directory)? {
            return Ok(value == "true");
        }
        let branch = self.try_trimmed(&["symbolic-ref", "--quiet", "--short", "HEAD"], directory);
        let branch_key = branch.map(|branch| format!("branch.{branch}.rebase"));
        let key = match branch_key {
            Some(key) if self.config(&key, false, directory)?.is_some() => key,
            _ => "pull.rebase".to_string(),
        };
        let rebase = self.config(&key, false, directory)?;
        let rebasing = ["merges", "m", "interactive", "i"].contains(&rebase.as_deref().unwrap_or(""))
            || self.config(&key, true, directory)?.as_deref() == Some("true");
        let setting = if rebasing { "rebase.autostash" } else { "merge.autostash" };
        Ok(self.config(setting, true, directory)?.as_deref() == Some("true"))
    }

    /// Fetches, then fast-forwards the current branch. `git pull` cannot protect ignored local
    /// files, so this fetches and merges with `--no-overwrite-ignore` itself, rechecking the
    /// checkout after the network step.
    pub fn pull(&self, snapshot: &Snapshot, directory: &Path) -> Result<()> {
        let expected_head = snapshot.head_hash.as_deref();
        let state = self.require_checkout(&snapshot.current_branch, expected_head, "pull", directory)?;
        if state.operation.is_some() {
            return Err(GitError::failed("pull", "Finish or abort the current Git operation before pulling."));
        }
        let expected_upstream =
            snapshot.upstream.as_deref().ok_or_else(|| GitError::failed("pull", "Set an upstream branch before pulling."))?;
        self.require_upstream(expected_upstream, "pull", directory)?;
        let remote = self.config(&format!("branch.{}.remote", snapshot.current_branch), false, directory)?.unwrap_or_default();
        if remote != "." {
            self.require_remote_addresses(&snapshot.remote_fetch_addresses, &remote, false, "pull", directory)?;
        }
        let recurse = self.config("submodule.recurse", true, directory)?.as_deref() == Some("true");
        let auto_stash = self.pull_auto_stash(directory)?;
        self.run(&["fetch"], directory)?;
        // Another client may switch checkouts during network activity.
        self.require_checkout(&snapshot.current_branch, expected_head, "pull", directory)?;
        let stash_flag = if auto_stash { "--autostash" } else { "--no-autostash" };
        self.run(&["merge", "--ff-only", "--no-overwrite-ignore", stash_flag, "FETCH_HEAD"], directory)?;
        if recurse {
            self.run(&["submodule", "update", "--recursive", "--checkout"], directory)?;
        }
        Ok(())
    }

    /// Pushes only the current branch to its upstream, with an explicit refspec and without
    /// mirroring or following tags, whatever the user's push settings.
    pub fn push(&self, snapshot: &Snapshot, directory: &Path) -> Result<()> {
        let (remote, upstream, head) = self.push_target(snapshot, "push", directory)?;
        let mirror = format!("remote.{remote}.mirror=false");
        let refspec = format!("{head}:{upstream}");
        self.run(&["-c", &mirror, "push", "--no-follow-tags", "--recurse-submodules=no", "--", &remote, &refspec], directory).map(drop)
    }

    /// The checks every push runs before it sends anything: the checkout is still the one
    /// `snapshot` describes, the branch has a remote branch as its upstream, and the upstream and
    /// the remote's push addresses are as displayed. Returns the remote, the upstream's full ref,
    /// and HEAD.
    fn push_target(&self, snapshot: &Snapshot, command: &str, directory: &Path) -> Result<(String, String, String)> {
        let branch = self
            .run_trimmed(&["symbolic-ref", "--quiet", "--short", "HEAD"], directory)
            .map_err(|_| GitError::failed(command, "Check out a branch before pushing."))?;
        let head = self.run_trimmed(&["rev-parse", "--verify", "HEAD"], directory)?;
        if branch != snapshot.current_branch || Some(head.as_str()) != snapshot.head_hash.as_deref() {
            return Err(GitError::failed(command, "The current branch changed since it was selected. Refresh and review the push again."));
        }
        let remote = self.config(&format!("branch.{branch}.remote"), false, directory)?.unwrap_or_default();
        let upstream = self.config(&format!("branch.{branch}.merge"), false, directory)?.unwrap_or_default();
        if remote.is_empty() || remote == "." || !upstream.starts_with("refs/heads/") {
            return Err(GitError::failed(command, "Set a remote branch as the upstream before pushing."));
        }
        if let Some(expected) = snapshot.upstream.as_deref() {
            self.require_upstream(expected, command, directory)?;
        }
        self.require_remote_addresses(&snapshot.remote_push_addresses, &remote, true, command, directory)?;
        Ok((remote, upstream, head))
    }

    /// Replaces the upstream branch with the current branch, as a forced push, but only while the
    /// remote branch still points to `expected_remote_tip` (the remote-tracking ref when the user
    /// confirmed). Every check of `push` applies; nothing else is pushed.
    pub fn force_push(&self, snapshot: &Snapshot, expected_remote_tip: &str, directory: &Path) -> Result<()> {
        const COMMAND: &str = "force push";
        if !is_object_id(expected_remote_tip) {
            return Err(GitError::failed(
                COMMAND,
                "The remote branch to replace is not identified by a valid object ID. Review the force push again.",
            ));
        }
        if snapshot.upstream.is_none() {
            return Err(GitError::failed(COMMAND, "Set an upstream branch before force pushing."));
        }
        let (remote, upstream, head) = self.push_target(snapshot, COMMAND, directory)?;
        let mirror = format!("remote.{remote}.mirror=false");
        let lease = format!("--force-with-lease={upstream}:{expected_remote_tip}");
        let refspec = format!("{head}:{upstream}");
        self.run(&["-c", &mirror, "push", "--no-follow-tags", "--recurse-submodules=no", &lease, "--", &remote, &refspec], directory)
            .map(drop)
    }

    /// What a force push would do: the remote-tracking branch's tip, which the lease expects on
    /// the remote, the commits on that tip the branch lacks, and the commits the branch would add.
    pub fn force_push_preview(&self, snapshot: &Snapshot, directory: &Path) -> Result<ForcePushPreview> {
        const COMMAND: &str = "force push";
        self.require_checkout(&snapshot.current_branch, snapshot.head_hash.as_deref(), COMMAND, directory)?;
        let upstream =
            snapshot.upstream.as_deref().ok_or_else(|| GitError::failed(COMMAND, "Set an upstream branch before force pushing."))?;
        let head = snapshot.head_hash.as_deref().ok_or_else(|| GitError::failed(COMMAND, "Make a commit before force pushing."))?;
        let reference = format!("refs/remotes/{upstream}");
        let remote_tip = self
            .try_trimmed(&["rev-parse", "--verify", "--quiet", "--end-of-options", &reference], directory)
            .filter(|tip| is_object_id(tip))
            .ok_or_else(|| GitError::failed(COMMAND, "This branch has no remote-tracking branch to compare with. Fetch first."))?;
        self.require_upstream(upstream, COMMAND, directory)?;
        let log_format = format!("--pretty=format:{LOG_FORMAT}");
        let removed =
            parse_log(&self.run(&["log", "--topo-order", "--max-count=50", &log_format, &remote_tip, "--not", head, "--"], directory)?);
        let removed_count = self.count_commits(&format!("{head}..{remote_tip}"), directory)?;
        let added_count = self.count_commits(&format!("{remote_tip}..{head}"), directory)?;
        Ok(ForcePushPreview { remote_tip, removed, removed_count, added_count })
    }

    /// The number of commits in `range`, a `A..B` revision range between resolved object IDs.
    fn count_commits(&self, range: &str, directory: &Path) -> Result<usize> {
        self.run_trimmed(&["rev-list", "--count", range, "--"], directory)?
            .parse()
            .map_err(|_| GitError::failed("force push", "Git returned a commit count in an unexpected format."))
    }

    /// Each local branch's distance from its upstream, for showing beside branch names. Read
    /// separately from the snapshot, as counting can take a while with many branches.
    pub fn branch_tracking(&self, directory: &Path) -> Result<std::collections::HashMap<String, crate::parsers::Tracking>> {
        let output = self.run(&["for-each-ref", "--format=%(refname)%09%(upstream:track,nobracket)", "refs/heads/"], directory)?;
        Ok(crate::parsers::parse_tracking(&output))
    }

    /// Commits the staged changes with `message`, then pushes that commit to the upstream, as
    /// Push would. Refuses before committing unless the branch and HEAD are still those in
    /// `snapshot` and the branch has an upstream; pushes only the commit just made, on top of
    /// that HEAD. If the push fails, the commit stays and the error says so.
    pub fn commit_and_push(&self, message: &str, snapshot: &Snapshot, directory: &Path) -> Result<()> {
        const COMMAND: &str = "commit and push";
        let branch = self.run_trimmed(&["symbolic-ref", "--quiet", "--short", "HEAD"], directory).unwrap_or_default();
        let head = self.try_trimmed(&["rev-parse", "--verify", "--quiet", "HEAD"], directory);
        if branch.is_empty() || branch != snapshot.current_branch || head != snapshot.head_hash {
            return Err(GitError::failed(COMMAND, "The current branch changed since it was shown. Refresh and try again."));
        }
        if snapshot.upstream.is_none() {
            return Err(GitError::failed(COMMAND, "Publish the branch before pushing commits to it."));
        }
        self.commit(message, directory)?;
        let committed = self.run_trimmed(&["rev-parse", "--verify", "HEAD"], directory)?;
        let parent = self.try_trimmed(&["rev-parse", "--verify", "--quiet", "HEAD^"], directory);
        let still_here = self.run_trimmed(&["symbolic-ref", "--quiet", "--short", "HEAD"], directory).ok() == Some(branch);
        if parent != snapshot.head_hash || !still_here {
            return Err(GitError::failed(COMMAND, "The commit was made, but the branch moved meanwhile, so it was not pushed."));
        }
        let mut committed_snapshot = snapshot.clone();
        committed_snapshot.head_hash = Some(committed);
        self.push(&committed_snapshot, directory)
            .map_err(|error| GitError::failed(COMMAND, format!("The commit was made, but pushing it failed: {error}")))
    }

    /// Pushes the current branch to `remote` under the same name and sets it as the upstream.
    pub fn publish(&self, remote: &str, snapshot: &Snapshot, directory: &Path) -> Result<()> {
        let branch = self
            .run_trimmed(&["symbolic-ref", "--quiet", "--short", "HEAD"], directory)
            .map_err(|_| GitError::failed("publish", "Check out a branch before publishing."))?;
        let head = self.run_trimmed(&["rev-parse", "--verify", "HEAD"], directory)?;
        if branch != snapshot.current_branch || Some(head.as_str()) != snapshot.head_hash.as_deref() {
            return Err(GitError::failed(
                "publish",
                "The current branch changed since it was selected. Refresh and review the publish again.",
            ));
        }
        self.require_remote_addresses(&snapshot.remote_push_addresses, remote, true, "publish", directory)?;
        let reference = format!("refs/heads/{branch}");
        let mirror = format!("remote.{remote}.mirror=false");
        let refspec = format!("{reference}:{reference}");
        self.run(
            &["-c", &mirror, "push", "--set-upstream", "--no-follow-tags", "--recurse-submodules=no", "--", remote, &refspec],
            directory,
        )
        .map(drop)
    }

    /// Pushes a local branch to the same-named branch on `remote`, with an explicit refspec,
    /// after confirming its tip and the remote's push addresses are as displayed.
    pub fn push_branch(
        &self,
        branch: &Branch,
        remote: &str,
        expected_push_addresses: &BTreeMap<String, Vec<String>>,
        directory: &Path,
    ) -> Result<()> {
        self.require_branch_tip(&branch.name, &branch.tip, directory)?;
        self.require_remote_addresses(expected_push_addresses, remote, true, "push", directory)?;
        let reference = format!("refs/heads/{}", branch.name);
        let mirror = format!("remote.{remote}.mirror=false");
        let refspec = format!("{reference}:{reference}");
        self.run(&["-c", &mirror, "push", "--no-follow-tags", "--recurse-submodules=no", "--", remote, &refspec], directory).map(drop)
    }

    /// Rewrites only the HEAD commit's message; staged and unstaged edits are left out.
    pub fn amend_message(&self, message: &str, expected_branch: &str, expected_head: &str, directory: &Path) -> Result<()> {
        if message.trim().is_empty() {
            return Err(GitError::EmptyCommitMessage);
        }
        let state = self.require_checkout(expected_branch, Some(expected_head), "edit message", directory)?;
        if state.operation.is_some() {
            return Err(GitError::failed("edit message", "Finish or abort the current Git operation first."));
        }
        self.run(&["commit", "--quiet", "--amend", "--only", "--message", message], directory).map(drop)
    }

    pub fn add_remote(&self, name: &str, address: &str, directory: &Path) -> Result<()> {
        self.run(&["remote", "add", "--", name.trim(), address.trim()], directory).map(drop)
    }

    // MARK: Stashes

    pub fn list_stashes(&self, directory: &Path) -> Result<Vec<Stash>> {
        Ok(parse_stashes(&self.run(&["stash", "list", &format!("--format={STASH_FORMAT}")], directory)?))
    }

    /// Saves the working changes that `options` take, and confirms that Git left only what they keep.
    pub fn save_stash(&self, message: &str, options: StashOptions, directory: &Path) -> Result<()> {
        self.require_finished_operation("stash", directory)?;
        let before = self.load_status(directory)?;
        let previous = self.try_trimmed(&["rev-parse", "--verify", "--quiet", "refs/stash"], directory);
        let message =
            if message.trim().is_empty() { format!("WIP on {}", self.current_branch(directory)) } else { message.trim().to_string() };
        let mut arguments = vec!["stash", "push"];
        if options.include_untracked {
            arguments.push("--include-untracked");
        }
        if options.keep_index {
            arguments.push("--keep-index");
        }
        arguments.extend(["-m", &message]);
        self.run(&arguments, directory)?;
        let saved = self.try_trimmed(&["rev-parse", "--verify", "--quiet", "refs/stash"], directory);
        let Some(saved) = saved.filter(|saved| Some(saved) != previous.as_ref()) else {
            return Err(GitError::failed("stash", "Git did not save any changes. Check any dirty submodules."));
        };
        if !stash_left_expected(&before, &self.load_status(directory)?, options) {
            return Err(GitError::failed(
                "stash",
                format!(
                    "Some changes were saved in stash {}, but changes remain in the working tree. Check submodules before proceeding.",
                    &saved[..12.min(saved.len())]
                ),
            ));
        }
        Ok(())
    }

    pub(crate) fn require_listed_stash(&self, stash: &Stash, command: &str, directory: &Path) -> Result<Stash> {
        self.list_stashes(directory)?
            .into_iter()
            .find(|listed| listed.hash == stash.hash)
            .ok_or_else(|| GitError::failed(command, "This stash no longer exists. Refresh the repository."))
    }

    pub fn apply_stash(&self, stash: &Stash, directory: &Path) -> Result<()> {
        self.require_finished_operation("stash apply", directory)?;
        self.require_listed_stash(stash, "stash apply", directory)?;
        self.run(&["stash", "apply", "--index", &stash.hash], directory).map(drop)
    }

    pub fn pop_stash(&self, stash: &Stash, directory: &Path) -> Result<()> {
        self.apply_stash(stash, directory)?;
        self.drop_stash(stash, directory).map_err(|error| {
            GitError::failed("stash pop", format!("The stash was applied, but could not be removed. Do not apply it again. {error}"))
        })
    }

    /// Drops a stash by its object ID; positions can change outside the app.
    pub fn drop_stash(&self, stash: &Stash, directory: &Path) -> Result<()> {
        let current = self.require_listed_stash(stash, "stash drop", directory)?;
        self.run(&["stash", "drop", &current.reference], directory).map(drop)
    }

    pub(crate) fn drop_stash_by_hash(&self, hash: &str, directory: &Path) -> Result<()> {
        if let Some(saved) = self.list_stashes(directory)?.into_iter().find(|stash| stash.hash == hash) {
            self.run(&["stash", "drop", &saved.reference], directory).map_err(|error| {
                GitError::failed(
                    "switch branch",
                    format!(
                        "Changes were restored, but stash {} could not be removed. Do not apply it again. {error}",
                        &hash[..12.min(hash.len())]
                    ),
                )
            })?;
        }
        Ok(())
    }

    pub fn stash_diff(&self, stash: &Stash, directory: &Path) -> Result<String> {
        self.stash_diff_with(stash, DiffRequest::default(), directory)
    }

    /// A stash's changes, optionally leaving out whitespace-only changes and showing the requested context.
    pub fn stash_diff_with(&self, stash: &Stash, request: DiffRequest, directory: &Path) -> Result<String> {
        let context = request.context.argument();
        let mut args = vec!["stash", "show", "--include-untracked", "--patch", "--no-ext-diff", "--no-color"];
        if request.ignore_whitespace {
            args.push("--ignore-all-space");
        }
        args.extend(context.as_deref());
        args.push(&stash.hash);
        self.run(&args, directory)
    }

    /// The files a stash saved, sorted by path: its tracked changes against the commit it was
    /// made on, then its untracked files from the third parent. Renames are not detected, as
    /// in a commit's file list, so each path is listed once.
    pub fn stash_files(&self, stash: &Stash, directory: &Path) -> Result<Vec<StashFile>> {
        let commit = self.resolve_commit(&stash.hash, directory)?;
        let parent = self.resolve_commit(&format!("{commit}^1"), directory)?;
        let tracked = self.run(&["diff", "--name-status", "-z", "--no-renames", &parent, &commit, "--"], directory)?;
        let mut files: Vec<StashFile> =
            parse_name_status(&tracked).into_iter().map(|(status, path)| StashFile { status, path, untracked: false }).collect();
        if let Some(untracked) = self.stash_untracked_commit(&commit, directory) {
            let listing = self.run(&["ls-tree", "-r", "-z", "--full-tree", "--name-only", &untracked], directory)?;
            files.extend(listing.split('\0').filter(|path| !path.is_empty()).map(|path| StashFile {
                status: "A".to_string(),
                path: path.to_string(),
                untracked: true,
            }));
        }
        files.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(files)
    }

    /// The commit holding a stash's untracked files, when the stash saved any.
    fn stash_untracked_commit(&self, commit: &str, directory: &Path) -> Option<String> {
        self.try_trimmed(&["rev-parse", "--verify", "--quiet", &format!("{commit}^3")], directory)
    }

    /// The patch for one file a stash saved. A tracked file is compared with the commit the
    /// stash was made on; an untracked file appears entirely as added lines.
    pub fn stash_file_diff(&self, stash: &Stash, file: &StashFile, request: DiffRequest, directory: &Path) -> Result<String> {
        let context = request.context.argument();
        let commit = self.resolve_commit(&stash.hash, directory)?;
        let (before, after) = if file.untracked {
            let untracked = self
                .stash_untracked_commit(&commit, directory)
                .ok_or_else(|| GitError::failed("stash diff", "This stash has no untracked files. Refresh the repository."))?;
            (self.empty_tree(directory)?, untracked)
        } else {
            (self.resolve_commit(&format!("{commit}^1"), directory)?, commit)
        };
        let mut arguments = vec!["diff", "--no-ext-diff", "--no-color"];
        if request.ignore_whitespace {
            arguments.push("--ignore-all-space");
        }
        arguments.extend(context.as_deref());
        // The runner passes literal pathspecs to `diff`, so a glob-like name matches only itself.
        arguments.extend([before.as_str(), after.as_str(), "--", file.path.as_str()]);
        self.run(&arguments, directory)
    }

    /// The object ID of the empty tree in this repository's format. Git writes it from empty input.
    fn empty_tree(&self, directory: &Path) -> Result<String> {
        self.run_trimmed(&["mktree"], directory)
    }

    // MARK: Patches

    /// Applies a patch file to the working files, checking it applies cleanly first. Both
    /// steps read the same captured bytes, not a file that could change in between.
    pub fn apply_patch(&self, contents: &[u8], directory: &Path) -> Result<()> {
        self.require_finished_operation("apply", directory)?;
        let file = std::env::temp_dir().join(format!(
            "nicegit-{}-{}.patch",
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0)
        ));
        std::fs::write(&file, contents).map_err(|e| GitError::failed("apply", e.to_string()))?;
        let path = file.to_string_lossy().into_owned();
        let result = self.run(&["apply", "--check", "--", &path], directory).and_then(|_| self.run(&["apply", "--", &path], directory));
        let _ = std::fs::remove_file(&file);
        result.map(drop)
    }

    /// One commit as an email-style patch, as `git format-patch` writes it.
    pub fn export_commit_patch(&self, hash: &str, directory: &Path) -> Result<String> {
        let resolved = self.resolve_commit(hash, directory)?;
        let parents = self.run(&["rev-list", "--parents", "-n", "1", &resolved], directory)?;
        if parents.split_whitespace().count() > 2 {
            return Err(GitError::failed(
                "format-patch",
                "Merge commits cannot be exported as a single patch. Select an individual commit.",
            ));
        }
        let files = self.run(&["diff-tree", "--root", "--no-commit-id", "--name-only", "-r", &resolved, "--"], directory)?;
        if files.is_empty() {
            return Err(GitError::failed("format-patch", "This commit has no file changes to export."));
        }
        let patch = self.run(
            &[
                "format-patch",
                "--stdout",
                "--root",
                "--no-cover-letter",
                "--no-signature",
                "--no-thread",
                "--no-attach",
                "--binary",
                "--full-index",
                "--no-ext-diff",
                "--no-textconv",
                "-1",
                &resolved,
                "--",
            ],
            directory,
        )?;
        if patch.is_empty() {
            return Err(GitError::failed("format-patch", "This commit has no exportable patch."));
        }
        Ok(patch)
    }

    /// The staged changes (`staged`) or the unstaged changes to tracked files as a patch that
    /// `git apply` accepts, with binary changes, for saving uncommitted work. `paths`, when given,
    /// limits it to those files; a renamed file's source path must be listed with its new path.
    /// Untracked files are included as new files only for unstaged changes and `include_untracked`.
    /// Folders, including nested repositories and links to folders, have no file contents to save
    /// and are skipped. Conflicted files are refused, since their diffs cannot be applied.
    pub fn working_changes_patch(
        &self,
        staged: bool,
        paths: Option<&[String]>,
        include_untracked: bool,
        directory: &Path,
    ) -> Result<String> {
        const NOTHING: &str = "There are no changes to save.";
        // An empty list would otherwise mean every file, so it is refused rather than run.
        if paths.is_some_and(<[String]>::is_empty) {
            return Err(GitError::failed("save patch", NOTHING));
        }
        let status = self.load_status(directory)?;
        let selected: Vec<&StatusEntry> = status.iter().filter(|entry| paths.is_none_or(|paths| paths.contains(&entry.path))).collect();
        if selected.iter().any(|entry| entry.kind == StatusKind::Conflicted) {
            return Err(GitError::failed("save patch", "Resolve conflicts before saving changes as a patch."));
        }
        // The prefixes are explicit, so `diff.noprefix` cannot produce a patch that `git apply` rejects.
        let mut arguments = vec![
            "diff",
            "--binary",
            "--full-index",
            "--no-ext-diff",
            "--no-textconv",
            "--no-color",
            "--no-renames",
            "--submodule=short",
            "--src-prefix=a/",
            "--dst-prefix=b/",
        ];
        if staged {
            arguments.push("--cached");
        }
        if let Some(paths) = paths {
            arguments.push("--");
            arguments.extend(paths.iter().map(String::as_str));
        }
        let mut patch = self.run(&arguments, directory)?;
        if include_untracked && !staged {
            for entry in selected.iter().filter(|entry| entry.kind == StatusKind::Untracked) {
                // `is_dir` follows links, so a link to a folder is skipped as well.
                if directory.join(&entry.path).is_dir() {
                    continue;
                }
                // Git exits with 1 when the files differ, which is the normal case here.
                patch.push_str(&self.run_accepting(
                    &[
                        "diff",
                        "--binary",
                        "--full-index",
                        "--no-ext-diff",
                        "--no-textconv",
                        "--no-color",
                        "--src-prefix=a/",
                        "--dst-prefix=b/",
                        "--no-index",
                        "--",
                        "/dev/null",
                        &entry.path,
                    ],
                    directory,
                    &[0, 1],
                )?);
            }
        }
        if patch.is_empty() {
            return Err(GitError::failed("save patch", NOTHING));
        }
        Ok(patch)
    }

    /// Writes the files of `commit` to `destination` as a ZIP archive, inside a top folder named
    /// `prefix`, as `git archive` makes it. Nothing in the repository changes.
    ///
    /// Git's `export-ignore` and `export-subst` attributes apply, as they do for GitHub's
    /// "Download ZIP". Submodule contents are not included; Git archives only the commit's own
    /// files. An existing file at `destination` is replaced, so the caller must have confirmed
    /// that. The archive is written beside `destination` first and renamed over it only when
    /// complete, so a failed export never leaves a truncated ZIP in place of a good one.
    pub fn archive_commit(&self, commit: &str, prefix: &str, destination: &Path, directory: &Path) -> Result<()> {
        validate_archive_prefix(prefix)?;
        // Git receives the paths as text, so a destination that is not valid UTF-8 cannot be used.
        let destination = std::path::absolute(destination).map_err(|error| GitError::failed("archive", error.to_string()))?;
        if destination.to_str().is_none() {
            return Err(GitError::failed("archive", "Choose a save location whose name is valid UTF-8 text."));
        }
        let resolved = self.resolve_commit(commit, directory)?;
        let mut partial_name = destination.as_os_str().to_owned();
        partial_name.push(format!(".partial-{}", std::process::id()));
        let partial = PathBuf::from(partial_name);
        let output = format!("--output={}", partial.to_string_lossy());
        let prefix_argument = format!("--prefix={prefix}/");
        let written = self
            .run(&["archive", "--format=zip", &prefix_argument, &output, &resolved], directory)
            .and_then(|_| std::fs::rename(&partial, &destination).map_err(|error| GitError::failed("archive", error.to_string())));
        if written.is_err() {
            let _ = std::fs::remove_file(&partial);
        }
        written
    }

    // MARK: Tags

    pub fn create_tag(&self, name: &str, target: &str, message: Option<&str>, directory: &Path) -> Result<()> {
        let name = name.trim();
        self.run(&["check-ref-format", &format!("refs/tags/{name}")], directory)?;
        let commit = self.resolve_commit(target, directory)?;
        match message.map(str::trim).filter(|message| !message.is_empty()) {
            Some(message) => {
                self.run(&["-c", "tag.gpgSign=false", "tag", "--annotate", "--message", message, "--", name, &commit], directory)
            }
            None => self.run(&["-c", "tag.gpgSign=false", "tag", "--", name, &commit], directory),
        }
        .map(drop)
    }

    /// The object a local tag points to: the commit for a lightweight tag, or the tag object for
    /// an annotated one. The name must already have passed `create_tag`'s check.
    pub fn tag_tip(&self, name: &str, directory: &Path) -> Result<String> {
        self.run_trimmed(&["rev-parse", "--verify", "--end-of-options", &format!("refs/tags/{name}")], directory)
    }

    /// The tagger, date, message, and target of an annotated tag, or `None` for a lightweight tag,
    /// which has no tag object. The name must already have passed `create_tag`'s check.
    pub fn tag_details(&self, name: &str, directory: &Path) -> Result<Option<TagDetails>> {
        let reference = format!("refs/tags/{name}");
        self.run(&["check-ref-format", &reference], directory)?;
        // Git's pattern matches only this tag: no other tag can be named under it, as a tag
        // cannot be both a ref and a directory of refs.
        let output = self.run(&["for-each-ref", &format!("--format={TAG_DETAILS_FORMAT}"), &reference], directory)?;
        parse_tag_details(&output)
    }

    /// Deletes a tag only while it still points to the object shown when it was selected.
    pub fn delete_tag(&self, name: &str, expected_tip: &str, directory: &Path) -> Result<()> {
        self.run(&["update-ref", "-d", &format!("refs/tags/{name}"), expected_tip], directory)
            .map(drop)
            .map_err(|_| GitError::failed("delete tag", "This tag changed since it was selected. Refresh and review it again."))
    }

    pub fn identity(&self, directory: &Path) -> (String, String) {
        let name = self.try_trimmed(&["config", "user.name"], directory).unwrap_or_default();
        let email = self.try_trimmed(&["config", "user.email"], directory).unwrap_or_default();
        (name, email)
    }

    /// Paths of branches already checked out in other worktrees, which Git refuses to switch to.
    pub fn branches_in_other_worktrees(snapshot: &Snapshot) -> HashSet<String> {
        snapshot
            .worktrees
            .iter()
            .filter(|worktree| worktree.path != snapshot.root_path)
            .filter_map(|worktree| worktree.branch.clone())
            .collect()
    }
}

/// The unfinished operation recorded in a Git directory, if any.
pub fn operation_in(git_directory: &Path) -> Option<Operation> {
    let markers = [
        ("rebase-merge", Operation::Rebase),
        ("rebase-apply", Operation::Rebase),
        ("MERGE_HEAD", Operation::Merge),
        ("CHERRY_PICK_HEAD", Operation::CherryPick),
        ("REVERT_HEAD", Operation::Revert),
    ];
    for (marker, operation) in markers {
        if git_directory.join(marker).exists() {
            return Some(operation);
        }
    }
    // A manual conflict commit removes the per-commit marker but leaves the sequence's todo.
    let todo = std::fs::read_to_string(git_directory.join("sequencer").join("todo")).ok()?;
    todo.lines().find_map(|line| match line.split_whitespace().next() {
        Some("pick") => Some(Operation::CherryPick),
        Some("revert") => Some(Operation::Revert),
        _ => None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status(path: &str, index: char, work: char) -> StatusEntry {
        let kind = if index == '?' { StatusKind::Untracked } else { StatusKind::Modified };
        StatusEntry { path: path.into(), original_path: None, kind, index_status: index, work_tree_status: work }
    }

    const DEFAULT: StashOptions = StashOptions { keep_index: false, include_untracked: true };
    const KEEP_INDEX: StashOptions = StashOptions { keep_index: true, include_untracked: true };
    const TRACKED_ONLY: StashOptions = StashOptions { keep_index: false, include_untracked: false };

    #[test]
    fn a_clean_tree_after_the_default_stash_is_expected() {
        let before = [status("a.txt", ' ', 'M'), status("new.txt", '?', '?')];
        assert!(stash_left_expected(&before, &[], DEFAULT));
    }

    #[test]
    fn untracked_files_left_without_include_untracked_are_expected_when_they_were_untracked_before() {
        let before = [status("a.txt", ' ', 'M'), status("new.txt", '?', '?')];
        let after = [status("new.txt", '?', '?')];
        assert!(stash_left_expected(&before, &after, TRACKED_ONLY));
        assert!(!stash_left_expected(&before, &after, DEFAULT), "with include_untracked, nothing untracked may remain");
    }

    #[test]
    fn an_untracked_file_that_was_not_there_before_is_unexpected() {
        let before = [status("a.txt", ' ', 'M')];
        let after = [status("appeared.txt", '?', '?')];
        assert!(!stash_left_expected(&before, &after, TRACKED_ONLY));
    }

    #[test]
    fn with_keep_index_a_staged_entry_with_a_clean_working_side_is_expected() {
        let before = [status("a.txt", 'M', ' '), status("b.txt", ' ', 'M')];
        let after = [status("a.txt", 'M', ' ')];
        assert!(stash_left_expected(&before, &after, KEEP_INDEX));
        assert!(!stash_left_expected(&before, &after, DEFAULT), "without keep_index, a staged entry must not remain");
    }

    #[test]
    fn with_keep_index_an_entry_that_still_has_unstaged_changes_is_unexpected() {
        let before = [status("a.txt", 'M', 'M')];
        let after = [status("a.txt", 'M', 'M')];
        assert!(!stash_left_expected(&before, &after, KEEP_INDEX));
    }

    #[test]
    fn with_keep_index_a_staged_side_that_changed_is_unexpected() {
        let before = [status("a.txt", 'M', ' ')];
        let after = [status("a.txt", 'A', ' ')];
        assert!(!stash_left_expected(&before, &after, KEEP_INDEX));
    }
}
