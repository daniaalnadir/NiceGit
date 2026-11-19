//! Splitting a commit in two. The person chooses which changes go into the first new commit,
//! and NiceGit rebuilds the commit as two commits, then replays the commits after it. Everything
//! is done with plumbing commands in a temporary index, so the working tree and the index are
//! never touched.

use std::collections::BTreeSet;
use std::path::Path;

use crate::blame::unquote;
use crate::client::{is_signing_refusal, GitClient};
use crate::diff::{hunk_header, parse_diff, DiffLineKind};
use crate::models::{GitError, Result};
use crate::rebase::Scratch;
use crate::runner::{self, RunOptions};

const COMMAND: &str = "split commit";
const NOTHING_TO_SPLIT: &str = "Choose some changes, but not all of them, for the first commit.";
const NOT_SEPARATED: &str = "The chosen changes could not be separated.";
const BRANCH_MOVED: &str = "The branch moved while the commit was being split. Refresh and review it again.";

/// A split as the app asks for it. `first_patch` holds the changes for the first new commit, as a
/// patch against the commit's parent, as `first_patch` builds it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SplitRequest {
    pub branch: String,
    pub expected_head: String,
    pub commit: String,
    pub first_patch: String,
    pub first_message: String,
    pub second_message: String,
}

/// The commits a split made, and the branch tip it replaced.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SplitOutcome {
    pub first: String,
    pub second: String,
    pub new_head: String,
    pub previous_head: String,
}

/// What one file contributes to the first commit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FileChoice {
    /// Nothing from this file goes into the first commit.
    Nothing,
    /// The whole file goes into the first commit.
    Whole,
    /// Only these hunks, by their index in `SplitFile::hunks`, go into the first commit.
    Hunks(BTreeSet<usize>),
}

/// A file changed by a commit, as the split window lists it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SplitFile {
    pub path: String,
    /// True when the file can only move as a whole: binary files, mode changes, submodules,
    /// symbolic links, renames, and new or deleted files.
    pub whole_only: bool,
    /// The file's patch as Git wrote it against the commit's parent.
    pub patch: String,
    /// The header lines before the first hunk, as Git wrote them.
    pub headers: Vec<String>,
    pub hunks: Vec<SplitHunk>,
}

/// One hunk of a file's patch, with the context Git chose for it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SplitHunk {
    /// Git's `@@` line, as written.
    pub header: String,
    /// The added and removed lines, with their signs, for display.
    pub changes: Vec<String>,
    /// The hunk's lines with their signs, context included, each ending in a newline.
    pub body: String,
    /// The first line of the hunk in the parent. For an insertion, the line after the one it follows.
    pub old_first: usize,
    pub old_count: usize,
    pub new_count: usize,
}

/// Reads a commit's patch (see `GitClient::split_patch`) into one entry per file.
pub fn split_files(patch: &str) -> Vec<SplitFile> {
    let mut pieces: Vec<&str> = patch.split('\n').collect();
    if patch.ends_with('\n') {
        pieces.pop();
    }
    let mut sections: Vec<Vec<&str>> = Vec::new();
    for piece in pieces {
        if sections.is_empty() || piece.starts_with("diff --git ") {
            sections.push(Vec::new());
        }
        sections.last_mut().expect("a section exists after the push above").push(piece);
    }
    sections.iter().map(|lines| split_section(lines)).collect()
}

fn split_section(lines: &[&str]) -> SplitFile {
    let mut section = String::new();
    for line in lines {
        section.push_str(line);
        section.push('\n');
    }
    // `parse_diff` reads the empty text after the last newline as one more line; it is dropped here.
    let parsed = parse_diff(&section);
    let parsed = &parsed[..lines.len()];

    let first_hunk = parsed.iter().position(|line| line.kind == DiffLineKind::Hunk).unwrap_or(lines.len());
    let headers: Vec<String> = lines[..first_hunk].iter().map(|line| line.to_string()).collect();
    let whole_only = headers.iter().any(|text| is_whole_file_header(text));

    let starts: Vec<usize> =
        parsed.iter().enumerate().filter(|(_, line)| line.kind == DiffLineKind::Hunk).map(|(index, _)| index).collect();
    let hunks = starts
        .iter()
        .enumerate()
        .map(|(number, &start)| {
            let end = starts.get(number + 1).copied().unwrap_or(parsed.len());
            let (old_start, old_count, _, new_count) = hunk_header(lines[start]).unwrap_or((0, 0, 0, 0));
            let mut body = String::new();
            let mut changes = Vec::new();
            for line in &parsed[start + 1..end] {
                body.push_str(&line.text);
                body.push('\n');
                if matches!(line.kind, DiffLineKind::Addition | DiffLineKind::Deletion) {
                    changes.push(line.text.clone());
                }
            }
            SplitHunk {
                header: lines[start].to_string(),
                changes,
                body,
                // Git numbers an insertion or deletion with no lines on one side from the line before it.
                old_first: if old_count > 0 { old_start } else { old_start + 1 },
                old_count,
                new_count,
            }
        })
        .collect();

    SplitFile { path: section_path(&headers), whole_only, patch: section, headers, hunks }
}

/// Headers that mean a file can only move as a whole: binary content, renames, mode changes,
/// new and deleted files, symbolic links, and submodules.
fn is_whole_file_header(text: &str) -> bool {
    const PREFIXES: &[&str] = &[
        "new file mode",
        "deleted file mode",
        "old mode",
        "new mode",
        "rename from",
        "rename to",
        "copy from",
        "copy to",
        "similarity index",
        "dissimilarity index",
        "Binary files",
        "GIT binary patch",
    ];
    PREFIXES.iter().any(|prefix| text.starts_with(prefix)) || text.ends_with("120000") || text.ends_with("160000")
}

/// The path a file's section names. The `+++` line names it unless the file was deleted, then the
/// `---` line does. A mode-only change or binary file has neither, so its `diff --git` line is read,
/// where both names are the same.
fn section_path(headers: &[String]) -> String {
    let named = |marker: &str, side: &str| -> Option<String> {
        let value = headers.iter().find_map(|text| text.strip_prefix(marker))?;
        if value == "/dev/null" {
            return None;
        }
        let path = unquote(value);
        Some(path.strip_prefix(side).unwrap_or(&path).to_string())
    };
    named("+++ ", "b/")
        .or_else(|| named("--- ", "a/"))
        .or_else(|| headers.iter().find_map(|text| text.strip_prefix("diff --git ")).and_then(identical_names))
        .unwrap_or_default()
}

/// The path from a `diff --git a/<path> b/<path>` line whose two names are equal.
fn identical_names(rest: &str) -> Option<String> {
    let names = rest.strip_prefix("a/")?;
    names.match_indices(" b/").find_map(|(index, _)| {
        let (before, after) = (&names[..index], &names[index + 3..]);
        (before == after).then(|| before.to_string())
    })
}

/// Builds the patch for the first commit from the choices made for each file. `choices` holds one
/// entry per file, in the order of `files`. The result applies to the commit's parent.
///
/// Hunks keep the context Git gave them, and each one's `@@` numbers are recalculated for the
/// hunks before it that were left out, so the patch applies to the parent's content.
pub fn first_patch(files: &[SplitFile], choices: &[FileChoice]) -> Result<String> {
    if files.len() != choices.len() {
        return Err(GitError::failed(COMMAND, "The choices do not match the commit's files."));
    }
    let mut patch = String::new();
    for (file, choice) in files.iter().zip(choices) {
        match choice {
            FileChoice::Nothing => {}
            FileChoice::Whole => patch.push_str(&file.patch),
            FileChoice::Hunks(selected) => {
                if selected.is_empty() {
                    continue;
                }
                if selected.len() == file.hunks.len() {
                    patch.push_str(&file.patch);
                    continue;
                }
                if file.whole_only {
                    return Err(GitError::failed(COMMAND, format!("{} can only be moved as a whole.", file.path)));
                }
                if selected.iter().any(|&index| index >= file.hunks.len()) {
                    return Err(GitError::failed(COMMAND, "A chosen change does not exist in the commit."));
                }
                for header in file.headers.iter().filter(|header| !header.starts_with("index ")) {
                    patch.push_str(header);
                    patch.push('\n');
                }
                // Hunks before this one that are left out move the line numbers of the result.
                let mut offset: isize = 0;
                for (index, hunk) in file.hunks.iter().enumerate() {
                    if !selected.contains(&index) {
                        continue;
                    }
                    let first = (hunk.old_first as isize + offset) as usize;
                    let old_start = if hunk.old_count > 0 { hunk.old_first } else { hunk.old_first - 1 };
                    let new_start = if hunk.new_count > 0 { first } else { first - 1 };
                    patch.push_str(&format!("@@ -{old_start},{} +{new_start},{} @@\n", hunk.old_count, hunk.new_count));
                    patch.push_str(&hunk.body);
                    offset += hunk.new_count as isize - hunk.old_count as isize;
                }
            }
        }
    }
    Ok(patch)
}

/// The author of a commit, as the commit it is rebuilt into must keep it.
pub(crate) struct Author {
    name: String,
    email: String,
    /// Git's raw date: seconds since the epoch and the offset.
    date: String,
}

impl GitClient {
    /// A commit's patch against its first parent, with paths in the `a/` and `b/` form. `split_files`
    /// reads it, and `first_patch` applies the person's choices to it.
    pub fn split_patch(&self, commit: &str, directory: &Path) -> Result<String> {
        let commit = self.resolve_commit(commit, directory)?;
        self.run(
            &[
                "show",
                "--format=",
                "--patch",
                "--no-renames",
                "--no-ext-diff",
                "--no-textconv",
                "--no-color",
                "--src-prefix=a/",
                "--dst-prefix=b/",
                "--unified=3",
                &commit,
                "--",
            ],
            directory,
        )
    }

    /// Whether a commit is already on a remote branch, so rewriting it means a force-push.
    pub fn commit_is_published(&self, commit: &str, directory: &Path) -> Result<bool> {
        let commit = self.resolve_commit(commit, directory)?;
        Ok(self.run_trimmed(&["rev-list", "-n", "1", &commit, "--not", "--remotes", "--"], directory)?.is_empty())
    }

    /// Splits `request.commit` on the current branch into two commits, the first holding the changes
    /// in `request.first_patch`. The commits after it are replayed onto the second. The branch moves
    /// only if it is still at `request.expected_head`, and nothing in the working tree or index changes.
    pub fn split_commit(&self, request: &SplitRequest, directory: &Path) -> Result<SplitOutcome> {
        let state = self.require_checkout(&request.branch, Some(&request.expected_head), COMMAND, directory)?;
        if state.operation.is_some() {
            return Err(GitError::failed(COMMAND, "Finish or abort the current Git operation first."));
        }
        let first_message = request.first_message.trim();
        let second_message = request.second_message.trim();
        if first_message.is_empty() || second_message.is_empty() {
            return Err(GitError::failed(COMMAND, "Both commits need a message."));
        }
        if request.first_patch.trim().is_empty() {
            return Err(GitError::failed(COMMAND, NOTHING_TO_SPLIT));
        }

        let head = request.expected_head.as_str();
        let commit = self.resolve_commit(&request.commit, directory)?;
        self.run(&["merge-base", "--is-ancestor", &commit, head], directory)
            .map_err(|_| GitError::failed(COMMAND, "This commit is not part of the current branch's history."))?;
        let parents: Vec<String> = self
            .run_trimmed(&["rev-list", "--parents", "-n", "1", &commit, "--"], directory)?
            .split_whitespace()
            .skip(1)
            .map(str::to_string)
            .collect();
        match parents.len() {
            0 => return Err(GitError::failed(COMMAND, "The first commit on a branch can't be split.")),
            1 => {}
            _ => return Err(GitError::failed(COMMAND, "Merge commits can't be split.")),
        }
        let parent = parents[0].as_str();
        let range = format!("{commit}^..{head}");
        if !self.run_trimmed(&["rev-list", "--min-parents=2", &range, "--"], directory)?.is_empty() {
            return Err(GitError::failed(COMMAND, "These commits include a merge. Choose a commit after the latest merge."));
        }
        let later: Vec<String> = self
            .run_trimmed(&["rev-list", "--reverse", &format!("{commit}..{head}"), "--"], directory)?
            .lines()
            .map(str::to_string)
            .collect();

        let tree_of = |revision: &str| self.run_trimmed(&["rev-parse", "--verify", &format!("{revision}^{{tree}}")], directory);
        let original_tree = tree_of(&commit)?;
        let parent_tree = tree_of(parent)?;
        let author = self.author_of(&commit, directory)?;
        let sign = self.signing_setup(directory)?.sign_by_default;

        let scratch = Scratch::new(COMMAND)?;
        let index_file = scratch.path("index").to_string_lossy().into_owned();
        let index_environment = [("GIT_INDEX_FILE", index_file.as_str())];
        let index_options = RunOptions { env: &index_environment, ..self.options() };
        runner::run(&["read-tree", parent], directory, &index_options)?;
        let patch_file = scratch.write(COMMAND, "first.patch", request.first_patch.as_bytes())?;
        let patch_path = patch_file.to_string_lossy().into_owned();
        runner::run(&["apply", "--cached", "--whitespace=nowarn", "--", &patch_path], directory, &index_options)
            .map_err(|_| GitError::failed(COMMAND, NOT_SEPARATED))?;
        let first_tree = runner::run(&["write-tree"], directory, &index_options)?.trim().to_string();
        if first_tree == parent_tree || first_tree == original_tree {
            return Err(GitError::failed(COMMAND, NOTHING_TO_SPLIT));
        }

        let first = self.commit_tree_as(&scratch, 0, &first_tree, &[parent], &format!("{first_message}\n"), &author, sign, directory)?;
        let second =
            self.commit_tree_as(&scratch, 1, &original_tree, &[first.as_str()], &format!("{second_message}\n"), &author, sign, directory)?;
        let mut new_head = second.clone();
        for (number, original) in later.iter().enumerate() {
            let tree = tree_of(original)?;
            let author = self.author_of(original, directory)?;
            let message = self.raw_commit_message(original, directory)?;
            new_head = self.commit_tree_as(&scratch, 2 + number, &tree, &[new_head.as_str()], &message, &author, sign, directory)?;
        }

        let short = &commit[..commit.len().min(7)];
        let reflog = format!("nicegit: split {short}");
        let reference = format!("refs/heads/{}", request.branch);
        self.run(&["update-ref", "-m", &reflog, &reference, &new_head, head], directory)
            .map_err(|_| GitError::failed(COMMAND, BRANCH_MOVED))?;
        Ok(SplitOutcome { first, second, new_head, previous_head: head.to_string() })
    }

    /// A commit's message exactly as stored, after its header block.
    pub(crate) fn raw_commit_message(&self, commit: &str, directory: &Path) -> Result<String> {
        let object = self.run(&["cat-file", "commit", commit], directory)?;
        Ok(object.split_once("\n\n").map(|(_, message)| message.to_string()).unwrap_or_default())
    }

    pub(crate) fn author_of(&self, commit: &str, directory: &Path) -> Result<Author> {
        let text =
            runner::strip_line_terminator(self.run(&["show", "-s", "--format=%an%x00%ae%x00%ad", "--date=raw", commit, "--"], directory)?);
        match text.split('\0').collect::<Vec<_>>().as_slice() {
            [name, email, date] => Ok(Author { name: name.to_string(), email: email.to_string(), date: date.to_string() }),
            _ => Err(GitError::failed(COMMAND, "The commit's author could not be read.")),
        }
    }

    /// Creates a commit with `tree` and `parents`, written by `author` with the message exactly as given.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn commit_tree_as(
        &self,
        scratch: &Scratch,
        number: usize,
        tree: &str,
        parents: &[&str],
        message: &str,
        author: &Author,
        sign: bool,
        directory: &Path,
    ) -> Result<String> {
        let message_file = scratch.write(COMMAND, &format!("message-{number}"), message.as_bytes())?;
        let message_path = message_file.to_string_lossy().into_owned();
        let mut arguments = vec!["commit-tree"];
        if sign {
            arguments.push("-S");
        }
        arguments.push(tree);
        for &parent in parents {
            arguments.extend(["-p", parent]);
        }
        arguments.extend(["-F", message_path.as_str()]);
        let environment = [
            ("GIT_AUTHOR_NAME", author.name.as_str()),
            ("GIT_AUTHOR_EMAIL", author.email.as_str()),
            ("GIT_AUTHOR_DATE", author.date.as_str()),
        ];
        let options = RunOptions { env: &environment, ..self.options() };
        let output = runner::run(&arguments, directory, &options).map_err(|error| match error {
            GitError::CommandFailed { command, message } if is_signing_refusal(&message) => {
                GitError::SigningRejected { command, output: message }
            }
            other => other,
        })?;
        Ok(output.trim().to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    /// A repository on `main` with one commit holding `base.txt`, and the test's identity set.
    fn repository() -> (tempfile::TempDir, GitClient) {
        let dir = tempfile::tempdir().unwrap();
        let client = GitClient::new();
        let root = dir.path();
        client.run(&["init", "--initial-branch=main"], root).unwrap();
        client.run(&["config", "user.name", "Test"], root).unwrap();
        client.run(&["config", "user.email", "test@example.com"], root).unwrap();
        client.run(&["config", "commit.gpgsign", "false"], root).unwrap();
        std::fs::write(root.join("base.txt"), "base\n").unwrap();
        client.run(&["add", "--all"], root).unwrap();
        client.run(&["commit", "-q", "-m", "Base"], root).unwrap();
        (dir, client)
    }

    /// Commits the staged changes as `author` on a fixed date, with `message`.
    fn commit_as(root: &Path, author: &str, message: &str) {
        let status = Command::new("git")
            .args(["commit", "-q", "-m", message])
            .env("GIT_AUTHOR_NAME", author)
            .env("GIT_AUTHOR_EMAIL", "writer@example.com")
            .env("GIT_AUTHOR_DATE", "1600000000 +0000")
            .current_dir(root)
            .status()
            .unwrap();
        assert!(status.success());
    }

    fn head(client: &GitClient, root: &Path) -> String {
        client.run_trimmed(&["rev-parse", "HEAD"], root).unwrap()
    }

    /// Every file goes whole into the first commit when its path is in `chosen`.
    fn whole_choices(files: &[SplitFile], chosen: &[&str]) -> Vec<FileChoice> {
        files.iter().map(|file| if chosen.contains(&file.path.as_str()) { FileChoice::Whole } else { FileChoice::Nothing }).collect()
    }

    fn request(client: &GitClient, root: &Path, commit: &str, first_patch: String, first: &str, second: &str) -> SplitRequest {
        SplitRequest {
            branch: "main".to_string(),
            expected_head: head(client, root),
            commit: commit.to_string(),
            first_patch,
            first_message: first.to_string(),
            second_message: second.to_string(),
        }
    }

    #[test]
    fn splits_head_into_two_commits_and_keeps_the_working_tree_and_index() {
        let (dir, client) = repository();
        let root = dir.path();
        std::fs::write(root.join("a.txt"), "one\n").unwrap();
        std::fs::write(root.join("b.txt"), "one\n").unwrap();
        client.run(&["add", "--all"], root).unwrap();
        client.run(&["commit", "-q", "-m", "Add a and b"], root).unwrap();
        std::fs::write(root.join("a.txt"), "two\n").unwrap();
        std::fs::write(root.join("b.txt"), "two\n").unwrap();
        client.run(&["add", "--all"], root).unwrap();
        commit_as(root, "Writer", "Change a and b\n\nWith a body.\n");
        let original = head(&client, root);
        let original_parent = client.run_trimmed(&["rev-parse", "HEAD~1"], root).unwrap();
        // Work that must survive the split: a staged new file and an unstaged edit to a.txt.
        std::fs::write(root.join("staged.txt"), "staged\n").unwrap();
        client.run(&["add", "staged.txt"], root).unwrap();
        std::fs::write(root.join("a.txt"), "unstaged\n").unwrap();
        let status_before = client.run(&["status", "--porcelain=v1"], root).unwrap();

        let files = split_files(&client.split_patch(&original, root).unwrap());
        let patch = first_patch(&files, &whole_choices(&files, &["a.txt"])).unwrap();
        let outcome = client.split_commit(&request(&client, root, &original, patch, "Change a", "Change b"), root).unwrap();

        assert_eq!(outcome.previous_head, original);
        assert_eq!(head(&client, root), outcome.new_head);
        assert_eq!(client.run_trimmed(&["rev-parse", "HEAD~1"], root).unwrap(), outcome.first);
        assert_eq!(client.run_trimmed(&["rev-parse", "HEAD~2"], root).unwrap(), original_parent);
        assert_eq!(client.run_trimmed(&["show", "-s", "--format=%s", "HEAD~1"], root).unwrap(), "Change a");
        assert_eq!(client.run_trimmed(&["show", "-s", "--format=%s", "HEAD"], root).unwrap(), "Change b");
        assert_eq!(client.raw_commit_message(&outcome.second, root).unwrap(), "Change b\n");
        assert_eq!(client.run_trimmed(&["show", "--format=", "--name-only", "HEAD~1"], root).unwrap(), "a.txt");
        assert_eq!(client.run_trimmed(&["show", "--format=", "--name-only", "HEAD"], root).unwrap(), "b.txt");
        assert_eq!(client.run(&["show", "HEAD~1:a.txt"], root).unwrap(), "two\n");
        assert_eq!(client.run(&["show", "HEAD~1:b.txt"], root).unwrap(), "one\n");
        assert_eq!(client.run(&["show", "HEAD:b.txt"], root).unwrap(), "two\n");
        let author = "Writer <writer@example.com> 1600000000 +0000";
        for revision in ["HEAD~1", "HEAD"] {
            assert_eq!(client.run_trimmed(&["show", "-s", "--format=%an <%ae> %ad", "--date=raw", revision], root).unwrap(), author);
        }
        assert!(client.run(&["show", "HEAD:staged.txt"], root).is_err(), "the staged file is not committed");
        assert_eq!(
            client.run_trimmed(&["rev-parse", "HEAD^{tree}"], root).unwrap(),
            client.run_trimmed(&["rev-parse", &format!("{original}^{{tree}}")], root).unwrap()
        );
        assert_eq!(std::fs::read_to_string(root.join("a.txt")).unwrap(), "unstaged\n");
        assert_eq!(client.run(&["status", "--porcelain=v1"], root).unwrap(), status_before);
    }

    #[test]
    fn splits_an_older_commit_and_replays_the_commits_after_it() {
        let (dir, client) = repository();
        let root = dir.path();
        let lines: String = (1..=12).map(|n| format!("l{n}\n")).collect();
        std::fs::write(root.join("f.txt"), &lines).unwrap();
        client.run(&["add", "--all"], root).unwrap();
        client.run(&["commit", "-q", "-m", "Add f"], root).unwrap();
        let changed = lines.replacen("l1\n", "L1\n", 1).replacen("l12\n", "L12\n", 1);
        std::fs::write(root.join("f.txt"), &changed).unwrap();
        std::fs::write(root.join("g.txt"), "g\n").unwrap();
        client.run(&["add", "--all"], root).unwrap();
        client.run(&["commit", "-q", "-m", "Change f and add g"], root).unwrap();
        let target = head(&client, root);
        std::fs::write(root.join("h.txt"), "h\n").unwrap();
        client.run(&["add", "--all"], root).unwrap();
        client.run(&["commit", "-q", "-m", "Add h\n\nBody of h.\n"], root).unwrap();
        let later_message = client.raw_commit_message(&head(&client, root), root).unwrap();
        std::fs::write(root.join("h.txt"), "h again\n").unwrap();
        client.run(&["commit", "-q", "-a", "-m", "Change h"], root).unwrap();
        let final_tree = client.run_trimmed(&["rev-parse", "HEAD^{tree}"], root).unwrap();

        let files = split_files(&client.split_patch(&target, root).unwrap());
        let f = files.iter().find(|file| file.path == "f.txt").unwrap();
        assert_eq!(f.hunks.len(), 2, "the two changes are separate hunks");
        let choices: Vec<FileChoice> = files
            .iter()
            .map(|file| if file.path == "f.txt" { FileChoice::Hunks(BTreeSet::from([0])) } else { FileChoice::Nothing })
            .collect();
        let patch = first_patch(&files, &choices).unwrap();
        client
            .split_commit(&request(&client, root, &target, patch, "Change the top of f", "Change the bottom of f and add g"), root)
            .unwrap();

        assert_eq!(client.run_trimmed(&["rev-parse", "HEAD^{tree}"], root).unwrap(), final_tree);
        assert_eq!(
            client.run(&["log", "--format=%s", "-4"], root).unwrap(),
            "Change h\nAdd h\nChange the bottom of f and add g\nChange the top of f\n"
        );
        assert_eq!(client.raw_commit_message("HEAD~1", root).unwrap(), later_message);
        // The first new commit has only the top change; the second has the bottom change and g.txt.
        assert_eq!(client.run(&["show", "HEAD~3:f.txt"], root).unwrap(), lines.replacen("l1\n", "L1\n", 1));
        assert_eq!(client.run(&["show", "HEAD~2:f.txt"], root).unwrap(), changed);
        assert!(client.run(&["show", "HEAD~3:g.txt"], root).is_err());
        assert_eq!(client.run(&["show", "HEAD~2:g.txt"], root).unwrap(), "g\n");
    }

    #[test]
    fn refuses_a_root_commit_empty_or_full_selections_and_a_stale_branch() {
        let (dir, client) = repository();
        let root = dir.path();
        std::fs::write(root.join("a.txt"), "a\n").unwrap();
        client.run(&["add", "--all"], root).unwrap();
        client.run(&["commit", "-q", "-m", "Add a"], root).unwrap();
        let target = head(&client, root);
        let base = client.run_trimmed(&["rev-parse", "HEAD~1"], root).unwrap();
        let files = split_files(&client.split_patch(&target, root).unwrap());
        let whole = first_patch(&files, &whole_choices(&files, &["a.txt"])).unwrap();
        let nothing = first_patch(&files, &whole_choices(&files, &[])).unwrap();

        let root_commit = client.run_trimmed(&["rev-list", "--max-parents=0", "HEAD"], root).unwrap();
        assert!(client.split_commit(&request(&client, root, &root_commit, whole.clone(), "x", "y"), root).is_err());
        assert!(client.split_commit(&request(&client, root, &target, nothing, "x", "y"), root).is_err());
        assert!(client.split_commit(&request(&client, root, &target, whole.clone(), "x", "y"), root).is_err(), "all changes");
        assert!(client.split_commit(&request(&client, root, &target, whole.clone(), "  ", "y"), root).is_err());
        let mut stale = request(&client, root, &target, whole.clone(), "x", "y");
        stale.expected_head = base;
        assert!(client.split_commit(&stale, root).is_err());
        assert_eq!(head(&client, root), target);
    }

    #[test]
    fn refuses_a_range_that_includes_a_merge() {
        let (dir, client) = repository();
        let root = dir.path();
        std::fs::write(root.join("c.txt"), "c\n").unwrap();
        client.run(&["add", "--all"], root).unwrap();
        client.run(&["commit", "-q", "-m", "Add c"], root).unwrap();
        let target = head(&client, root);
        client.run(&["checkout", "-q", "-b", "side", "HEAD~1"], root).unwrap();
        std::fs::write(root.join("s.txt"), "s\n").unwrap();
        client.run(&["add", "--all"], root).unwrap();
        client.run(&["commit", "-q", "-m", "Add s"], root).unwrap();
        client.run(&["checkout", "-q", "main"], root).unwrap();
        client.run(&["merge", "-q", "--no-ff", "--no-edit", "side"], root).unwrap();

        let files = split_files(&client.split_patch(&target, root).unwrap());
        let patch = first_patch(&files, &whole_choices(&files, &["c.txt"])).unwrap();
        assert!(client.split_commit(&request(&client, root, &target, patch, "x", "y"), root).is_err());
    }

    #[test]
    fn a_failed_signature_leaves_the_branch_alone() {
        let (dir, client) = repository();
        let root = dir.path();
        std::fs::write(root.join("a.txt"), "a\n").unwrap();
        client.run(&["add", "--all"], root).unwrap();
        client.run(&["commit", "-q", "-m", "Add a"], root).unwrap();
        let target = head(&client, root);
        client.run(&["config", "commit.gpgsign", "true"], root).unwrap();
        client.run(&["config", "gpg.program", "/nonexistent/nicegit-gpg"], root).unwrap();

        let files = split_files(&client.split_patch(&target, root).unwrap());
        let patch = first_patch(&files, &whole_choices(&files, &["a.txt"])).unwrap();
        assert!(client.split_commit(&request(&client, root, &target, patch, "x", "y"), root).is_err());
        assert_eq!(head(&client, root), target);
    }

    const TWO_HUNKS: &str = "diff --git a/f.txt b/f.txt\nindex 1111111..2222222 100644\n--- a/f.txt\n+++ b/f.txt\n@@ -1,1 +1,3 @@\n-l1\n+x\n+y\n+z\n@@ -9,4 +11,4 @@\n l9\n l10\n l11\n-l12\n+L12\n@@ -20,1 +22,1 @@\n-q\n+Q\n";

    #[test]
    fn hunks_are_split_out_with_line_numbers_for_the_hunks_left_out() {
        let files = split_files(TWO_HUNKS);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, "f.txt");
        assert!(!files[0].whole_only);
        assert_eq!(files[0].hunks.len(), 3);
        assert_eq!(files[0].hunks[1].changes, vec!["-l12".to_string(), "+L12".to_string()]);

        let only_b = first_patch(&files, &[FileChoice::Hunks(BTreeSet::from([1]))]).unwrap();
        assert_eq!(only_b, "diff --git a/f.txt b/f.txt\n--- a/f.txt\n+++ b/f.txt\n@@ -9,4 +9,4 @@\n l9\n l10\n l11\n-l12\n+L12\n");

        // The first hunk adds two lines, so the second moves down by two in the first commit.
        let first_and_b = first_patch(&files, &[FileChoice::Hunks(BTreeSet::from([0, 1]))]).unwrap();
        assert_eq!(
            first_and_b,
            "diff --git a/f.txt b/f.txt\n--- a/f.txt\n+++ b/f.txt\n@@ -1,1 +1,3 @@\n-l1\n+x\n+y\n+z\n@@ -9,4 +11,4 @@\n l9\n l10\n l11\n-l12\n+L12\n"
        );

        // With the first hunk left out, the last hunk moves up by the second hunk's change alone.
        let b_and_c = first_patch(&files, &[FileChoice::Hunks(BTreeSet::from([1, 2]))]).unwrap();
        assert_eq!(
            b_and_c,
            "diff --git a/f.txt b/f.txt\n--- a/f.txt\n+++ b/f.txt\n@@ -9,4 +9,4 @@\n l9\n l10\n l11\n-l12\n+L12\n@@ -20,1 +20,1 @@\n-q\n+Q\n"
        );
    }

    #[test]
    fn new_deleted_and_mode_changes_move_only_as_whole_files() {
        let patch = concat!(
            "diff --git a/n.txt b/n.txt\nnew file mode 100644\nindex 0000000..3333333\n--- /dev/null\n+++ b/n.txt\n@@ -0,0 +1 @@\n+hello\n",
            "diff --git a/gone.txt b/gone.txt\ndeleted file mode 100644\nindex 4444444..0000000\n--- a/gone.txt\n+++ /dev/null\n@@ -1 +0,0 @@\n-bye\n",
            "diff --git a/m.sh b/m.sh\nold mode 100644\nnew mode 100755\n",
            "diff --git a/bin.dat b/bin.dat\nindex 5555555..6666666 100644\nBinary files a/bin.dat and b/bin.dat differ\n",
        );
        let files = split_files(patch);
        let summary: Vec<(&str, bool)> = files.iter().map(|file| (file.path.as_str(), file.whole_only)).collect();
        assert_eq!(summary, vec![("n.txt", true), ("gone.txt", true), ("m.sh", true), ("bin.dat", true)]);
        assert!(files[2].hunks.is_empty());
    }

    #[test]
    fn choices_must_match_the_files() {
        let files = split_files(TWO_HUNKS);
        assert!(first_patch(&files, &[]).is_err());
        assert!(first_patch(&files, &[FileChoice::Hunks(BTreeSet::from([7]))]).is_err());
    }
}
