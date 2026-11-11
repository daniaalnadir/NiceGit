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
fn tip_history_from_the_sidebar_selects_an_older_tip_in_the_graph() {
    let repo = repository();
    let path = repo.path();
    git(path, &["switch", "-q", "feature"]);
    commit_file(path, "draft.txt", "idea\n", "Feature tip before reset", None);
    let old_tip = git(path, &["rev-parse", "HEAD"]);
    git(path, &["reset", "-q", "--hard", "HEAD~1"]);
    git(path, &["switch", "-q", "main"]);
    let mut harness = open(path);
    loaded(&mut harness);

    harness.get_by_label("feature").click_secondary();
    settle(&mut harness);
    harness.get_by_label_contains("Tip history").click();
    wait(&mut harness, "the tip history of feature", |h| h.state().tools.iter().any(|t| t.title() == "Tip history of feature"));
    wait(&mut harness, "the old tip of feature", |h| shows(h, "Feature tip before reset"));
    settle(&mut harness);
    // The first match is the entry in the list.
    harness.get_all_by_label("Feature tip before reset").next().expect("the old tip's entry").click();
    settle(&mut harness);
    assert_eq!(selected_commit(&harness).as_deref(), Some(old_tip.as_str()), "the old tip is selected in the graph");
    assert_ne!(git(path, &["rev-parse", "feature"]), old_tip, "the branch itself is not moved");
}

#[test]
fn tip_history_of_the_current_branch_resets_it_to_an_older_tip() {
    let repo = repository();
    let path = repo.path();
    commit_file(path, "draft.txt", "idea\n", "Main tip before reset", None);
    let old_tip = git(path, &["rev-parse", "HEAD"]);
    git(path, &["reset", "-q", "--hard", "HEAD~1"]);
    let mut harness = open(path);
    loaded(&mut harness);

    // The sidebar lists the branch first; the graph's label for it comes after.
    harness.get_all_by_label("main").next().expect("the sidebar's main branch").click_secondary();
    settle(&mut harness);
    harness.get_by_label_contains("Tip history").click();
    wait(&mut harness, "the tip history of main", |h| h.state().tools.iter().any(|t| t.title() == "Tip history of main"));
    wait(&mut harness, "the old tip of main", |h| shows(h, "Main tip before reset"));
    settle(&mut harness);
    harness.get_all_by_label("Main tip before reset").next().expect("the old tip's entry").click();
    settle(&mut harness);
    harness.get_by_label_contains("Reset main to here").click();
    wait(&mut harness, "the reset confirmation", |h| shows(h, "Reset branch"));
    harness.get_by_label("Reset branch").click();
    idle(&mut harness);
    assert_eq!(git(path, &["rev-parse", "main"]), old_tip, "main moves back to the older tip");
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

/// How many files the selected stash's listing has, or zero while it loads.
fn stash_files_listed(harness: &App) -> usize {
    match harness.state().repo().map(|r| r.stash_files.clone()) {
        Some(Some((_, Ok(files)))) => files.len(),
        _ => 0,
    }
}

/// Whether the open diff has a line containing `text`.
fn diff_shows(harness: &App, text: &str) -> bool {
    harness.state().repo().and_then(|r| r.diff.as_ref()).is_some_and(|diff| diff.lines.iter().any(|line| line.text.contains(text)))
}

#[test]
fn selecting_a_stash_lists_its_files() {
    let repo = repository();
    let path = repo.path();
    commit_file(path, "other.txt", "other\n", "Add the other file", None);
    std::fs::write(path.join("other.txt"), "changed other\n").unwrap();
    std::fs::write(path.join("notes.txt"), "stashed notes\n").unwrap();
    std::fs::write(path.join("scratch.txt"), "untracked\n").unwrap();
    git(path, &["stash", "push", "-u", "-q", "-m", "Mixed work"]);
    let mut harness = open(path);
    loaded(&mut harness);

    harness.get_by_label_contains("Mixed work").click();
    wait(&mut harness, "the stash's files", |h| stash_files_listed(h) == 3);
    settle(&mut harness);
    assert!(shows(&harness, "notes.txt") && shows(&harness, "other.txt"), "the tracked files are listed");
    assert!(shows(&harness, "scratch.txt"), "the untracked file is listed");
    assert!(shows(&harness, "untracked"), "the untracked file is marked");
    assert!(
        harness.state().repo().is_some_and(|r| matches!(&r.selection, Selection::Stash { file: None, .. })),
        "the whole stash is selected"
    );
}

#[test]
fn clicking_a_stash_file_shows_its_diff() {
    let repo = repository();
    let path = repo.path();
    commit_file(path, "other.txt", "other\n", "Add the other file", None);
    std::fs::write(path.join("notes.txt"), "first\nsecond\nstashed notes\n").unwrap();
    std::fs::write(path.join("other.txt"), "other\nstashed other\n").unwrap();
    git(path, &["stash", "push", "-q", "-m", "Two files"]);
    let mut harness = open(path);
    loaded(&mut harness);

    harness.get_by_label_contains("Two files").click();
    wait(&mut harness, "the stash's files", |h| stash_files_listed(h) == 2);
    wait(&mut harness, "the whole stash's diff", |h| diff_shows(h, "stashed notes") && diff_shows(h, "stashed other"));
    settle(&mut harness);

    harness
        .query_all(egui_kittest::kittest::By::new().role(egui::accesskit::Role::Button).label("other.txt"))
        .next()
        .expect("the file's row")
        .click();
    wait(&mut harness, "the other file's diff", |h| diff_shows(h, "stashed other") && !diff_shows(h, "stashed notes"));
    assert!(
        harness.state().repo().is_some_and(|r| matches!(&r.selection, Selection::Stash { file: Some(f), .. } if f == "other.txt")),
        "the file is selected"
    );

    harness.get_by_label_contains("Show all changes").click();
    wait(&mut harness, "the whole stash's diff again", |h| diff_shows(h, "stashed notes") && diff_shows(h, "stashed other"));
}

#[test]
fn restore_a_file_from_a_stash_through_the_inspector() {
    let repo = repository();
    let path = repo.path();
    commit_file(path, "other.txt", "other\n", "Add the other file", None);
    std::fs::write(path.join("notes.txt"), "first\nsecond\nstashed notes\n").unwrap();
    std::fs::write(path.join("other.txt"), "other\nstashed other\n").unwrap();
    git(path, &["stash", "push", "-q", "-m", "Two files"]);
    // Other work that the restore must leave alone.
    std::fs::write(path.join("other.txt"), "other work\n").unwrap();
    let mut harness = open(path);
    loaded(&mut harness);

    harness.get_by_label_contains("Two files").click();
    wait(&mut harness, "the stash's files", |h| stash_files_listed(h) == 2);
    settle(&mut harness);
    harness
        .query_all(egui_kittest::kittest::By::new().role(egui::accesskit::Role::Button).label("notes.txt"))
        .next()
        .expect("the file's row")
        .click_secondary();
    settle(&mut harness);
    harness.get_by_label_contains("Restore this file from the stash").click();
    wait(&mut harness, "the restore confirmation", |h| h.state().dialog.is_some());
    harness.get_all_by_label("Restore").last().expect("confirm button").click();
    idle(&mut harness);

    assert_eq!(std::fs::read_to_string(path.join("notes.txt")).unwrap(), "first\nsecond\nstashed notes\n", "the working file is restored");
    assert_eq!(git(path, &["show", ":notes.txt"]), "first\nsecond\nstashed notes", "the staged version is restored");
    assert_eq!(std::fs::read_to_string(path.join("other.txt")).unwrap(), "other work\n", "other work is untouched");
    assert_eq!(git(path, &["stash", "list"]).lines().count(), 1, "the stash is kept");
}

#[test]
fn save_unstaged_changes_as_a_patch() {
    let repo = repository();
    let path = repo.path();
    std::fs::write(path.join("notes.txt"), "first\nsecond\nthird\n").unwrap();
    std::fs::write(path.join("new.txt"), "untracked\n").unwrap();
    let saved_folder = temp_dir();
    let saved = saved_folder.path().join("saved.patch");
    let mut harness = open(path);
    loaded(&mut harness);

    crate::file_dialog::answer_next(&saved);
    settle(&mut harness);
    harness.get_by_label_contains("Save unstaged changes as a patch").click();
    wait(&mut harness, "the patch file", |_| saved.exists());

    let patch = std::fs::read_to_string(&saved).unwrap();
    assert!(patch.contains("diff --git"), "the file is a patch");
    assert!(patch.contains("diff --git a/notes.txt b/notes.txt"), "the tracked change is saved");
    assert!(patch.contains("diff --git a/new.txt b/new.txt"), "the untracked file is saved too");
    // A clean checkout of HEAD takes the patch, as it would in another place.
    let elsewhere = temp_dir();
    git(elsewhere.path(), &["clone", "-q", &path.display().to_string(), "copy"]);
    let copy = elsewhere.path().join("copy");
    git(&copy, &["apply", "--check", &saved.display().to_string()]);
    assert_eq!(std::fs::read_to_string(path.join("notes.txt")).unwrap(), "first\nsecond\nthird\n", "saving leaves the change in place");
}

#[test]
fn the_clone_dialog_opens_the_github_repository_picker() {
    let source = repository();
    let mut harness = open(source.path());
    loaded(&mut harness);

    repository_menu(&mut harness, &folder_name(source.path()), "Clone repository");
    wait(&mut harness, "the clone dialog", |h| h.state().dialog.is_some());
    harness.get_by_label("Choose a GitHub repository").click();
    wait(&mut harness, "the repository picker", |h| h.state().clone_picker.is_some());
    // The picker shows a list, or why gh cannot list one. CI may have no gh sign-in, so the states
    // are not checked beyond the picker's controls and that its read has finished.
    wait(&mut harness, "the picker's answer", |h| !shows(h, "Loading your repositories"));
    assert!(shows(&harness, "Filter repositories"), "the picker has a filter");
    assert!(shows(&harness, "Use SSH"), "the picker offers the SSH address");

    // Escape closes the picker alone; the Clone dialog stays open for its address.
    harness.key_press(Key::Escape);
    wait(&mut harness, "the picker to close", |h| h.state().clone_picker.is_none());
    assert!(harness.state().dialog.is_some(), "the Clone dialog stays open");
    assert!(shows(&harness, "Repository URL or local path"), "the address field is still there");
}

#[test]
fn create_a_pull_request_from_the_branch_menu_and_open_it_in_the_browser() {
    let repo = repository();
    let path = repo.path();
    git(path, &["remote", "add", "origin", "https://github.com/owner/repo.git"]);
    git(path, &["update-ref", "refs/remotes/origin/feature", "HEAD"]);
    git(path, &["config", "branch.feature.remote", "origin"]);
    git(path, &["config", "branch.feature.merge", "refs/heads/feature"]);
    let mut harness = open(path);
    loaded(&mut harness);
    crate::ui::changes::OPENED_URLS.with(|opened| opened.borrow_mut().clear());

    harness.get_by_label("feature").click_secondary();
    settle(&mut harness);
    harness.get_by_label_contains("Create pull request on GitHub").click();
    wait(&mut harness, "the pull request dialog", |h| h.state().pull_request.is_some());
    settle(&mut harness);

    // The title comes from the branch name: the base is still unknown, so no commits are read.
    // Nothing here depends on gh, which may be missing or signed out on the machine running it.
    let title = harness.get_by_role_and_label(Role::TextInput, "Title").accesskit_node().value();
    assert_eq!(title.as_deref(), Some("Feature"));

    // The base lookup may finish at any moment and add a message above the buttons, so the
    // click waits for the dialog to settle and for the button to stop moving.
    wait(&mut harness, "the base lookup", |h| h.state().pull_request.as_ref().is_some_and(|d| d.settled()));
    let last = std::cell::Cell::new(None);
    wait(&mut harness, "the buttons to settle", |h| {
        let rect = h.get_by_label_contains("Open in browser instead").rect();
        last.replace(Some(rect)) == Some(rect)
    });
    harness.get_by_label_contains("Open in browser instead").click();
    settle(&mut harness);
    let opened = crate::ui::changes::OPENED_URLS.with(|opened| opened.borrow().clone());
    assert_eq!(opened, vec!["https://github.com/owner/repo/compare/feature?expand=1".to_string()]);
    assert!(harness.state().pull_request.is_none(), "the dialog closes once the browser opens");
}

#[test]
fn a_gitlab_merge_request_says_how_to_fix_a_missing_glab() {
    let repo = repository();
    let path = repo.path();
    git(path, &["remote", "add", "origin", "https://gitlab.com/group/proj.git"]);
    git(path, &["update-ref", "refs/remotes/origin/feature", "HEAD"]);
    git(path, &["config", "branch.feature.remote", "origin"]);
    git(path, &["config", "branch.feature.merge", "refs/heads/feature"]);
    let mut harness = open(path);
    loaded(&mut harness);
    // glab is looked for at a file that does not exist, as on a Mac without it. Tests never
    // change PATH, so this names the program the app runs instead.
    harness.state_mut().glab = nicegit_core::gitlab::GlabProgram::Given(std::path::PathBuf::from("/nonexistent/nicegit-test/glab"));
    harness.get_by_label("feature").click_secondary();
    settle(&mut harness);
    harness.get_by_label_contains("Create merge request on GitLab").click();
    wait(&mut harness, "the merge request dialog", |h| h.state().pull_request.is_some());
    wait(&mut harness, "the base lookup", |h| h.state().pull_request.as_ref().is_some_and(|d| d.settled()));
    settle(&mut harness);

    assert!(harness.query_by_label_contains("GitLab CLI (glab) is not installed").is_some(), "the dialog says glab is missing");
    assert!(harness.query_by_label_contains("glab auth login --hostname gitlab.com").is_some(), "and how to sign in once it is installed");
    assert!(harness.query_by_label_contains("Create merge request").is_some(), "the dialog is for a merge request");
}

/// The Launchpad reads each open repository's pull requests through `gh`, and merge requests through
/// `glab`. With neither to run, each repository says why it is not available, and no network call is
/// made: both repositories stop at their missing programs.
#[test]
fn the_launchpad_says_why_each_repository_is_not_available() {
    let github = repository();
    git(github.path(), &["remote", "add", "origin", "https://github.com/acme/app.git"]);
    let gitlab = repository();
    git(gitlab.path(), &["remote", "add", "origin", "https://gitlab.com/group/proj.git"]);
    let mut harness = open(github.path());
    loaded(&mut harness);
    harness.state_mut().open(gitlab.path().to_path_buf());
    wait(&mut harness, "the second repository to load", |h| h.state().repos.len() == 2 && h.state().busy.is_none());
    loaded(&mut harness);

    let missing = nicegit_core::github::GhProgram::Given(std::path::PathBuf::from("/nonexistent/nicegit-test/gh"));
    let missing_glab = nicegit_core::gitlab::GlabProgram::Given(std::path::PathBuf::from("/nonexistent/nicegit-test/glab"));
    let paths = vec![github.path().to_path_buf(), gitlab.path().to_path_buf()];
    let launchpad = crate::tools::launchpad::LaunchpadWindow::new(paths, Vec::new(), Vec::new(), Vec::new())
        .with_program(missing)
        .with_glab_program(missing_glab);
    harness.state_mut().open_tool(Box::new(launchpad));
    // Each repository answers on its own; "Not available" shows with the first, so the wait is
    // for both answers and for nothing still loading.
    wait(&mut harness, "both repositories' answers", |h| {
        shows(h, "The GitHub CLI (gh) is not installed") && shows(h, "The GitLab CLI (glab) is not installed") && !shows(h, "Loading")
    });
    settle(&mut harness);

    assert!(shows(&harness, "Needs your review (0)"), "the sections are listed, empty");
    assert!(shows(&harness, "Yours (0)") && shows(&harness, "Other open (0)"), "every section is listed");
    assert!(shows(&harness, "The GitHub CLI (gh) is not installed"), "the GitHub repository says gh is missing");
    assert!(shows(&harness, "The GitLab CLI (glab) is not installed"), "the GitLab repository says glab is missing");
    assert!(shows(&harness, "glab auth login --hostname gitlab.com"), "and how to sign in once it is installed");
    assert!(!shows(&harness, "Loading"), "nothing is still loading");
}

/// A Check out from the Launchpad for a repository that is not shown switches to it, and the
/// Check out pull request dialog opens once it is shown. Nothing is fetched until its button is pressed.
#[test]
fn a_launchpad_check_out_switches_to_its_repository_and_opens_the_dialog() {
    let first = repository();
    let second = repository();
    let mut harness = open(first.path());
    loaded(&mut harness);
    harness.state_mut().open(second.path().to_path_buf());
    wait(&mut harness, "the second repository to load", |h| h.state().repos.len() == 2 && h.state().busy.is_none());
    loaded(&mut harness);

    let request = crate::tools::CheckOutRequest {
        path: first.path().to_path_buf(),
        remote: "origin".into(),
        number: 7,
        title: "Add the review file".into(),
        author: "octocat".into(),
    };
    harness.state_mut().check_out_from_launchpad(request);
    wait(
        &mut harness,
        "the check out dialog",
        |h| matches!(&h.state().dialog, Some(crate::ui::dialogs::Dialog::CheckOutPullRequest { local, .. }) if local == "pr/7-add-the-review-file"),
    );
    let shown = harness.state().repo().and_then(|r| r.path.canonicalize().ok());
    assert_eq!(shown, first.path().canonicalize().ok(), "the repository of the pull request is the one shown");
    assert!(harness.state().pending_check_out.is_none(), "nothing is left waiting");
}

// MARK: External diff and merge tools

/// Whether a Changes panel row for `file` is on screen.
fn change_row_shown(harness: &App, file: &str) -> bool {
    harness.query_by_label(file).is_some()
}

/// Whether the menu item whose label contains `text` is disabled.
fn menu_item_disabled(harness: &App, text: &str) -> bool {
    harness.get_by_label_contains(text).accesskit_node().is_disabled()
}

/// Opens the Changes panel menu for `file`, once its row is on screen.
fn open_change_menu(harness: &mut App, file: &str) {
    wait(harness, "the row for the file", |h| change_row_shown(h, file));
    harness.get_by_label(file).click_secondary();
    settle(harness);
}

#[cfg(unix)]
#[test]
fn changes_menu_opens_a_changed_file_in_the_configured_diff_tool() {
    let repo = repository();
    let path = repo.path();
    std::fs::write(path.join("notes.txt"), "first\nsecond\nthird\n").unwrap();
    // The fake tool records the working file it was given, so the test can see what was opened.
    let scratch = temp_dir();
    let out = scratch.path().join("recorded.txt");
    let command = format!("printf '%s\\n' \"$REMOTE\" > '{}'", out.display());
    git(path, &["config", "diff.tool", "fake"]);
    git(path, &["config", "difftool.fake.cmd", command.as_str()]);

    let mut harness = open(path);
    loaded(&mut harness);
    open_change_menu(&mut harness, "notes.txt");
    wait(&mut harness, "the diff tool item", |h| shows(h, "Open in fake"));
    assert!(!menu_item_disabled(&harness, "Open in fake"), "the item is enabled once a tool is set");

    harness.get_by_label_contains("Open in fake").click();
    wait(&mut harness, "the diff tool to run", |_| out.exists());
    let recorded = std::fs::read_to_string(&out).unwrap();
    assert!(recorded.contains("notes.txt"), "the tool was given the changed file: {recorded}");
}

#[test]
fn changes_menu_disables_the_diff_tool_item_when_none_is_set() {
    let repo = repository();
    let path = repo.path();
    std::fs::write(path.join("notes.txt"), "first\nsecond\nthird\n").unwrap();
    // An empty value in the repository overrides a diff tool set globally, so the test does not
    // depend on the machine's own configuration.
    git(path, &["config", "diff.tool", ""]);

    let mut harness = open(path);
    loaded(&mut harness);
    open_change_menu(&mut harness, "notes.txt");
    wait(&mut harness, "the diff tool item", |h| shows(h, "Open in diff tool"));
    assert!(menu_item_disabled(&harness, "Open in diff tool"), "the item is disabled without a tool");
}

#[test]
fn saving_a_copied_version_in_the_conflict_editor_resolves_the_file() {
    let repo = repository();
    let path = repo.path();
    git(path, &["switch", "-q", "feature"]);
    std::fs::write(path.join("notes.txt"), "first\nfrom feature\n").unwrap();
    git(path, &["commit", "-qam", "Change notes on feature"]);
    git(path, &["switch", "-q", "main"]);
    let merge = Command::new("git").args(["merge", "--no-edit", "feature"]).current_dir(path).output().unwrap();
    assert!(!merge.status.success(), "the merge conflicts");

    let mut harness = open(path);
    loaded(&mut harness);
    open_change_menu(&mut harness, "notes.txt");
    wait(&mut harness, "the resolve item", |h| shows(h, "Resolve conflict…"));
    // The row's icon button has the same label; the menu item is drawn after it, so it comes last.
    let matches = harness.query_all_by_label_contains("Resolve conflict…").count();
    harness.get_all_by_label_contains("Resolve conflict…").nth(matches - 1).expect("the menu item").click();
    wait(&mut harness, "the conflict editor", |h| h.state().tools.iter().any(|t| t.id().starts_with("conflict:")));
    wait(&mut harness, "the versions to load", |h| shows(h, "Copy into result"));
    settle(&mut harness);

    // Copying the current version leaves the result with edits, so saving runs with unsaved text.
    // The current version is the second of the three copy buttons.
    harness.get_all_by_label("Copy into result").nth(1).expect("the current version's copy button").click();
    wait(&mut harness, "the copy confirmation", |h| shows(h, "Replace the result with the current version?"));
    harness.get_by_label("Use version").click();
    settle(&mut harness);
    wait(&mut harness, "the save button to be enabled", |h| {
        h.query_by_label("Save and mark resolved").is_some_and(|button| !button.accesskit_node().is_disabled())
    });
    harness.get_by_label("Save and mark resolved").click();
    wait(&mut harness, "the conflict to be resolved", |h| {
        h.state().snapshot().is_some_and(|s| !s.status.iter().any(|e| e.kind == nicegit_core::StatusKind::Conflicted))
    });
    idle(&mut harness);
    assert_eq!(std::fs::read_to_string(path.join("notes.txt")).unwrap(), "first\nsecond\n");
}

#[cfg(unix)]
#[test]
fn resolve_in_the_merge_tool_resolves_the_conflict() {
    let repo = repository();
    let path = repo.path();
    git(path, &["switch", "-q", "feature"]);
    std::fs::write(path.join("notes.txt"), "first\nfrom feature\n").unwrap();
    git(path, &["commit", "-qam", "Change notes on feature"]);
    git(path, &["switch", "-q", "main"]);
    let merge = std::process::Command::new("git").args(["merge", "--no-edit", "feature"]).current_dir(path).output().unwrap();
    assert!(!merge.status.success(), "the merge conflicts");

    // The fake merge tool writes the result, and trustExitCode lets Git mark the file resolved.
    git(path, &["config", "merge.tool", "fake"]);
    git(path, &["config", "mergetool.fake.cmd", "printf 'resolved\\n' > \"$MERGED\""]);
    git(path, &["config", "mergetool.fake.trustExitCode", "true"]);
    git(path, &["config", "mergetool.keepBackup", "false"]);

    let mut harness = open(path);
    loaded(&mut harness);
    open_change_menu(&mut harness, "notes.txt");
    wait(&mut harness, "the merge tool item", |h| shows(h, "Resolve in fake"));
    assert!(!menu_item_disabled(&harness, "Resolve in fake"), "the item is enabled once a tool is set");

    harness.get_by_label_contains("Resolve in fake").click();
    wait(&mut harness, "the conflict to be resolved", |h| {
        h.state().snapshot().is_some_and(|s| !s.status.iter().any(|e| e.kind == nicegit_core::StatusKind::Conflicted))
    });
    idle(&mut harness);
    assert_eq!(std::fs::read_to_string(path.join("notes.txt")).unwrap(), "resolved\n");
}

#[test]
fn git_settings_adds_lists_and_removes_a_local_setting() {
    let repo = repository();
    let path = repo.path();
    let mut harness = open(path);
    loaded(&mut harness);

    open_repository_tool(&mut harness, path, "Git settings", "Git Settings");
    wait(&mut harness, "the add row", |h| h.query_by_role_and_label(Role::TextInput, "Setting name").is_some());

    harness.get_by_role_and_label(Role::TextInput, "Setting name").focus();
    harness.step();
    harness.get_by_role_and_label(Role::TextInput, "Setting name").type_text("nicegit.test");
    harness.step();
    harness.get_by_role_and_label(Role::TextInput, "Setting value").focus();
    harness.step();
    harness.get_by_role_and_label(Role::TextInput, "Setting value").type_text("1");
    harness.step();
    // `--get-all` exits 1 while the key is unset, so the value is read without asserting success.
    let local_value = |dir: &Path| {
        let output = Command::new("git")
            .args(["config", "--local", "--get-all", "nicegit.test"])
            .current_dir(dir)
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .output()
            .expect("run git");
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    };
    // The button waits while the app is busy, which on a slow runner can outlast one click.
    click_until(
        &mut harness,
        "the new setting to be written",
        |h| {
            if let Some(button) = h.query_by_role_and_label(Role::Button, "Add") {
                button.click();
            }
        },
        |_| local_value(path) == "1",
    );
    idle(&mut harness);
    wait(&mut harness, "the new setting to be listed", |h| h.query_by_label("Remove nicegit.test").is_some());

    // The new setting is last in the list, below the visible rows, so the filter brings it up.
    harness.get_by_role_and_label(Role::TextInput, "Filter settings").focus();
    harness.step();
    harness.get_by_role_and_label(Role::TextInput, "Filter settings").type_text("nicegit");
    settle(&mut harness);
    harness.get_by_role_and_label(Role::Button, "Remove nicegit.test").click();
    wait(&mut harness, "the removal confirmation", |h| h.query_by_role_and_label(Role::Button, "Remove").is_some());
    click_until(
        &mut harness,
        "the setting to be removed",
        |h| {
            if let Some(button) = h.query_by_role_and_label(Role::Button, "Remove") {
                button.click();
            }
        },
        |_| !git(path, &["config", "--local", "--list"]).contains("nicegit.test="),
    );
    idle(&mut harness);
    wait(&mut harness, "the removed setting to leave the list", |h| h.query_by_label("Remove nicegit.test").is_none());
}

/// Clicks with `click` every few frames until `done` holds, for buttons that stay disabled while
/// the app is busy. Fails after 30 seconds.
fn click_until(
    harness: &mut Harness<'static, NiceGitApp>,
    what: &str,
    click: impl Fn(&Harness<'static, NiceGitApp>),
    done: impl Fn(&Harness<'static, NiceGitApp>) -> bool,
) {
    let start = std::time::Instant::now();
    while !done(harness) {
        click(harness);
        harness.run_steps(5);
        assert!(start.elapsed() < std::time::Duration::from_secs(30), "timed out waiting for {what}");
    }
}

#[test]
fn a_pull_request_detail_window_shows_its_checks_conversation_and_files() {
    use crate::tools::pull_request_detail::PullRequestDetailWindow;
    use nicegit_core::github::{decode_pull_request_detail, GitHubRepository};

    let repo = repository();
    let mut harness = open(repo.path());
    loaded(&mut harness);
    settle(&mut harness);

    // The answer is decoded from a fixture and injected, so the window never asks gh for it.
    let json = r#"{"number": 12, "title": "Add the status bar", "body": "Shows the pull request.",
        "author": {"login": "octocat"}, "state": "OPEN", "isDraft": false, "baseRefName": "main",
        "headRefName": "status-bar", "headRefOid": "0123456789abcdef0123456789abcdef01234567",
        "url": "https://github.com/acme/app/pull/12",
        "createdAt": "2026-01-02T03:04:05Z", "updatedAt": "2026-01-02T03:04:05Z", "reviewDecision": null,
        "mergeable": "MERGEABLE", "additions": 10, "deletions": 2, "changedFiles": 1, "commits": [],
        "files": [{"path": "src/status_bar.rs", "additions": 10, "deletions": 2}],
        "reviews": [], "comments": [{"author": {"login": "carol"}, "body": "Please rename the helper.",
        "createdAt": "2026-01-02T04:00:00Z"}],
        "statusCheckRollup": [{"__typename": "CheckRun", "name": "Integration suite", "status": "COMPLETED",
        "conclusion": "SUCCESS", "detailsUrl": "https://github.com/acme/app/actions/runs/9"}], "labels": []}"#;
    let github = GitHubRepository { host: "github.com".into(), owner: "acme".into(), name: "app".into() };
    let detail = decode_pull_request_detail(json, &github, 12).expect("the fixture decodes");
    harness.state_mut().open_tool(Box::new(PullRequestDetailWindow::loaded(github, 12, detail)));
    wait(&mut harness, "the detail window", |h| h.state().tools.iter().any(|t| t.title() == "Pull request #12"));
    settle(&mut harness);

    assert!(shows(&harness, "Add the status bar"), "the title is shown");
    harness.get_by_label_contains("Checks (").click();
    settle(&mut harness);
    assert!(shows(&harness, "Integration suite"), "the check is listed");
    assert!(shows(&harness, "Open check"), "the check links to its run");
    harness.get_by_label_contains("Conversation (").click();
    settle(&mut harness);
    assert!(shows(&harness, "Please rename the helper."), "the comment is shown");
    harness.get_by_label_contains("Files (").click();
    settle(&mut harness);
    assert!(shows(&harness, "src/status_bar.rs"), "the changed file is listed");
}

/// A pull request as `gh pr view` prints it, for the review and merge tests. The draft flag and the
/// mergeability word are the ones the tests vary. The author is not the signed-in user.
fn pull_request_view_json(draft: bool, mergeable: &str) -> String {
    format!(
        r#"{{"number": 12, "title": "Add the status bar", "body": "", "author": {{"login": "octocat"}},
        "state": "OPEN", "isDraft": {draft}, "baseRefName": "main", "headRefName": "status-bar",
        "headRefOid": "0123456789abcdef0123456789abcdef01234567", "url": "https://github.com/acme/app/pull/12",
        "createdAt": "2026-01-02T03:04:05Z", "updatedAt": "2026-01-02T03:04:05Z", "reviewDecision": null,
        "mergeable": "{mergeable}", "additions": 10, "deletions": 2, "changedFiles": 1, "commits": [],
        "files": [{{"path": "src/status_bar.rs", "additions": 10, "deletions": 2}}], "reviews": [], "comments": [],
        "statusCheckRollup": [], "labels": []}}"#
    )
}

/// Opens the detail window of pull request 12 with the fixture, read as the signed-in user "daniaal".
/// Nothing reaches GitHub: the detail is decoded here and handed to the window.
fn open_pull_request_window(harness: &mut App, draft: bool) {
    use crate::tools::pull_request_detail::PullRequestDetailWindow;
    use nicegit_core::github::{decode_pull_request_detail, GitHubRepository};

    let github = GitHubRepository { host: "github.com".into(), owner: "acme".into(), name: "app".into() };
    let detail = decode_pull_request_detail(&pull_request_view_json(draft, "MERGEABLE"), &github, 12).expect("the fixture decodes");
    harness.state_mut().open_tool(Box::new(PullRequestDetailWindow::loaded_as(github, 12, detail, "daniaal")));
    wait(harness, "the detail window", |h| h.state().tools.iter().any(|t| t.title() == "Pull request #12"));
    settle(harness);
}

#[test]
fn merge_stays_disabled_for_a_draft_pull_request() {
    let repo = repository();
    let mut harness = open(repo.path());
    loaded(&mut harness);
    settle(&mut harness);
    open_pull_request_window(&mut harness, true);

    assert!(shows(&harness, "Add the status bar"), "the title is shown");
    let merge = harness.get_by_label_contains("Merge…");
    assert!(merge.accesskit_node().is_disabled(), "a draft cannot be merged");
}

#[test]
fn review_and_merge_confirmations_name_the_repository_number_and_title() {
    let repo = repository();
    let mut harness = open(repo.path());
    loaded(&mut harness);
    settle(&mut harness);
    open_pull_request_window(&mut harness, false);

    harness.get_by_label_contains("Review…").click();
    settle(&mut harness);
    assert!(shows(&harness, "Review pull request"), "the review confirmation is open");
    assert!(shows(&harness, "github.com/acme/app"), "the repository is named");
    assert!(shows(&harness, "#12"), "the number is named");
    assert!(shows(&harness, "Add the status bar"), "the title is named");
    harness.get_by_role_and_label(Role::Button, "Cancel").click();
    settle(&mut harness);
    assert!(!shows(&harness, "Submit review"), "cancel closes the review without running it");

    harness.get_by_label_contains("Merge…").click();
    settle(&mut harness);
    assert!(shows(&harness, "Merge pull request"), "the merge confirmation is open");
    assert!(shows(&harness, "github.com/acme/app"), "the repository is named for a merge");
    assert!(shows(&harness, "Squash and merge"), "the merge methods are offered");
    harness.get_by_role_and_label(Role::Button, "Cancel").click();
    settle(&mut harness);
    assert!(!shows(&harness, "Squash and merge"), "cancel closes the merge without running it");
}

/// Sets one of the graph's Show menu items on or off, opening the menu first when it is closed.
/// The menu stays open while its items are toggled.
fn set_graph_show(harness: &mut App, item: &str, on: bool) {
    if harness.query_by_role_and_label(Role::CheckBox, item).is_none() {
        harness.get_by_label_contains("Show…").click();
        wait(harness, "the Show menu", |h| h.query_by_role_and_label(Role::CheckBox, item).is_some());
    }
    let wanted = if on { Toggled::True } else { Toggled::False };
    if harness.get_by_role_and_label(Role::CheckBox, item).accesskit_node().toggled() != Some(wanted) {
        harness.get_by_role_and_label(Role::CheckBox, item).click();
    }
    settle(harness);
}

#[test]
fn the_show_menu_hides_and_restores_tag_labels_in_the_graph() {
    let repo = repository();
    let path = repo.path();
    commit_file(path, "notes.txt", "one\n", "Tagged commit", None);
    git(path, &["tag", "v1.0"]);
    let mut harness = open(path);
    loaded(&mut harness);
    wait(&mut harness, "the tag label in the graph", |h| h.query_by_label("Tag v1.0").is_some());

    set_graph_show(&mut harness, "Tag labels", false);
    wait(&mut harness, "the tag label to go", |h| h.query_by_label("Tag v1.0").is_none());
    assert!(!harness.state().settings.show_tag_labels, "the choice is kept in the settings");
    assert!(harness.query_by_role_and_label(Role::Button, "Tagged commit").is_some(), "the commit stays in the graph");

    set_graph_show(&mut harness, "Tag labels", true);
    wait(&mut harness, "the tag label back", |h| h.query_by_label("Tag v1.0").is_some());
}

/// Whether the graph draws a stash row, which is a button named by the stash's reference.
fn stash_row_shown(harness: &App) -> bool {
    harness.query_all(egui_kittest::kittest::By::new().role(Role::Button).label_contains("stash@{0}")).next().is_some()
}

#[test]
fn the_show_menu_draws_a_stash_row_that_selects_the_stash() {
    let repo = repository();
    let path = repo.path();
    commit_file(path, "other.txt", "other\n", "Add the other file", None);
    std::fs::write(path.join("other.txt"), "changed other\n").unwrap();
    git(path, &["stash", "push", "-q", "-m", "Graph stash"]);
    let mut harness = open(path);
    loaded(&mut harness);
    settle(&mut harness);
    assert!(!stash_row_shown(&harness), "stashes are not drawn until asked");

    set_graph_show(&mut harness, "Stashes", true);
    wait(&mut harness, "the stash row in the graph", stash_row_shown);
    settle(&mut harness);
    harness
        .query_all(egui_kittest::kittest::By::new().role(Role::Button).label_contains("stash@{0}"))
        .next()
        .expect("the stash row")
        .click();
    wait(&mut harness, "the stash's files", |h| stash_files_listed(h) == 1);
    settle(&mut harness);
    assert!(
        harness.query_all(egui_kittest::kittest::By::new().role(Role::Button).label("other.txt")).next().is_some(),
        "the stash's file is listed in the inspector"
    );
    assert!(
        harness.state().repo().is_some_and(|r| matches!(&r.selection, Selection::Stash { file: None, .. })),
        "the whole stash is selected, as the sidebar selects it"
    );
}
