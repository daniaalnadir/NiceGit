//! Interface tests for the feature windows, built on the helpers in `ui_tests`.

use std::path::Path;
use std::process::Command;

use egui::accesskit::{Role, Toggled};
use egui::{Key, Modifiers};
use egui_kittest::kittest::{NodeT, Queryable};
use egui_kittest::Harness;

use crate::app::{NiceGitApp, Selection};
use crate::ui_tests::*;

type App = Harness<'static, NiceGitApp>;

/// Commits a change to `file`. A fixed `date` sets both the author and committer time, so
/// commits made within the same second can still be told apart by age.
fn commit_file(dir: &Path, file: &str, contents: &str, message: &str, date: Option<&str>) {
    std::fs::write(dir.join(file), contents).unwrap();
    git(dir, &["add", file]);
    let mut command = Command::new("git");
    command.args(["commit", "-q", "-m", message]).current_dir(dir).env_remove("GIT_DIR").env_remove("GIT_WORK_TREE");
    if let Some(date) = date {
        command.env("GIT_AUTHOR_DATE", date).env("GIT_COMMITTER_DATE", date);
    }
    let output = command.output().expect("run git");
    assert!(output.status.success(), "git commit failed: {}", String::from_utf8_lossy(&output.stderr));
}

/// Whether any widget's label contains `text`. Unlike `has_label`, several matches are fine.
fn shows(harness: &App, text: &str) -> bool {
    harness.query_all_by_label_contains(text).next().is_some()
}

/// The commit selected in the graph, if any.
fn selected_commit(harness: &App) -> Option<String> {
    match harness.state().repo().map(|r| r.selection.clone()) {
        Some(Selection::Commit { hash, .. }) => Some(hash),
        _ => None,
    }
}

/// Opens a window from the Repository menu and waits until it is listed.
fn open_repository_tool(harness: &mut App, path: &Path, item: &str, title: &str) {
    repository_menu(harness, &folder_name(path), item);
    wait(harness, title, |h| h.state().tools.iter().any(|t| t.title() == title));
    settle(harness);
}

#[test]
fn bisect_finds_the_first_bad_commit_through_the_bar() {
    let repo = repository();
    let path = repo.path();
    commit_file(path, "bug.txt", "ok\n", "Add the bug file", None);
    commit_file(path, "bug.txt", "broken\n", "Introduce the bug", None);
    commit_file(path, "bug.txt", "broken\ntidy\n", "Unrelated tidy-up", None);
    let original = git(path, &["rev-parse", "HEAD"]);
    let culprit = git(path, &["rev-parse", "HEAD~1"]);

    let mut harness = open(path);
    loaded(&mut harness);
    // The commit that was known to work comes from the graph; the bad one is the current commit.
    harness.get_by_label("Add a second line").click_secondary();
    settle(&mut harness);
    harness.get_by_label_contains("Start bisect: this commit is good").click();
    wait(&mut harness, "the bisect window", |h| h.state().tools.iter().any(|t| t.id() == "bisect"));
    wait(&mut harness, "the start button", |h| shows(h, "Start bisect"));
    settle(&mut harness);
    harness.get_by_label("Start bisect").click();
    idle(&mut harness);

    // Each commit under test is checked out; the file tells us whether it has the bug.
    for _ in 0..5 {
        if shows(&harness, "First bad commit:") {
            break;
        }
        let tested = git(path, &["rev-parse", "HEAD"]);
        wait(&mut harness, "the commit under test", |h| shows(h, &format!("Bisecting: test {}", nicegit_core::models::short(&tested))));
        let contents = std::fs::read_to_string(path.join("bug.txt")).unwrap();
        let mark = if contents.contains("broken") { "Bad" } else { "Good" };
        // The bisect bar and the bisect window both offer the marks; either one records it.
        harness.query_all(egui_kittest::kittest::By::new().role(Role::Button).label(mark)).next().expect("a mark button").click();
        idle(&mut harness);
        wait(&mut harness, "the next step", |h| shows(h, "First bad commit:") || git(path, &["rev-parse", "HEAD"]) != tested);
    }
    wait(&mut harness, "the first bad commit", |h| shows(h, "First bad commit:"));
    settle(&mut harness);
    let log = git(path, &["bisect", "log"]);
    // Newer Git writes "first 'bad' commit", older Git "first bad commit".
    let verdict = log.lines().find(|line| line.starts_with("# first") && line.contains("bad")).unwrap_or_default();
    assert!(verdict.contains(&format!("[{culprit}]")), "Git found the culprit: {log}");
    assert!(shows(&harness, "Introduce the bug"), "the first bad commit's subject is shown");

    harness.get_by_label_contains("End bisect, return to main").click();
    idle(&mut harness);
    wait(&mut harness, "the bisect to end", |h| !shows(h, "First bad commit:"));
    assert_eq!(git(path, &["branch", "--show-current"]), "main", "the starting checkout is back");
    assert_eq!(git(path, &["rev-parse", "HEAD"]), original);
}

#[test]
fn clean_up_branches_deletes_a_merged_branch_and_undo_restores_it() {
    let repo = repository();
    let path = repo.path();
    // Work on a branch, so main is a merged branch that the clean-up lists.
    git(path, &["switch", "-q", "-c", "work"]);
    git(path, &["branch", "old-topic"]);
    let mut harness = open(path);
    loaded(&mut harness);

    open_repository_tool(&mut harness, path, "Clean up branches", "Clean Up Branches");
    // The sidebar lists the branches too; the window's own rows are its checkboxes.
    wait(&mut harness, "the clean-up listing", |h| h.query_by_role_and_label(Role::CheckBox, "main").is_some());
    settle(&mut harness);

    // Merged branches are preselected, except the main line.
    let checkbox = |harness: &App, name: &str| harness.get_by_role_and_label(Role::CheckBox, name).accesskit_node().toggled();
    assert_eq!(checkbox(&harness, "main"), Some(Toggled::False), "main is not preselected");
    assert_eq!(checkbox(&harness, "old-topic"), Some(Toggled::True), "old-topic is preselected");

    harness.get_by_label("Clear").click();
    harness.step();
    harness.get_by_role_and_label(Role::CheckBox, "old-topic").click();
    harness.step();
    harness.get_by_label_contains("Delete 1 branch").click();
    wait(&mut harness, "the delete confirmation", |h| shows(h, "Delete branches"));
    harness.get_by_label("Delete branches").click();
    wait(&mut harness, "old-topic to be deleted", |_| git(path, &["branch", "--list", "old-topic"]).is_empty());
    idle(&mut harness);

    // Undo from the toolbar puts it back.
    toolbar_button(&mut harness, "Undo");
    wait(&mut harness, "the undo confirmation", |h| h.state().dialog.is_some());
    harness.get_all_by_label("Undo").last().expect("confirm button").click();
    wait(&mut harness, "old-topic to be restored", |_| !git(path, &["branch", "--list", "old-topic"]).is_empty());
    idle(&mut harness);
    assert_eq!(git(path, &["rev-parse", "old-topic"]), git(path, &["rev-parse", "main"]), "old-topic is restored");
}

#[test]
fn gitflow_sets_up_develop_and_starts_a_feature() {
    let repo = repository();
    let path = repo.path();
    // The fixture's `feature` branch would block `feature/login`, so it is removed first.
    git(path, &["branch", "-D", "feature"]);
    let mut harness = open(path);
    loaded(&mut harness);

    open_repository_tool(&mut harness, path, "GitFlow", "GitFlow");
    wait(&mut harness, "the GitFlow setup form", |h| shows(h, "Set up GitFlow"));
    harness.get_by_label("Set up GitFlow").click();
    idle(&mut harness);
    assert!(git(path, &["branch", "--list", "develop"]).contains("develop"), "develop is created");
    assert_eq!(git(path, &["config", "--local", "gitflow.branch.develop"]), "develop");

    wait(&mut harness, "the feature form", |h| shows(h, "Feature name"));
    type_into(&mut harness, "Feature name", "login");
    harness.get_by_label("Start feature").click();
    idle(&mut harness);
    assert_eq!(git(path, &["branch", "--show-current"]), "feature/login");
}

#[test]
fn recover_lost_work_creates_a_branch_at_a_reset_away_commit() {
    let repo = repository();
    let path = repo.path();
    commit_file(path, "draft.txt", "idea\n", "Write the lost draft", None);
    let lost = git(path, &["rev-parse", "HEAD"]);
    git(path, &["reset", "-q", "--hard", "HEAD~1"]);
    let mut harness = open(path);
    loaded(&mut harness);

    open_repository_tool(&mut harness, path, "Recover lost work", "Recover Lost Work");
    wait(&mut harness, "the lost commit", |h| shows(h, "Write the lost draft"));
    settle(&mut harness);
    // The first match is the entry in the list.
    harness.get_all_by_label("Write the lost draft").next().expect("the lost commit's entry").click();
    settle(&mut harness);
    harness.get_by_label_contains("Create branch here").click();
    wait(&mut harness, "the branch name field", |h| h.query_by_role_and_label(Role::TextInput, "Branch name").is_some());
    type_into(&mut harness, "Branch name", "rescued");
    harness.get_by_label("Create branch").click();
    idle(&mut harness);
    assert_eq!(git(path, &["rev-parse", "rescued"]), lost, "the branch keeps the lost commit");
}

#[test]
fn blame_labels_each_line_with_its_commit_and_author() {
    let repo = repository();
    let path = repo.path();
    let head = git(path, &["rev-parse", "HEAD"]);
    // An uncommitted third line, so the file has committed and working lines.
    std::fs::write(path.join("notes.txt"), "first\nsecond\nthird\n").unwrap();
    let mut harness = open(path);
    loaded(&mut harness);

    harness.get_by_label("notes.txt").click_secondary();
    settle(&mut harness);
    harness.get_by_label_contains("  Blame").click();
    wait(&mut harness, "the blame lines", |h| shows(h, "Uncommitted · Your working changes · third"));
    settle(&mut harness);
    assert!(shows(&harness, &format!("{} · Test · second", &head[..7])), "the second line names its commit and author");
    assert!(shows(&harness, "Test · first"), "the first line names its author");
}

#[test]
fn file_history_lists_the_commits_that_changed_the_file() {
    let repo = repository();
    let path = repo.path();
    // A working change makes the file offer its history from the Changes panel.
    std::fs::write(path.join("notes.txt"), "first\nsecond\nthird\n").unwrap();
    let mut harness = open(path);
    loaded(&mut harness);

    harness.get_by_label("notes.txt").click_secondary();
    settle(&mut harness);
    harness.get_by_label_contains("File history").click();
    wait(&mut harness, "the file history", |h| shows(h, "Start the notes · Test"));
    settle(&mut harness);
    assert!(harness.state().tools.iter().any(|t| t.title() == "File history: notes.txt"));
    assert!(shows(&harness, "Add a second line · Test"), "the later change is listed");
    assert!(shows(&harness, "Start the notes · Test"), "the first commit is listed");
}

#[test]
fn search_history_finds_a_commit_by_its_message() {
    let repo = repository();
    let path = repo.path();
    let head = git(path, &["rev-parse", "HEAD"]);
    let mut harness = open(path);
    loaded(&mut harness);

    open_repository_tool(&mut harness, path, "Search history", "Search commits");
    // The search field takes focus when the window opens.
    harness.event(egui::Event::Text("second".into()));
    harness.step();
    harness.key_press(Key::Enter);
    wait(&mut harness, "the search results", |h| shows(h, "Add a second line · Test"));
    harness.get_by_label_contains("Add a second line · Test").click();
    idle(&mut harness);
    assert_eq!(selected_commit(&harness), Some(head), "choosing a result selects its commit");
}

#[test]
fn compare_with_working_files_lists_the_changed_file() {
    let repo = repository();
    let path = repo.path();
    let mut harness = open(path);
    loaded(&mut harness);

    // The first commit differs from the clean working files, through the change in the second.
    harness.get_by_label("Start the notes").click_secondary();
    settle(&mut harness);
    harness.get_by_label_contains("Compare with working files").click();
    wait(&mut harness, "the comparison", |h| h.query_by_role_and_label(Role::Button, "notes.txt").is_some());
    assert!(harness.state().tools.iter().any(|t| t.title() == "Compare"));
}

#[test]
fn command_click_cherry_picks_the_marked_commits_oldest_first() {
    let repo = repository();
    let path = repo.path();
    git(path, &["switch", "-q", "-c", "source"]);
    commit_file(path, "one.txt", "one\n", "Pick one", Some("2001-01-01T10:00:00+0000"));
    commit_file(path, "two.txt", "two\n", "Pick two", Some("2001-01-01T10:01:00+0000"));
    git(path, &["switch", "-q", "main"]);
    let mut harness = open(path);
    loaded(&mut harness);

    // Marked newest first, so the order of the clicks does not decide the order applied.
    harness.get_by_label("Pick two").click_modifiers(Modifiers::COMMAND);
    harness.step();
    harness.get_by_label("Pick one").click_modifiers(Modifiers::COMMAND);
    harness.step();
    wait(&mut harness, "the marked commits bar", |h| shows(h, "2 commits selected"));

    harness.get_by_label("Cherry-pick").click();
    wait(&mut harness, "the cherry-pick confirmation", |h| h.state().dialog.is_some());
    harness.get_all_by_label("Cherry-pick").last().expect("confirm button").click();
    idle(&mut harness);
    assert_eq!(git(path, &["log", "--format=%s", "-2"]), "Pick two\nPick one", "applied oldest first");
    assert_eq!(std::fs::read_to_string(path.join("two.txt")).unwrap(), "two\n");
}

#[test]
fn discard_all_restores_every_changed_file() {
    let repo = repository();
    let path = repo.path();
    commit_file(path, "other.txt", "other\n", "Add the other file", None);
    std::fs::write(path.join("notes.txt"), "changed\n").unwrap();
    std::fs::write(path.join("other.txt"), "changed too\n").unwrap();
    let mut harness = open(path);
    loaded(&mut harness);

    harness.get_by_label("Discard all changes…").click();
    wait(&mut harness, "the discard confirmation", |h| h.state().dialog.is_some());
    harness.get_by_label("Discard all").click();
    idle(&mut harness);
    assert_eq!(std::fs::read_to_string(path.join("notes.txt")).unwrap(), "first\nsecond\n");
    assert_eq!(std::fs::read_to_string(path.join("other.txt")).unwrap(), "other\n");
    assert!(git(path, &["status", "--porcelain"]).is_empty(), "nothing is left changed");
}

#[test]
fn create_lightweight_and_annotated_tags_from_the_graph() {
    let repo = repository();
    let path = repo.path();
    let mut harness = open(path);
    loaded(&mut harness);

    // A tag without a message is lightweight: it names the commit directly.
    harness.get_by_label("Start the notes").click_secondary();
    settle(&mut harness);
    harness.get_by_label_contains("Create tag here").click();
    wait(&mut harness, "the tag dialog", |h| h.query_by_role_and_label(Role::TextInput, "Tag name").is_some());
    type_into(&mut harness, "Tag name", "v0.1");
    harness.get_by_label("OK").click();
    idle(&mut harness);
    assert_eq!(git(path, &["cat-file", "-t", "refs/tags/v0.1"]), "commit", "v0.1 is a lightweight tag");

    // Choosing an annotated tag asks for a message.
    harness.get_by_label("Add a second line").click_secondary();
    settle(&mut harness);
    harness.get_by_label_contains("Create tag here").click();
    wait(&mut harness, "the tag dialog", |h| h.query_by_role_and_label(Role::TextInput, "Tag name").is_some());
    type_into(&mut harness, "Tag name", "v1.0");
    harness.get_by_label("Annotated, with a message").click();
    settle(&mut harness);
    type_into(&mut harness, "Tag message", "First release");
    harness.get_by_label("OK").click();
    idle(&mut harness);
    assert_eq!(git(path, &["cat-file", "-t", "refs/tags/v1.0"]), "tag", "v1.0 is annotated");
    assert_eq!(git(path, &["tag", "-l", "--format=%(contents:subject)", "v1.0"]), "First release");
}
