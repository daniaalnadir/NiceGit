//! Absorbing staged changes into the earlier commits that last changed their lines, as `git absorb`
//! does. Each staged change is traced, with `git blame`, to the commit that last changed the lines it
//! removes or edits. Those commits are rebuilt with their changes folded in, and the commits after
//! them are replayed. The branch keeps its commits, and the working tree and index are not touched:
//! the index already holds the new HEAD plus the changes that stay staged.
//!
//! A change is placed by where its lines are in each rebuilt commit, which later commits move. A
//! line's place is found by diffing the rebuilt tree against HEAD. Commits that are replayed are
//! merged as a cherry-pick is, so a change next to a later commit's edit can conflict; then nothing
//! is changed.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use crate::client::GitClient;
use crate::diff::hunk_header;
use crate::models::{GitError, Result};
use crate::rebase::Scratch;
use crate::runner::{self, RunOptions};
use crate::split::{first_patch, split_files, FileChoice, SplitFile, SplitHunk};

const COMMAND: &str = "absorb";
const NOTHING_STAGED: &str = "Stage the changes to absorb first.";
const NO_SOURCE: &str = "Could not find which commit changed these lines";
const NOT_APPLIED: &str = "A staged change could not be placed in its commit, so nothing was absorbed.";
const CONFLICT: &str = "The commits after a change could not be replayed without a conflict, so nothing was absorbed.";
const DID_NOT_MATCH: &str = "The rebuilt commits did not match the changes, so nothing was absorbed.";
const REVIEW_AGAIN: &str = "The staged changes or the branch changed since they were shown. Review them again.";
const BRANCH_MOVED: &str = "The branch moved while the changes were absorbed. Refresh and review them again.";

/// One staged change: a hunk of a file, or a whole file that can only move as a whole.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AbsorbHunk {
    pub path: String,
    /// Git's `@@` line, or empty for a whole file.
    pub header: String,
    /// The change as a patch against HEAD: the file's header lines, then the hunk.
    pub patch: String,
}

/// A commit that changes will be folded into, with the changes that go into it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AbsorbTarget {
    pub commit: String,
    pub subject: String,
    pub hunks: Vec<AbsorbHunk>,
}

/// Where the staged changes go. `unassigned` holds the changes that stay staged, each with the reason.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AbsorbPlan {
    pub head: String,
    pub branch: String,
    pub targets: Vec<AbsorbTarget>,
    pub unassigned: Vec<(AbsorbHunk, String)>,
}

/// Where a traced hunk sits in the staged diff: its file and its hunk, by index.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct HunkRef {
    file: usize,
    hunk: usize,
}

/// What tracing found for one hunk: the commit it belongs to, or why it stays staged.
enum Source {
    Commit(String),
    Refused(String),
}

/// The staged diff, traced to commits.
#[derive(Clone)]
struct Traced {
    head: String,
    branch: String,
    files: Vec<SplitFile>,
    /// Each hunk that goes to a commit, in staged-diff order.
    assigned: Vec<(HunkRef, String)>,
    unassigned: Vec<(AbsorbHunk, String)>,
    /// The commits not yet on a remote, oldest first. Only these can be rewritten.
    unpushed: Vec<String>,
}

/// Works out which commit each staged change belongs to. Nothing is changed.
pub fn plan_absorb(client: &GitClient, directory: &Path) -> Result<AbsorbPlan> {
    let traced = trace(client, directory)?;
    build_plan(client, traced, directory)
}

/// Folds the chosen targets' staged changes into their commits, then replays the commits after them.
/// `accepted` holds commit hashes from `plan`. Refuses unless the branch, HEAD, and staged changes
/// are still those `plan` shows. Returns the number of changes absorbed.
pub fn apply_absorb(client: &GitClient, plan: &AbsorbPlan, accepted: &BTreeSet<String>, directory: &Path) -> Result<usize> {
    client.require_checkout(&plan.branch, Some(&plan.head), COMMAND, directory)?;
    let traced = trace(client, directory)?;
    if build_plan(client, traced.clone(), directory)? != *plan {
        return Err(GitError::failed(COMMAND, REVIEW_AGAIN));
    }
    if accepted.is_empty() {
        return Err(GitError::failed(COMMAND, "Choose at least one commit to absorb into."));
    }
    if !accepted.iter().all(|commit| plan.targets.iter().any(|target| &target.commit == commit)) {
        return Err(GitError::failed(COMMAND, REVIEW_AGAIN));
    }

    // The accepted commits by their place in the first-parent chain, from the oldest to HEAD.
    let oldest =
        plan.targets.iter().find(|target| accepted.contains(&target.commit)).ok_or_else(|| GitError::failed(COMMAND, REVIEW_AGAIN))?;
    let base = client.resolve_commit(&format!("{}^", oldest.commit), directory)?;
    let chain: Vec<String> = client
        .run_trimmed(&["rev-list", "--reverse", "--topo-order", "--first-parent", &format!("{base}..{}", plan.head), "--"], directory)?
        .lines()
        .map(str::to_string)
        .collect();
    let mut groups: BTreeMap<usize, Vec<HunkRef>> = BTreeMap::new();
    for (reference, commit) in &traced.assigned {
        if accepted.contains(commit) {
            let index = chain.iter().position(|item| item == commit).ok_or_else(|| GitError::failed(COMMAND, REVIEW_AGAIN))?;
            groups.entry(index).or_default().push(*reference);
        }
    }

    let sign = client.signing_setup(directory)?.sign_by_default;
    let scratch = Scratch::new(COMMAND)?;
    let head_tree = tree_of(client, &plan.head, directory)?;
    let rebuild = Rebuild { client, files: &traced.files, head_tree: &head_tree, scratch: &scratch, directory };

    let mut tip = base.clone();
    let mut step = 0usize;
    for (index, commit) in chain.iter().enumerate() {
        let parent = if index == 0 { base.as_str() } else { chain[index - 1].as_str() };
        step += 1;
        let mut tree = merge(client, parent, &tip, commit, directory)?;
        for reference in groups.get(&index).into_iter().flatten() {
            step += 1;
            tree = rebuild.place(&tree, *reference, step)?;
        }
        let author = client.author_of(commit, directory)?;
        let message = client.raw_commit_message(commit, directory)?;
        step += 1;
        tip = client.commit_tree_as(&scratch, step, &tree, &[tip.as_str()], &message, &author, sign, directory)?;
    }

    // Every accepted change must be in the rebuilt branch, and nothing else may have changed.
    let all: Vec<HunkRef> = groups.values().flat_map(|refs| refs.iter().copied()).collect();
    step += 1;
    let expected = rebuild.state(&all, step)?;
    if tree_of(client, &tip, directory)? != expected {
        return Err(GitError::failed(COMMAND, DID_NOT_MATCH));
    }

    let reflog = format!("nicegit: absorb into {} commits", groups.len());
    let reference = format!("refs/heads/{}", plan.branch);
    client
        .run(&["update-ref", "-m", &reflog, &reference, &tip, &plan.head], directory)
        .map_err(|_| GitError::failed(COMMAND, BRANCH_MOVED))?;
    Ok(all.len())
}

/// Reads the staged diff and traces each hunk to the commit that last changed its lines.
fn trace(client: &GitClient, directory: &Path) -> Result<Traced> {
    let state = client.checkout_state(directory)?;
    if state.operation.is_some() {
        return Err(GitError::failed(COMMAND, "Finish or abort the current Git operation first."));
    }
    let branch = client
        .try_trimmed(&["symbolic-ref", "--quiet", "--short", "HEAD"], directory)
        .ok_or_else(|| GitError::failed(COMMAND, "Check out a branch first."))?;
    let head = state.head_hash.ok_or_else(|| GitError::failed(COMMAND, "Make a commit before absorbing changes."))?;

    let patch = client.run(
        &[
            "diff",
            "--cached",
            "--no-ext-diff",
            "--no-textconv",
            "--no-color",
            "--no-renames",
            "--unified=0",
            "--src-prefix=a/",
            "--dst-prefix=b/",
        ],
        directory,
    )?;
    if patch.trim().is_empty() {
        return Err(GitError::failed(COMMAND, NOTHING_STAGED));
    }
    let files = split_files(&patch);
    let unpushed: Vec<String> = client
        .run_trimmed(&["rev-list", "--reverse", "--topo-order", &head, "--not", "--remotes", "--"], directory)?
        .lines()
        .map(str::to_string)
        .collect();

    let mut assigned = Vec::new();
    let mut unassigned = Vec::new();
    for (file_index, file) in files.iter().enumerate() {
        if file.whole_only || file.hunks.is_empty() {
            unassigned.push((whole_file(file), whole_reason(file)));
            continue;
        }
        for (hunk_index, hunk) in file.hunks.iter().enumerate() {
            let reference = HunkRef { file: file_index, hunk: hunk_index };
            match source_of(client, &head, file, hunk, &unpushed, directory)? {
                Source::Commit(commit) => assigned.push((reference, commit)),
                Source::Refused(reason) => unassigned.push((absorb_hunk(file, hunk), reason)),
            }
        }
    }
    Ok(Traced { head, branch, files, assigned, unassigned, unpushed })
}

/// Groups the traced hunks by commit, in the order of the unpushed commits, and reads each commit's subject.
fn build_plan(client: &GitClient, traced: Traced, directory: &Path) -> Result<AbsorbPlan> {
    let mut by_commit: BTreeMap<String, Vec<HunkRef>> = BTreeMap::new();
    for (reference, commit) in &traced.assigned {
        by_commit.entry(commit.clone()).or_default().push(*reference);
    }
    let mut targets = Vec::new();
    for commit in &traced.unpushed {
        let Some(refs) = by_commit.remove(commit) else { continue };
        let subject = client.run_trimmed(&["show", "-s", "--format=%s", commit, "--"], directory)?;
        let hunks = refs.iter().map(|reference| hunk_of(&traced.files, *reference)).collect();
        targets.push(AbsorbTarget { commit: commit.clone(), subject, hunks });
    }
    Ok(AbsorbPlan { head: traced.head, branch: traced.branch, targets, unassigned: traced.unassigned })
}

fn hunk_of(files: &[SplitFile], reference: HunkRef) -> AbsorbHunk {
    absorb_hunk(&files[reference.file], &files[reference.file].hunks[reference.hunk])
}

/// Finds the commit that last changed the lines a hunk removes or edits. A pure addition uses the
/// line above it, or the line below at the top of the file.
fn source_of(client: &GitClient, head: &str, file: &SplitFile, hunk: &SplitHunk, unpushed: &[String], directory: &Path) -> Result<Source> {
    let (first, last) = blamed_lines(hunk);
    let range = format!("{first},{last}");
    let output = match client.run(&["blame", "--porcelain", "-L", &range, head, "--", &file.path], directory) {
        Ok(output) => output,
        Err(_) => return Ok(Source::Refused(NO_SOURCE.to_string())),
    };
    let commits = blamed_commits(&output);
    let [commit] = commits.as_slice() else {
        let reason = if commits.is_empty() { NO_SOURCE.to_string() } else { format!("Lines come from {} commits", commits.len()) };
        return Ok(Source::Refused(reason));
    };
    if !unpushed.contains(commit) {
        return Ok(Source::Refused("Commit already pushed".to_string()));
    }
    let parents = client.run_trimmed(&["rev-list", "--parents", "-n", "1", commit, "--"], directory)?;
    match parents.split_whitespace().count().saturating_sub(1) {
        0 => return Ok(Source::Refused("The first commit on the branch".to_string())),
        1 => {}
        _ => return Ok(Source::Refused("Merge commit".to_string())),
    }
    // Every commit from this one to HEAD is replayed, so none of them may be a merge.
    let merges = client.run_trimmed(&["rev-list", "--min-parents=2", &format!("{commit}^..{head}"), "--"], directory)?;
    if !merges.is_empty() {
        return Ok(Source::Refused("A merge commit comes after it".to_string()));
    }
    Ok(Source::Commit(commit.clone()))
}

/// The lines of the file, in HEAD, that a hunk's change is traced through. For a pure addition, the
/// line above it, or the first line when it is at the top.
fn blamed_lines(hunk: &SplitHunk) -> (usize, usize) {
    if hunk.old_count > 0 {
        (hunk.old_first, hunk.old_first + hunk.old_count - 1)
    } else {
        let line = hunk.old_first.saturating_sub(1).max(1);
        (line, line)
    }
}

/// The distinct commits named in `git blame --porcelain` output. Each line's header starts with its
/// commit, and content lines start with a tab.
fn blamed_commits(output: &str) -> Vec<String> {
    let mut commits = BTreeSet::new();
    for line in output.lines() {
        if line.starts_with('\t') {
            continue;
        }
        let Some(first) = line.split(' ').next() else { continue };
        if (first.len() == 40 || first.len() == 64) && first.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            commits.insert(first.to_string());
        }
    }
    commits.into_iter().collect()
}

fn whole_reason(file: &SplitFile) -> String {
    if file.headers.iter().any(|text| text.starts_with("new file mode")) {
        "New file".to_string()
    } else if file.headers.iter().any(|text| text.starts_with("deleted file mode")) {
        "Deleted file".to_string()
    } else {
        "Binary or mode change".to_string()
    }
}

fn whole_file(file: &SplitFile) -> AbsorbHunk {
    AbsorbHunk { path: file.path.clone(), header: String::new(), patch: file.patch.clone() }
}

/// One hunk as its own patch: the file's header lines, then the hunk.
fn absorb_hunk(file: &SplitFile, hunk: &SplitHunk) -> AbsorbHunk {
    let mut patch = String::new();
    for header in file.headers.iter().filter(|header| !header.starts_with("index ")) {
        patch.push_str(header);
        patch.push('\n');
    }
    patch.push_str(&hunk.header);
    patch.push('\n');
    patch.push_str(&hunk.body);
    AbsorbHunk { path: file.path.clone(), header: hunk.header.clone(), patch }
}

/// Where HEAD's lines are in another tree of the same file. Lines that differ between the two are
/// not mapped; each entry is a hunk of the diff, as HEAD's start, HEAD's count, and the other tree's count.
struct LineMap {
    changes: Vec<(usize, usize, usize)>,
}

impl LineMap {
    /// The line in the other tree that HEAD's `line` is, or `None` if `line` itself differs. Line
    /// zero is the top of the file, before any line.
    fn line(&self, line: usize) -> Option<usize> {
        let mut shift: isize = 0;
        for &(start, head_count, other_count) in &self.changes {
            if head_count == 0 {
                // Lines of the other tree removed after HEAD's line `start`.
                if line > start {
                    shift += other_count as isize;
                }
            } else if line >= start && line < start + head_count {
                return None;
            } else if line >= start + head_count {
                shift += other_count as isize - head_count as isize;
            }
        }
        usize::try_from(line as isize + shift).ok()
    }
}

/// The trees the rebuilt commits are made from. A rebuilt tree is HEAD with some of the staged
/// changes applied, or a commit's own tree with the changes that come before it.
struct Rebuild<'a> {
    client: &'a GitClient,
    files: &'a [SplitFile],
    head_tree: &'a str,
    scratch: &'a Scratch,
    directory: &'a Path,
}

impl Rebuild<'_> {
    /// The tree of HEAD with `selected` applied. Each change is placed by its line in HEAD, which is exact.
    fn state(&self, selected: &[HunkRef], number: usize) -> Result<String> {
        if selected.is_empty() {
            return Ok(self.head_tree.to_string());
        }
        let mut picked: BTreeMap<usize, BTreeSet<usize>> = BTreeMap::new();
        for reference in selected {
            picked.entry(reference.file).or_default().insert(reference.hunk);
        }
        let choices: Vec<FileChoice> = (0..self.files.len())
            .map(|file| picked.get(&file).map_or(FileChoice::Nothing, |hunks| FileChoice::Hunks(hunks.clone())))
            .collect();
        let patch = first_patch(self.files, &choices)?;
        self.apply_to(self.head_tree, &patch, &format!("state-{number}"))
    }

    /// The tree `tree` with one staged hunk applied. The hunk's lines are found in `tree` by mapping
    /// HEAD's line numbers onto it, so a commit's later edits above the hunk do not move it.
    fn place(&self, tree: &str, reference: HunkRef, number: usize) -> Result<String> {
        let file = &self.files[reference.file];
        let hunk = &file.hunks[reference.hunk];
        let map = self.line_map(tree, &file.path)?;
        let header = if hunk.old_count > 0 {
            let start = map.line(hunk.old_first).ok_or_else(|| GitError::failed(COMMAND, NOT_APPLIED))?;
            let end = map.line(hunk.old_first + hunk.old_count - 1).ok_or_else(|| GitError::failed(COMMAND, NOT_APPLIED))?;
            if end + 1 != start + hunk.old_count {
                return Err(GitError::failed(COMMAND, NOT_APPLIED));
            }
            format!("@@ -{start},{} +{start},{} @@", hunk.old_count, hunk.new_count)
        } else {
            // An insertion follows the line above it, which is line zero at the top of the file.
            let after = map.line(hunk.old_first - 1).ok_or_else(|| GitError::failed(COMMAND, NOT_APPLIED))?;
            format!("@@ -{after},0 +{},{} @@", after + 1, hunk.new_count)
        };
        let mut patch = String::new();
        for text in file.headers.iter().filter(|text| !text.starts_with("index ")) {
            patch.push_str(text);
            patch.push('\n');
        }
        patch.push_str(&header);
        patch.push('\n');
        patch.push_str(&hunk.body);
        self.apply_to(tree, &patch, &format!("place-{number}"))
    }

    /// Maps HEAD's lines onto `tree` for one file. The diff reads from `tree` to HEAD, so its
    /// new side is HEAD.
    fn line_map(&self, tree: &str, path: &str) -> Result<LineMap> {
        let literal = format!(":(literal){path}");
        let diff = self.client.run(
            &["diff", "--no-ext-diff", "--no-textconv", "--no-color", "--no-renames", "--unified=0", tree, self.head_tree, "--", &literal],
            self.directory,
        )?;
        let changes = diff
            .lines()
            .filter(|line| line.starts_with("@@"))
            .filter_map(hunk_header)
            // The new side is HEAD, so each hunk is HEAD's start and count, and the tree's count.
            .map(|(_, old_count, new_start, new_count)| (new_start, new_count, old_count))
            .collect();
        Ok(LineMap { changes })
    }

    /// Applies `patch` to `tree` in a temporary index, and returns the resulting tree.
    fn apply_to(&self, tree: &str, patch: &str, name: &str) -> Result<String> {
        let patch_file = self.scratch.write(COMMAND, &format!("{name}.patch"), patch.as_bytes())?;
        let patch_path = patch_file.to_string_lossy().into_owned();
        let index_file = self.scratch.path(&format!("{name}.index")).to_string_lossy().into_owned();
        let environment = [("GIT_INDEX_FILE", index_file.as_str())];
        let options = RunOptions { env: &environment, ..self.client.options() };
        runner::run(&["read-tree", tree], self.directory, &options)?;
        runner::run(&["apply", "--cached", "--unidiff-zero", "--whitespace=nowarn", "--", &patch_path], self.directory, &options)
            .map_err(|_| GitError::failed(COMMAND, NOT_APPLIED))?;
        Ok(runner::run(&["write-tree"], self.directory, &options)?.trim().to_string())
    }
}

fn tree_of(client: &GitClient, revision: &str, directory: &Path) -> Result<String> {
    client.run_trimmed(&["rev-parse", "--verify", &format!("{revision}^{{tree}}")], directory)
}

/// The tree of a 3-way merge of `ours` and `theirs` from `base`, made without touching the working tree.
fn merge(client: &GitClient, base: &str, ours: &str, theirs: &str, directory: &Path) -> Result<String> {
    let merge_base = format!("--merge-base={base}");
    // Git exits with 1 when the merge conflicts, and that is the only expected failure here.
    let output = client
        .run(&["merge-tree", "--write-tree", &merge_base, ours, theirs], directory)
        .map_err(|_| GitError::failed(COMMAND, CONFLICT))?;
    Ok(output.lines().next().unwrap_or_default().trim().to_string())
}
