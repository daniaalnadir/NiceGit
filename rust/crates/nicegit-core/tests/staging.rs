//! Integration tests for line staging, conflict resolution, and inline highlights, run against
//! real repositories in temporary directories.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;
use std::process::{Command, Output};

use nicegit_core::client::GitClient;
use nicegit_core::conflict::has_conflict_markers;
use nicegit_core::diff::{parse_diff, DiffLine};
use nicegit_core::inline::{self, InlineChange};
use nicegit_core::staging::{DiffHunk, FileReview};
use nicegit_core::GitError;
use tempfile::TempDir;

// MARK: Helpers

/// Runs raw Git in `directory`, returning its output without checking the status.
fn git_output(directory: &Path, arguments: &[&str]) -> Output {
    let mut command = Command::new("git");
    command.args(arguments).current_dir(directory);
    // Inherited repository variables would redirect these commands to another repository.
    for key in ["GIT_DIR", "GIT_WORK_TREE", "GIT_INDEX_FILE", "GIT_OBJECT_DIRECTORY", "GIT_COMMON_DIR"] {
        command.env_remove(key);
    }
    command.output().unwrap_or_else(|error| panic!("could not run git {arguments:?}: {error}"))
}

/// Runs raw Git in `directory` and returns its standard output, panicking on failure.
fn git(directory: &Path, arguments: &[&str]) -> String {
    let output = git_output(directory, arguments);
    assert!(output.status.success(), "git {arguments:?} failed: {}", String::from_utf8_lossy(&output.stderr));
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// Sets the identity and signing options in the repository's own config only.
fn configure(directory: &Path) {
    for (key, value) in [
        ("user.name", "Test"),
        ("user.email", "test@example.com"),
        ("commit.gpgsign", "false"),
        ("tag.gpgsign", "false"),
        // Keeps line endings byte-for-byte on Windows, where Git may default to autocrlf.
        ("core.autocrlf", "false"),
    ] {
        git(directory, &["config", key, value]);
    }
}

/// A temporary repository on branch `main`.
struct Repo {
    dir: TempDir,
}

impl Repo {
    fn new() -> Self {
        let dir = tempfile::tempdir().expect("create temp dir");
        git(dir.path(), &["init", "-b", "main"]);
        configure(dir.path());
        Repo { dir }
    }

    fn path(&self) -> &Path {
        self.dir.path()
    }

    fn git(&self, arguments: &[&str]) -> String {
        git(self.path(), arguments)
    }

    fn write(&self, relative: &str, contents: &str) {
        self.write_bytes(relative, contents.as_bytes());
    }

    fn write_bytes(&self, relative: &str, contents: &[u8]) {
        let file = self.path().join(relative);
        if let Some(parent) = file.parent() {
            fs::create_dir_all(parent).expect("create parent folder");
        }
        fs::write(file, contents).expect("write file");
    }

    fn read(&self, relative: &str) -> String {
        fs::read_to_string(self.path().join(relative)).expect("read file")
    }

    /// The file's content as the index holds it.
    fn index_text(&self, relative: &str) -> String {
        self.git(&["show", &format!(":{relative}")])
    }

    /// Writes, stages, and commits one file.
    fn commit(&self, relative: &str, contents: &str, message: &str) {
        self.write(relative, contents);
        self.git(&["add", "--", relative]);
        self.git(&["commit", "--quiet", "-m", message]);
    }
}

fn client() -> GitClient {
    GitClient::new()
}

/// The index of the diff line with exactly this text, panicking unless it is unique.
fn line_index(review: &FileReview, text: &str) -> usize {
    let matches: Vec<usize> = review.lines.iter().enumerate().filter(|(_, line)| line.text == text).map(|(index, _)| index).collect();
    assert_eq!(matches.len(), 1, "expected exactly one diff line {text:?}, found {matches:?}");
    matches[0]
}

fn select(review: &FileReview, texts: &[&str]) -> BTreeSet<usize> {
    texts.iter().map(|text| line_index(review, text)).collect()
}

// MARK: Line staging

#[test]
fn staging_one_of_two_hunks_leaves_the_other_unstaged() {
    let repo = Repo::new();
    let original: String = (1..=20).map(|n| format!("line {n}\n")).collect();
    repo.commit("notes.txt", &original, "Base");
    let edited = original.replace("line 2\n", "two\n").replace("line 19\n", "nineteen\n");
    repo.write("notes.txt", &edited);

    let review = client().file_review("notes.txt", false, repo.path()).expect("review");
    assert_eq!(DiffHunk::grouped(&review.lines, 3).len(), 2);
    let selected = select(&review, &["-line 2", "+two"]);
    client().stage_lines(&selected, &review, repo.path()).expect("stage first hunk");

    assert_eq!(repo.index_text("notes.txt"), original.replace("line 2\n", "two\n"));
    assert_eq!(repo.read("notes.txt"), edited, "the working file is not changed by staging");
}

#[test]
fn stages_a_single_added_line_and_leaves_the_other_addition() {
    let repo = Repo::new();
    repo.commit("list.txt", "a\nb\nc\n", "Base");
    repo.write("list.txt", "a\nb\nNEW\nc\nOTHER\n");

    let review = client().file_review("list.txt", false, repo.path()).expect("review");
    let selected = select(&review, &["+NEW"]);
    client().stage_lines(&selected, &review, repo.path()).expect("stage one line");

    assert_eq!(repo.index_text("list.txt"), "a\nb\nNEW\nc\n");
}

#[test]
fn unstages_one_line_of_a_staged_change() {
    let repo = Repo::new();
    repo.commit("letters.txt", "a\nb\nc\n", "Base");
    repo.write("letters.txt", "A\nb\nC\n");
    repo.git(&["add", "--", "letters.txt"]);

    let review = client().file_review("letters.txt", true, repo.path()).expect("staged review");
    // Unstaging the added C restores nothing else: the staged deletion of c remains.
    let selected = select(&review, &["+C"]);
    client().stage_lines(&selected, &review, repo.path()).expect("unstage one line");

    assert_eq!(repo.index_text("letters.txt"), "A\nb\n");
    assert_eq!(repo.read("letters.txt"), "A\nb\nC\n", "the working file keeps every change");
}

#[test]
fn unstages_a_replacement_pair_together() {
    let repo = Repo::new();
    repo.commit("letters.txt", "a\nb\nc\n", "Base");
    repo.write("letters.txt", "A\nb\nC\n");
    repo.git(&["add", "--", "letters.txt"]);

    let review = client().file_review("letters.txt", true, repo.path()).expect("staged review");
    let selected = select(&review, &["-a", "+A"]);
    client().stage_lines(&selected, &review, repo.path()).expect("unstage the first replacement");

    assert_eq!(repo.index_text("letters.txt"), "a\nb\nC\n");
}

#[test]
fn file_content_that_looks_like_patch_headers_is_staged_as_content() {
    let repo = Repo::new();
    let base = "--- /dev/null\n+++ b/x\n@@ -1 +1 @@\nend\n";
    repo.commit("header-like.txt", base, "Base");
    repo.write("header-like.txt", "--- /dev/null\n+++ b/y\n@@ -1 +1 @@\nEND\n");

    let review = client().file_review("header-like.txt", false, repo.path()).expect("review");
    assert_eq!(review.line_staging_unavailable, None);
    // Content lines carry their own sign, so the removed header-like line reads "-+++ b/x".
    let selected = select(&review, &["-+++ b/x", "++++ b/y"]);
    client().stage_lines(&selected, &review, repo.path()).expect("stage content line");

    assert_eq!(repo.index_text("header-like.txt"), "--- /dev/null\n+++ b/y\n@@ -1 +1 @@\nend\n");
}

#[test]
fn filenames_with_spaces_unicode_tabs_and_glob_characters_stage_only_the_selected_file() {
    let repo = Repo::new();
    let awkward = "dir/café file\tname.txt";
    let glob = "x[1].txt";
    let lookalike = "x1.txt";
    repo.commit(awkward, "one\ntwo\n", "Base awkward");
    repo.commit(glob, "one\ntwo\n", "Base glob");
    repo.commit(lookalike, "one\ntwo\n", "Base lookalike");
    repo.write(awkward, "one\nTWO\n");
    repo.write(glob, "one\nTWO\n");
    repo.write(lookalike, "one\nTWO\n");

    let review = client().file_review(awkward, false, repo.path()).expect("review awkward name");
    let selected = select(&review, &["-two", "+TWO"]);
    client().stage_lines(&selected, &review, repo.path()).expect("stage awkward name");
    assert_eq!(repo.index_text(awkward), "one\nTWO\n");

    let review = client().file_review(glob, false, repo.path()).expect("review glob name");
    let selected = select(&review, &["-two", "+TWO"]);
    client().stage_lines(&selected, &review, repo.path()).expect("stage glob name");
    assert_eq!(repo.index_text(glob), "one\nTWO\n");
    // The file whose name only matches the glob pattern keeps its index entry.
    assert_eq!(repo.index_text(lookalike), "one\ntwo\n");
}

#[test]
fn partial_staging_of_a_new_file_creates_it_with_the_selected_lines() {
    let repo = Repo::new();
    repo.commit("seed.txt", "seed\n", "Base");
    repo.write("new.txt", "a\nb\nc\n");

    let review = client().file_review("new.txt", false, repo.path()).expect("review");
    assert!(review.untracked);
    let selected = select(&review, &["+a", "+c"]);
    client().stage_lines(&selected, &review, repo.path()).expect("stage selected lines of a new file");

    assert_eq!(repo.index_text("new.txt"), "a\nc\n");
    assert_eq!(repo.read("new.txt"), "a\nb\nc\n");
}

#[test]
fn a_missing_final_newline_is_part_of_the_change() {
    let repo = Repo::new();
    repo.commit("end.txt", "a\nb\n", "Base");
    repo.write("end.txt", "a\nB");

    let review = client().file_review("end.txt", false, repo.path()).expect("review");
    let selected = select(&review, &["-b", "+B"]);
    client().stage_lines(&selected, &review, repo.path()).expect("stage replacement");
    assert_eq!(repo.index_text("end.txt"), "a\nB");
}

#[test]
fn staging_only_the_addition_keeps_the_old_line_and_its_missing_newline() {
    let repo = Repo::new();
    repo.commit("end.txt", "a\nb\n", "Base");
    repo.write("end.txt", "a\nB");

    let review = client().file_review("end.txt", false, repo.path()).expect("review");
    let selected = select(&review, &["+B"]);
    client().stage_lines(&selected, &review, repo.path()).expect("stage the addition only");
    // The unselected deletion keeps `b`, and the selected `B` lacks a final newline.
    assert_eq!(repo.index_text("end.txt"), "a\nb\nB");
}

#[test]
fn empty_context_lines_survive_when_git_suppresses_blank_prefixes() {
    let repo = Repo::new();
    repo.git(&["config", "diff.suppressBlankEmpty", "true"]);
    repo.commit("blank.txt", "x\n\ny\n", "Base");
    repo.write("blank.txt", "x\n\nY\nz\n");

    let review = client().file_review("blank.txt", false, repo.path()).expect("review");
    let selected = select(&review, &["+z"]);
    client().stage_lines(&selected, &review, repo.path()).expect("stage with blank context");
    assert_eq!(repo.index_text("blank.txt"), "x\n\ny\nz\n");
}

#[test]
fn a_stale_review_is_refused_without_touching_the_index() {
    let repo = Repo::new();
    repo.commit("stale.txt", "a\nb\n", "Base");
    repo.write("stale.txt", "a\nB\n");
    let review = client().file_review("stale.txt", false, repo.path()).expect("review");
    repo.write("stale.txt", "a\nB2\n");

    let selected = select(&review, &["-b", "+B"]);
    let error = client().stage_lines(&selected, &review, repo.path()).expect_err("stale review refused");
    assert!(error.to_string().contains("changed"), "unexpected error: {error}");
    assert_eq!(repo.index_text("stale.txt"), "a\nb\n");
}

#[test]
fn binary_files_are_refused_for_line_staging() {
    let repo = Repo::new();
    repo.write_bytes("blob.bin", &[0, 1, 2, 3]);
    repo.git(&["add", "--", "blob.bin"]);
    repo.git(&["commit", "--quiet", "-m", "Base"]);
    repo.write_bytes("blob.bin", &[0, 9, 9, 3]);

    let review = client().file_review("blob.bin", false, repo.path()).expect("review");
    let reason = review.line_staging_unavailable.expect("binary files need whole-file staging");
    assert!(reason.contains("whole-file"), "unexpected reason: {reason}");
}

#[test]
fn file_review_reports_staged_renames_as_whole_file_only() {
    let repo = Repo::new();
    repo.commit("old.txt", "one\ntwo\nthree\nfour\n", "Base");
    repo.git(&["mv", "old.txt", "new.txt"]);
    let review = client().file_review("new.txt", true, repo.path()).expect("review");
    assert!(review.line_staging_unavailable.is_some());
}

#[test]
fn hunk_grouping_and_inline_highlights_work_on_parsed_patches() {
    let lines: Vec<DiffLine> = parse_diff("@@ -1,3 +1,3 @@\n keep\n-let v = 1;\n+let v = 2;\n tail\n");
    let hunks = DiffHunk::grouped(&lines, 1);
    assert_eq!(hunks.len(), 1);
    assert_eq!(hunks[0].changed_indices, BTreeSet::from([2, 3]));

    let highlights = inline::highlights(&lines);
    assert_eq!(highlights[&2], InlineChange { prefix: "let v = ".into(), changed: "1".into(), suffix: ";".into() });
    assert_eq!(highlights[&3].changed, "2");
}

// MARK: Conflicts

/// A repository with `a.txt` conflicted between `main` ("main") and `side` ("side"), optionally
/// after committing `attributes` as `.gitattributes`.
fn conflicted_repo(attributes: Option<&str>) -> Repo {
    let repo = Repo::new();
    if let Some(attributes) = attributes {
        repo.commit(".gitattributes", attributes, "Attributes");
    }
    repo.commit("a.txt", "base\n", "Base");
    repo.git(&["switch", "-c", "side"]);
    repo.commit("a.txt", "side\n", "Side change");
    repo.git(&["switch", "main"]);
    repo.commit("a.txt", "main\n", "Main change");
    let merge = git_output(repo.path(), &["merge", "--no-edit", "side"]);
    assert!(!merge.status.success(), "the merge was expected to conflict");
    repo
}

fn unmerged_paths(repo: &Repo) -> String {
    repo.git(&["ls-files", "-u"])
}

#[test]
fn load_reads_the_base_current_and_incoming_versions() {
    let repo = conflicted_repo(None);
    let document = client().load_conflict("a.txt", repo.path()).expect("load conflict");
    assert_eq!(document.base.as_deref(), Some("base\n"));
    assert_eq!(document.current.as_deref(), Some("main\n"));
    assert_eq!(document.incoming.as_deref(), Some("side\n"));
    assert_eq!(document.marker_size, 7);
    assert!(document.content.contains("<<<<<<<"));
}

#[test]
fn a_result_with_markers_is_refused_and_the_file_is_untouched() {
    let repo = conflicted_repo(None);
    let document = client().load_conflict("a.txt", repo.path()).expect("load conflict");
    let before = repo.read("a.txt");
    let error = client().resolve_conflict(&document, &document.content, repo.path()).expect_err("markers refused");
    assert!(error.to_string().contains("conflict markers"), "unexpected error: {error}");
    assert_eq!(repo.read("a.txt"), before);
    assert!(!unmerged_paths(&repo).is_empty(), "the conflict is still unresolved");
}

#[test]
fn a_custom_marker_size_is_honored_in_both_directions() {
    let repo = conflicted_repo(Some("a.txt conflict-marker-size=3\n"));
    let document = client().load_conflict("a.txt", repo.path()).expect("load conflict");
    assert_eq!(document.marker_size, 3);

    let error = client().resolve_conflict(&document, "keep\n===\n", repo.path()).expect_err("three-character marker refused");
    assert!(error.to_string().contains("conflict markers"), "unexpected error: {error}");

    // A run of two is not a marker at size three, so this line is allowed.
    client().resolve_conflict(&document, "keep\n==\n", repo.path()).expect("two characters are not a marker");
    assert_eq!(repo.read("a.txt"), "keep\n==\n");
    assert!(unmerged_paths(&repo).is_empty());
}

#[test]
fn a_clean_result_is_saved_and_staged() {
    let repo = conflicted_repo(None);
    let document = client().load_conflict("a.txt", repo.path()).expect("load conflict");
    client().resolve_conflict(&document, "resolved\n", repo.path()).expect("save resolution");

    assert_eq!(repo.read("a.txt"), "resolved\n");
    assert!(unmerged_paths(&repo).is_empty());
    assert_eq!(repo.index_text("a.txt"), "resolved\n");
}

#[test]
fn a_file_changed_on_disk_since_loading_is_not_overwritten() {
    let repo = conflicted_repo(None);
    let document = client().load_conflict("a.txt", repo.path()).expect("load conflict");
    repo.write("a.txt", "someone else\n");

    let error = client().resolve_conflict(&document, "resolved\n", repo.path()).expect_err("external edit refused");
    assert!(error.to_string().contains("changed outside"), "unexpected error: {error}");
    assert_eq!(repo.read("a.txt"), "someone else\n");
}

#[test]
fn keeping_ours_and_theirs_takes_the_whole_side() {
    let repo = conflicted_repo(None);
    client().resolve_conflict_side("a.txt", false, repo.path()).expect("keep ours");
    assert_eq!(repo.read("a.txt"), "main\n");
    assert!(unmerged_paths(&repo).is_empty());

    let repo = conflicted_repo(None);
    client().resolve_conflict_side("a.txt", true, repo.path()).expect("keep theirs");
    assert_eq!(repo.read("a.txt"), "side\n");
    assert!(unmerged_paths(&repo).is_empty());
}

#[test]
fn deleting_a_conflicted_file_resolves_it() {
    let repo = conflicted_repo(None);
    client().resolve_conflict_deletion("a.txt", repo.path()).expect("delete");
    assert!(!repo.path().join("a.txt").exists());
    assert!(unmerged_paths(&repo).is_empty());
    assert_eq!(repo.git(&["ls-files", "a.txt"]), "");
}

#[test]
fn resolving_a_file_without_a_conflict_is_refused() {
    let repo = Repo::new();
    repo.commit("plain.txt", "plain\n", "Base");
    let error = client().resolve_conflict_side("plain.txt", false, repo.path()).expect_err("no conflict");
    assert!(matches!(error, GitError::CommandFailed { .. }));
    assert!(error.to_string().contains("no longer has an unresolved conflict"));
}

#[test]
fn binary_conflicts_cannot_be_edited_but_take_a_whole_side() {
    let repo = Repo::new();
    repo.write_bytes("blob.bin", b"\0base");
    repo.git(&["add", "--", "blob.bin"]);
    repo.git(&["commit", "--quiet", "-m", "Base"]);
    repo.git(&["switch", "-c", "side"]);
    repo.write_bytes("blob.bin", b"\0side");
    repo.git(&["commit", "--quiet", "-am", "Side"]);
    repo.git(&["switch", "main"]);
    repo.write_bytes("blob.bin", b"\0main");
    repo.git(&["commit", "--quiet", "-am", "Main"]);
    assert!(!git_output(repo.path(), &["merge", "--no-edit", "side"]).status.success());

    let error = client().load_conflict("blob.bin", repo.path()).expect_err("binary file is not editable");
    assert!(error.to_string().contains("not UTF-8"), "unexpected error: {error}");
    assert_eq!(client().conflict_version("blob.bin", 2, repo.path()), None);

    client().resolve_conflict_side("blob.bin", true, repo.path()).expect("take theirs");
    assert_eq!(fs::read(repo.path().join("blob.bin")).expect("read binary"), b"\0side");
    assert!(unmerged_paths(&repo).is_empty());
}

#[test]
fn marker_detection_matches_the_configured_length() {
    assert!(has_conflict_markers(">>>>>>> side\n", 7));
    assert!(!has_conflict_markers("|||\n", 7));
    assert!(has_conflict_markers("|||\n", 3));
}
