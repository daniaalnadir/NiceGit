//! Interface tests for the remaining details of Mac app features, built on the helpers in
//! `ui_tests`.

#![allow(unused_imports)]

use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

use egui::accesskit::Role;
use egui::{Key, Modifiers};
use egui_kittest::kittest::{By, NodeT, Queryable};
use egui_kittest::Harness;
use egui_phosphor::regular as icon;
use nicegit_core::{Operation, StatusKind};

use crate::app::{NiceGitApp, Selection};
use crate::ui_tests::*;

type App = Harness<'static, NiceGitApp>;

/// Commits a change to `file` with `message`.
fn commit_file(dir: &Path, file: &str, contents: &str, message: &str) {
    std::fs::write(dir.join(file), contents).unwrap();
    git(dir, &["add", "--", file]);
    git(dir, &["commit", "-q", "-m", message]);
}

/// Whether any widget's label contains `text`.
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

/// The operation Git has in progress, as the last snapshot read it.
fn operation(harness: &App) -> Option<Operation> {
    harness.state().snapshot().and_then(|s| s.operation)
}

/// Whether the last snapshot lists a conflicted file.
fn has_conflicts(harness: &App) -> bool {
    harness.state().snapshot().is_some_and(|s| s.status.iter().any(|entry| entry.kind == StatusKind::Conflicted))
}

/// Opens the conflict editor for `notes.txt` from the operation banner, takes the version
/// labelled `take`, and confirms it. The window is closed afterwards.
fn take_version_from_the_banner(harness: &mut App, take: &str) {
    harness.get_by_label_contains("  notes.txt").click();
    wait(harness, "the conflict editor", |h| h.state().tools.iter().any(|t| t.id().starts_with("conflict:")));
    wait(harness, "the conflict to load", |h| shows(h, take));
    // Wait until the window has finished sizing itself, so the button stays where it was found.
    let mut last = harness.get_by_label_contains(take).rect();
    for _ in 0..30 {
        harness.step();
        let now = harness.get_by_label_contains(take).rect();
        if now == last {
            break;
        }
        last = now;
    }
    harness.get_by_label_contains(take).click();
    settle(harness);
    harness.get_by_label("Take version").click();
    idle(harness);
    harness.state_mut().tools.clear();
    settle(harness);
}

/// A repository whose `side` branch changes line two of `notes.txt` differently from main's
/// second commit, so rebasing main onto it conflicts.
fn rebase_conflict_repository() -> tempfile::TempDir {
    let repo = repository();
    let path = repo.path();
    git(path, &["switch", "-q", "-c", "side", "HEAD~1"]);
    commit_file(path, "notes.txt", "first\nside\n", "Side notes");
    git(path, &["switch", "-q", "main"]);
    repo
}

/// Rebases main onto `side` from the sidebar, which Git stops with a conflict.
fn stop_a_rebase_on_a_conflict(harness: &mut App) {
    harness.get_by_label("side").click_secondary();
    settle(harness);
    harness.get_by_label_contains("Rebase main onto this").click();
    wait(harness, "the rebase confirmation", |h| shows(h, "Rebase main onto side?"));
    settle(harness);
    harness.get_by_role_and_label(Role::Button, "Rebase").click();
    idle(harness);
    wait(harness, "the stopped rebase", |h| operation(h) == Some(Operation::Rebase));
    wait(harness, "the operation banner", |h| h.query_by_label("Abort…").is_some());
    settle(harness);
}

/// A repository whose `loud` branch changes line two of `notes.txt` to something main's second
/// commit does not, so cherry-picking it onto main conflicts.
fn cherry_pick_conflict_repository() -> tempfile::TempDir {
    let repo = repository();
    let path = repo.path();
    git(path, &["switch", "-q", "-c", "loud", "HEAD~1"]);
    commit_file(path, "notes.txt", "first\nSHOUT\n", "Shout the line");
    git(path, &["switch", "-q", "main"]);
    repo
}

/// Cherry-picks "Shout the line" onto main from its graph menu, which Git stops with a conflict.
fn stop_a_cherry_pick_on_a_conflict(harness: &mut App) {
    harness.get_by_label("Shout the line").click_secondary();
    settle(harness);
    harness.get_by_label_contains("Cherry-pick").click();
    wait(harness, "the cherry-pick confirmation", |h| shows(h, "Cherry-pick this commit?"));
    settle(harness);
    harness.get_all_by_label("Cherry-pick").last().expect("confirm button").click();
    idle(harness);
    wait(harness, "the stopped cherry-pick", |h| operation(h) == Some(Operation::CherryPick));
    wait(harness, "the operation banner", |h| h.query_by_label("Abort…").is_some());
    settle(harness);
}

/// A repository whose main line changes "second" again after "Add a second line", so reverting
/// that commit conflicts with the later change.
fn revert_conflict_repository() -> tempfile::TempDir {
    let repo = repository();
    commit_file(repo.path(), "notes.txt", "first\nSECOND\n", "Capitalise the second line");
    repo
}

/// Reverts "Add a second line" from its graph menu, which Git stops with a conflict.
fn stop_a_revert_on_a_conflict(harness: &mut App) {
    harness.get_by_label("Add a second line").click_secondary();
    settle(harness);
    harness.get_by_label_contains("Revert").click();
    wait(harness, "the revert confirmation", |h| h.state().dialog.is_some());
    harness.get_by_label("Revert").click();
    idle(harness);
    wait(harness, "the stopped revert", |h| operation(h) == Some(Operation::Revert));
    wait(harness, "the operation banner", |h| h.query_by_label("Abort…").is_some());
    settle(harness);
}

/// Aborts the stopped operation from the banner and confirms.
fn abort_from_the_banner(harness: &mut App, name: &str) {
    harness.get_by_label("Abort…").click();
    wait(harness, "the abort confirmation", |h| shows(h, &format!("Abort the {name}?")));
    settle(harness);
    harness.get_by_role_and_label(Role::Button, "Abort").click();
    idle(harness);
    wait(harness, "the operation to end", |h| operation(h).is_none());
    settle(harness);
}

#[test]
fn bisect_skip_marks_a_commit_untestable_and_git_records_it() {
    let repo = repository();
    let path = repo.path();
    commit_file(path, "bug.txt", "one\n", "Step one");
    commit_file(path, "bug.txt", "two\n", "Step two");
    commit_file(path, "bug.txt", "three\n", "Step three");

    let mut harness = open(path);
    loaded(&mut harness);
    harness.get_by_label("Add a second line").click_secondary();
    settle(&mut harness);
    harness.get_by_label_contains("Start bisect: this commit is good").click();
    wait(&mut harness, "the bisect window", |h| h.state().tools.iter().any(|t| t.id() == "bisect"));
    wait(&mut harness, "the start button", |h| shows(h, "Start bisect"));
    settle(&mut harness);
    harness.get_by_label("Start bisect").click();
    idle(&mut harness);

    // The bar above the graph offers the verdicts for the commit under test.
    wait(&mut harness, "the bisect bar", |h| h.query_by_role_and_label(Role::Button, "Skip").is_some());
    settle(&mut harness);
    let tested = git(path, &["rev-parse", "HEAD"]);
    harness.get_by_role_and_label(Role::Button, "Skip").click();
    idle(&mut harness);

    wait(&mut harness, "the skip to be recorded", |_| {
        git(path, &["bisect", "log"]).lines().any(|line| line.contains("skip") && line.contains(&tested))
    });
    assert_ne!(git(path, &["rev-parse", "HEAD"]), tested, "Git moves on to another commit to test");
    wait(&mut harness, "the window to count the skip", |h| shows(h, "1 skipped so far"));
}

/// Every refresh reads the bisect again. The bar keeps the state it had until the new read
/// arrives, so its Skip button does not vanish under a click.
#[test]
fn bisect_bar_keeps_its_buttons_across_refreshes() {
    let repo = repository();
    let path = repo.path();
    commit_file(path, "bug.txt", "one\n", "Step one");
    commit_file(path, "bug.txt", "two\n", "Step two");
    commit_file(path, "bug.txt", "three\n", "Step three");
    git(path, &["bisect", "start", "HEAD", "HEAD~3"]);
    let mut harness = open(path);
    loaded(&mut harness);
    wait(&mut harness, "the bisect bar", |h| h.query_by_role_and_label(Role::Button, "Skip").is_some());

    for refresh in 0..5 {
        harness.state_mut().load(false);
        for frame in 0..30 {
            harness.step();
            assert!(
                harness.query_by_role_and_label(Role::Button, "Skip").is_some(),
                "the Skip button stayed through refresh {refresh}, frame {frame}"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        idle(&mut harness);
    }
}

#[test]
fn interactive_rebase_warns_about_commits_already_on_a_remote() {
    let repo = repository();
    let path = repo.path();
    let remote = temp_dir();
    git(remote.path(), &["init", "-q", "--bare"]);
    git(path, &["remote", "add", "origin", &remote.path().display().to_string()]);
    git(path, &["push", "-q", "-u", "origin", "main"]);
    // "Add a second line" is now on the remote; this commit is only local.
    commit_file(path, "local.txt", "local\n", "Keep this local");
    let before = git(path, &["rev-parse", "HEAD"]);
    let mut harness = open(path);
    loaded(&mut harness);

    harness.get_by_label("Start the notes").click_secondary();
    settle(&mut harness);
    harness.get_by_label_contains("Interactive rebase from here").click();
    wait(&mut harness, "the rebase plan", |h| shows(h, "Rewrite 2 commits on main"));
    settle(&mut harness);
    assert!(shows(&harness, "1 of these commit is already on a remote"), "the warning names the published commit");
    assert!(shows(&harness, "force-push"), "the warning says a rewrite needs a force-push");
    assert!(shows(&harness, "Already on a remote"), "the legend marks the published row");

    harness.get_by_label("Cancel").click();
    wait(&mut harness, "the rebase window to close", |h| h.state().tools.iter().all(|t| t.title() != "Interactive rebase"));
    assert_eq!(git(path, &["rev-parse", "HEAD"]), before, "cancelling rewrites nothing");
}

#[test]
fn command_click_squashes_the_marked_commits_through_the_rebase_window() {
    let repo = repository();
    let path = repo.path();
    commit_file(path, "one.txt", "one\n", "Squash one");
    commit_file(path, "two.txt", "two\n", "Squash two");
    commit_file(path, "three.txt", "three\n", "Squash three");
    let count = |path: &Path| git(path, &["rev-list", "--count", "HEAD"]).trim().parse::<usize>().expect("commit count");
    let before = count(path);
    let mut harness = open(path);
    loaded(&mut harness);

    harness.get_by_label("Squash one").click_modifiers(Modifiers::COMMAND);
    harness.step();
    harness.get_by_label("Squash three").click_modifiers(Modifiers::COMMAND);
    harness.step();
    wait(&mut harness, "the marked commits bar", |h| shows(h, "2 commits selected"));

    harness.get_by_label("Squash…").click();
    wait(&mut harness, "the squash window", |h| h.state().tools.iter().any(|t| t.title() == "Squash commits"));
    wait(&mut harness, "the squash plan", |h| shows(h, "Rewrite 3 commits on main"));
    settle(&mut harness);
    assert!(shows(&harness, "Selected commits were moved"), "the note warns that the commits moved");
    harness.get_by_role_and_label(Role::Button, "Rewrite commits").click();
    idle(&mut harness);
    wait(&mut harness, "the squashed history", |_| count(path) == before - 1);
    let log = git(path, &["log", "--format=%s"]);
    assert!(log.starts_with("Squash two\nSquash one\nAdd a second line"), "{log}");
    assert!(git(path, &["status", "--porcelain"]).is_empty(), "the working tree is clean");
}

#[test]
fn command_click_reverts_the_marked_commits() {
    let repo = repository();
    let path = repo.path();
    commit_file(path, "one.txt", "one\n", "Undo one");
    commit_file(path, "two.txt", "two\n", "Keep two");
    commit_file(path, "three.txt", "three\n", "Undo three");
    let before = git(path, &["rev-parse", "HEAD"]);
    let mut harness = open(path);
    loaded(&mut harness);

    harness.get_by_label("Undo one").click_modifiers(Modifiers::COMMAND);
    harness.step();
    harness.get_by_label("Undo three").click_modifiers(Modifiers::COMMAND);
    harness.step();
    wait(&mut harness, "the marked commits bar", |h| shows(h, "2 commits selected"));

    harness.get_by_role_and_label(Role::Button, "Revert…").click();
    wait(&mut harness, "the revert confirmation", |h| shows(h, "Revert 2 commits?"));
    harness.get_by_role_and_label(Role::Button, "Revert").click();
    idle(&mut harness);
    wait(&mut harness, "the two revert commits", |_| git(path, &["rev-list", "--count", &format!("{before}..HEAD")]).trim() == "2");
    let log = git(path, &["log", "--format=%s", &format!("{before}..HEAD")]);
    // Newest first in the log: the revert of "Undo three" was made first, so "Undo one" is on top.
    assert_eq!(log.lines().collect::<Vec<_>>(), ["Revert \"Undo one\"", "Revert \"Undo three\""], "the newest commit is reverted first");
    assert!(git(path, &["status", "--porcelain"]).is_empty(), "the working tree is clean");
}

#[test]
fn mark_a_commit_then_compare_it_with_another() {
    let repo = repository();
    let path = repo.path();
    let base = git(path, &["rev-parse", "HEAD~1"]);
    let head = git(path, &["rev-parse", "HEAD"]);
    let mut harness = open(path);
    loaded(&mut harness);

    harness.get_by_label("Start the notes").click_secondary();
    settle(&mut harness);
    harness.get_by_label_contains("Mark for comparison").click();
    wait(&mut harness, "the mark notice", |h| h.state().notice.as_ref().is_some_and(|n| n.text.starts_with("Marked")));
    assert_eq!(harness.state().repo().and_then(|r| r.compare_base.clone()), Some(base.clone()), "the first commit is marked");

    harness.get_by_label("Add a second line").click_secondary();
    settle(&mut harness);
    harness.get_by_label_contains("Compare with marked commit").click();
    let id = format!("compare:{base}:{head}");
    wait(&mut harness, "the comparison window", |h| h.state().tools.iter().any(|t| t.id() == id));
    wait(&mut harness, "the changed file", |h| h.query_by_role_and_label(Role::Button, "notes.txt").is_some());
    settle(&mut harness);
    assert!(shows(&harness, "1 changed file"), "the comparison counts the changed file");
    assert_eq!(harness.state().repo().and_then(|r| r.compare_base.clone()), None, "the mark is used up by the comparison");
}

#[test]
fn blame_ignore_whitespace_gives_the_line_back_to_its_earlier_commit() {
    let repo = repository();
    let path = repo.path();
    let original = git(path, &["rev-parse", "HEAD"]);
    // A whitespace-only change to the second line, which Git blames on this commit by default.
    commit_file(path, "notes.txt", "first\nsecond  \n", "Pad the second line");
    let padded = git(path, &["rev-parse", "HEAD"]);
    let mut harness = open(path);
    loaded(&mut harness);

    harness.get_by_label("Pad the second line").click();
    wait_for_commit_file(&mut harness, "notes.txt");
    harness.get_by_label("notes.txt").click_secondary();
    settle(&mut harness);
    harness.get_by_label_contains("Blame at this commit").click();
    let padded_line = format!("{} · Test · second", &padded[..7]);
    let original_line = format!("{} · Test · second", &original[..7]);
    wait(&mut harness, "the blame lines", |h| shows(h, &padded_line));
    settle(&mut harness);
    assert!(!shows(&harness, &original_line), "the whitespace change takes the line's blame by default");

    harness.get_by_role_and_label(Role::CheckBox, "Ignore whitespace").click();
    wait(&mut harness, "the blame without whitespace", |h| shows(h, &original_line));
    settle(&mut harness);
    assert!(!shows(&harness, &padded_line), "ignoring whitespace gives the line to the commit that wrote its content");
}

#[test]
fn clicking_a_blame_line_selects_its_commit_and_shows_its_change() {
    let repo = repository();
    let path = repo.path();
    let head = git(path, &["rev-parse", "HEAD"]);
    let first = git(path, &["rev-parse", "HEAD~1"]);
    let mut harness = open(path);
    loaded(&mut harness);

    harness.get_by_label("Add a second line").click();
    wait_for_commit_file(&mut harness, "notes.txt");
    harness.get_by_label("notes.txt").click_secondary();
    settle(&mut harness);
    harness.get_by_label_contains("Blame at this commit").click();
    let second_line = format!("{} · Test · second", &head[..7]);
    let first_line = format!("{} · Test · first", &first[..7]);
    wait(&mut harness, "the blame lines", |h| shows(h, &second_line) && shows(h, &first_line));
    settle(&mut harness);

    // The second line was written by "Add a second line"; its change opens beside the lines.
    harness.get_by_label(&second_line).click();
    wait(&mut harness, "the change for the clicked line", |h| {
        selected_commit(h) == Some(head.clone()) && change_for_commit_is_shown(h, &head)
    });
    settle(&mut harness);
    assert_eq!(selected_commit(&harness), Some(head.clone()), "the clicked line's commit is selected");
    assert!(change_for_commit_is_shown(&harness, &head), "the change shown is the clicked line's commit");

    // The first line belongs to the first commit, so that change replaces it.
    harness.get_by_label(&first_line).click();
    wait(&mut harness, "the change for the first line", |h| {
        selected_commit(h) == Some(first.clone()) && change_for_commit_is_shown(h, &first)
    });
    settle(&mut harness);
    assert!(change_for_commit_is_shown(&harness, &first), "the change now shown is the first line's commit");
    assert_eq!(harness.query_all(By::new().label(&head[..8])).count(), 0, "the earlier commit's change is replaced");
}

/// Whether the blame window draws the change for `hash`: its header shows the commit's
/// eight-character hash, which only that header uses, and the change has loaded with lines.
/// The diff's lines are painted without accessible labels, so the header identifies the commit.
fn change_for_commit_is_shown(harness: &App, hash: &str) -> bool {
    harness.query_all(By::new().label(&hash[..8])).count() == 1
        && harness.query_by_label("Loading change").is_none()
        && harness.query_by_label("No changes to show").is_none()
}

#[test]
fn blame_before_this_change_steps_back_and_back_returns() {
    let repo = repository();
    let path = repo.path();
    let start = git(path, &["rev-list", "--max-parents=0", "HEAD"]);
    // "Start the notes" wrote the first line; this commit reformats it.
    commit_file(path, "notes.txt", "First line.\nsecond\n", "Tidy the first line");
    let tidy = git(path, &["rev-parse", "HEAD"]);
    let mut harness = open(path);
    loaded(&mut harness);

    harness.get_by_label("Tidy the first line").click();
    wait_for_commit_file(&mut harness, "notes.txt");
    harness.get_by_label("notes.txt").click_secondary();
    settle(&mut harness);
    harness.get_by_label_contains("Blame at this commit").click();
    let tidy_line = format!("{} · Test · First line.", &tidy[..7]);
    let start_line = format!("{} · Test · first", &start[..7]);
    wait(&mut harness, "the blame lines", |h| shows(h, &tidy_line));
    settle(&mut harness);
    assert!(harness.get_by_label("Back to the newer blame").accesskit_node().is_disabled(), "there is no earlier view yet");

    // The parent is looked up when the menu opens, so the item is enabled once it has arrived.
    harness.get_by_label(&tidy_line).click_secondary();
    wait(&mut harness, "the enabled blame-before item", |h| {
        h.query_all_by_label_contains("Blame before this change").next().is_some_and(|item| !item.accesskit_node().is_disabled())
    });
    harness.get_by_label_contains("Blame before this change").click();
    wait(&mut harness, "the blame before the change", |h| shows(h, &start_line));
    settle(&mut harness);
    assert!(!shows(&harness, &tidy_line), "the line is blamed on the commit before its change");
    // The view blames the parent, "Add a second line", where the first line still read "first".
    wait(&mut harness, "the header to name the revision's commit", |h| shows(h, "(Add a second line)"));

    harness.get_by_label("Back to the newer blame").click();
    wait(&mut harness, "the newer blame", |h| shows(h, &tidy_line));
    settle(&mut harness);
    assert!(!shows(&harness, &start_line), "Back returns to the newer blame");
    assert!(harness.get_by_label("Back to the newer blame").accesskit_node().is_disabled(), "the stack is used up");
}

#[test]
fn file_history_restores_an_earlier_version_after_confirmation() {
    let repo = repository();
    let path = repo.path();
    let mut harness = open(path);
    loaded(&mut harness);

    harness.get_by_label("Add a second line").click();
    wait_for_commit_file(&mut harness, "notes.txt");
    harness.get_by_label("notes.txt").click_secondary();
    settle(&mut harness);
    harness.get_by_label_contains("File history").click();
    wait(&mut harness, "the file history", |h| shows(h, "Start the notes · Test"));
    settle(&mut harness);

    harness.get_by_label_contains("Start the notes · Test").click();
    wait(&mut harness, "the earlier version", |h| h.query_by_label_contains("Restore this version").is_some());
    settle(&mut harness);
    harness.get_by_label_contains("Restore this version").click();
    wait(&mut harness, "the restore confirmation", |h| shows(h, "Restore notes.txt?"));
    settle(&mut harness);
    assert_eq!(std::fs::read_to_string(path.join("notes.txt")).unwrap(), "first\nsecond\n", "nothing changes before confirming");

    harness.get_by_label("Restore file").click();
    idle(&mut harness);
    assert_eq!(std::fs::read_to_string(path.join("notes.txt")).unwrap(), "first\n", "the earlier version is back");
    assert_eq!(git(path, &["diff", "--cached", "--name-only"]), "notes.txt", "the restore is staged");
}

#[test]
fn rebase_the_current_branch_onto_another_from_the_branch_menu() {
    let repo = repository();
    let path = repo.path();
    git(path, &["switch", "-q", "-c", "topic", "HEAD~1"]);
    commit_file(path, "other.txt", "other\n", "Topic work");
    let topic = git(path, &["rev-parse", "HEAD"]);
    git(path, &["switch", "-q", "main"]);
    let mut harness = open(path);
    loaded(&mut harness);

    harness.get_by_label("topic").click_secondary();
    settle(&mut harness);
    harness.get_by_label_contains("Rebase main onto this").click();
    wait(&mut harness, "the rebase confirmation", |h| shows(h, "Rebase main onto topic?"));
    settle(&mut harness);
    harness.get_by_role_and_label(Role::Button, "Rebase").click();
    idle(&mut harness);

    wait(&mut harness, "the rebase", |_| git(path, &["log", "--format=%s", "-3"]) == "Add a second line\nTopic work\nStart the notes");
    assert_eq!(git(path, &["rev-parse", "HEAD~1"]), topic, "main now sits on the topic branch");
    assert_eq!(git(path, &["branch", "--show-current"]), "main");
    assert!(git(path, &["status", "--porcelain"]).is_empty());
}

#[test]
fn cherry_pick_one_commit_from_its_menu() {
    let repo = repository();
    let path = repo.path();
    git(path, &["switch", "-q", "-c", "source", "HEAD~1"]);
    commit_file(path, "pick.txt", "picked\n", "Pick this commit");
    let source = git(path, &["rev-parse", "HEAD"]);
    git(path, &["switch", "-q", "main"]);
    let mut harness = open(path);
    loaded(&mut harness);

    harness.get_by_label("Pick this commit").click_secondary();
    settle(&mut harness);
    harness.get_by_label_contains("Cherry-pick").click();
    wait(&mut harness, "the cherry-pick confirmation", |h| shows(h, "Cherry-pick this commit?"));
    settle(&mut harness);
    harness.get_all_by_label("Cherry-pick").last().expect("confirm button").click();
    idle(&mut harness);

    wait(&mut harness, "the cherry-picked commit", |_| git(path, &["log", "-1", "--format=%s"]) == "Pick this commit");
    assert_eq!(std::fs::read_to_string(path.join("pick.txt")).unwrap(), "picked\n");
    assert_eq!(git(path, &["log", "--format=%s", "-2"]), "Pick this commit\nAdd a second line", "applied on top of main");
    assert_eq!(git(path, &["rev-parse", "source"]), source, "the source branch is unchanged");
}

#[test]
fn abort_a_rebase_stopped_by_a_conflict_from_the_banner() {
    let repo = rebase_conflict_repository();
    let path = repo.path();
    let before = git(path, &["rev-parse", "HEAD"]);
    let mut harness = open(path);
    loaded(&mut harness);

    stop_a_rebase_on_a_conflict(&mut harness);
    assert!(has_conflicts(&harness), "the rebase stopped on notes.txt");
    abort_from_the_banner(&mut harness, "rebase");

    assert_eq!(git(path, &["rev-parse", "HEAD"]), before, "main is back where the rebase started");
    assert_eq!(git(path, &["branch", "--show-current"]), "main");
    assert_eq!(std::fs::read_to_string(path.join("notes.txt")).unwrap(), "first\nsecond\n");
    assert!(git(path, &["status", "--porcelain"]).is_empty(), "nothing is left in progress");
}

#[test]
fn continue_a_rebase_after_resolving_its_conflict_from_the_banner() {
    let repo = rebase_conflict_repository();
    let path = repo.path();
    let mut harness = open(path);
    loaded(&mut harness);

    stop_a_rebase_on_a_conflict(&mut harness);
    // The commit being replayed is "Add a second line", so keep its version of the line.
    take_version_from_the_banner(&mut harness, "Take replayed commit");
    wait(&mut harness, "the conflict to be resolved", |h| !has_conflicts(h));
    harness.get_by_label("Continue").click();
    idle(&mut harness);
    wait(&mut harness, "the rebase to finish", |h| operation(h).is_none());

    assert_eq!(git(path, &["log", "--format=%s", "-3"]), "Add a second line\nSide notes\nStart the notes", "the commit is replayed");
    assert_eq!(std::fs::read_to_string(path.join("notes.txt")).unwrap(), "first\nsecond\n");
    assert!(git(path, &["status", "--porcelain"]).is_empty());
}

#[test]
fn abort_a_cherry_pick_stopped_by_a_conflict_from_the_banner() {
    let repo = cherry_pick_conflict_repository();
    let path = repo.path();
    let before = git(path, &["rev-parse", "HEAD"]);
    let mut harness = open(path);
    loaded(&mut harness);

    stop_a_cherry_pick_on_a_conflict(&mut harness);
    assert!(has_conflicts(&harness), "the cherry-pick stopped on notes.txt");
    abort_from_the_banner(&mut harness, "cherry-pick");

    assert_eq!(git(path, &["rev-parse", "HEAD"]), before, "main has no new commit");
    assert_eq!(std::fs::read_to_string(path.join("notes.txt")).unwrap(), "first\nsecond\n");
    assert!(git(path, &["status", "--porcelain"]).is_empty(), "nothing is left in progress");
}

#[test]
fn continue_a_cherry_pick_after_resolving_its_conflict_from_the_banner() {
    let repo = cherry_pick_conflict_repository();
    let path = repo.path();
    let mut harness = open(path);
    loaded(&mut harness);

    stop_a_cherry_pick_on_a_conflict(&mut harness);
    // Keep the picked commit's version of the line, which is "SHOUT".
    take_version_from_the_banner(&mut harness, "Take incoming");
    wait(&mut harness, "the conflict to be resolved", |h| !has_conflicts(h));
    harness.get_by_label("Continue").click();
    idle(&mut harness);
    wait(&mut harness, "the cherry-pick to finish", |h| operation(h).is_none());

    assert_eq!(git(path, &["log", "--format=%s", "-2"]), "Shout the line\nAdd a second line", "the commit is applied");
    assert_eq!(std::fs::read_to_string(path.join("notes.txt")).unwrap(), "first\nSHOUT\n");
    assert!(git(path, &["status", "--porcelain"]).is_empty());
}

#[test]
fn abort_a_revert_stopped_by_a_conflict_from_the_banner() {
    let repo = revert_conflict_repository();
    let path = repo.path();
    let before = git(path, &["rev-parse", "HEAD"]);
    let mut harness = open(path);
    loaded(&mut harness);

    stop_a_revert_on_a_conflict(&mut harness);
    assert!(has_conflicts(&harness), "the revert stopped on notes.txt");
    abort_from_the_banner(&mut harness, "revert");

    assert_eq!(git(path, &["rev-parse", "HEAD"]), before, "the revert added no commit");
    assert_eq!(git(path, &["log", "-1", "--format=%s"]), "Capitalise the second line");
    assert_eq!(std::fs::read_to_string(path.join("notes.txt")).unwrap(), "first\nSECOND\n");
    assert!(git(path, &["status", "--porcelain"]).is_empty(), "nothing is left in progress");
}

#[test]
fn continue_a_revert_after_resolving_its_conflict_from_the_banner() {
    let repo = revert_conflict_repository();
    let path = repo.path();
    let mut harness = open(path);
    loaded(&mut harness);

    stop_a_revert_on_a_conflict(&mut harness);
    // The reverted change removes "second", so keep the parent's version of the line.
    take_version_from_the_banner(&mut harness, "Take incoming");
    wait(&mut harness, "the conflict to be resolved", |h| !has_conflicts(h));
    harness.get_by_label("Continue").click();
    idle(&mut harness);
    wait(&mut harness, "the revert to finish", |h| operation(h).is_none());

    assert!(git(path, &["log", "-1", "--format=%s"]).starts_with("Revert"), "a revert commit is made");
    assert_eq!(std::fs::read_to_string(path.join("notes.txt")).unwrap(), "first\n");
    assert!(git(path, &["status", "--porcelain"]).is_empty());
}

/// Points `commit.template` in `dir` at a file outside the repository holding `contents`. The
/// returned folder must live as long as the test uses the template.
fn commit_template_file(dir: &Path, contents: &str) -> tempfile::TempDir {
    let folder = temp_dir();
    let file = folder.path().join("message.txt");
    std::fs::write(&file, contents).unwrap();
    git(dir, &["config", "commit.template", &file.to_string_lossy()]);
    folder
}

/// Opens the repository at `path` with `draft` saved for its checkout, as an earlier session
/// would have left it. The draft is in place before the app reads the repository.
fn open_with_saved_draft(path: &Path, draft: crate::settings::Draft) -> App {
    let key = git(path, &["rev-parse", "--show-toplevel"]);
    let path = path.to_path_buf();
    arm_watchdog();
    Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_eframe(move |cc| {
        let mut app = NiceGitApp::new(cc, None);
        app.settings.auto_fetch_minutes = 0;
        app.settings.drafts.insert(key, draft);
        app.open(path);
        app
    })
}

#[test]
fn empty_commit_box_starts_from_the_commit_template() {
    let repo = repository();
    let _template = commit_template_file(repo.path(), "Fix:\n\nDetails\n");
    let mut harness = open(repo.path());
    wait(&mut harness, "the commit template", |h| h.state().repo().is_some_and(|r| r.draft.summary == "Fix:"));
    let draft = harness.state().repo().expect("repository").draft.clone();
    assert_eq!(draft.description, "Details");
    assert!(shows(&harness, "From commit.template"), "the box says where its message came from");
}

#[test]
fn commit_template_does_not_replace_a_saved_draft() {
    let repo = repository();
    let _template = commit_template_file(repo.path(), "Fix:\n\nDetails\n");
    // The checkout's saved draft is in settings before the app reads the repository.
    let saved = crate::settings::Draft { summary: "Half-written".into(), description: "Keep me".into() };
    let mut harness = open_with_saved_draft(repo.path(), saved);
    loaded(&mut harness);
    settle(&mut harness);
    let repo_state = harness.state().repo().expect("repository");
    assert_eq!(repo_state.draft.summary, "Half-written");
    assert_eq!(repo_state.draft.description, "Keep me");
    assert!(repo_state.template_read.is_none(), "no template is read over a saved draft");
    assert!(!shows(&harness, "From commit.template"));
}

#[test]
fn commit_template_refills_after_a_commit() {
    let repo = repository();
    let _template = commit_template_file(repo.path(), "Fix:\n\nDetails\n");
    std::fs::write(repo.path().join("notes.txt"), "first\nsecond\nthird\n").unwrap();
    let mut harness = open(repo.path());
    loaded(&mut harness);
    wait(&mut harness, "the commit template", |h| h.state().repo().is_some_and(|r| r.draft.summary == "Fix:"));

    harness.get_by_label("Stage All Changes").click();
    idle(&mut harness);
    // As Git refuses an unedited template, the untouched template cannot be committed.
    wait(&mut harness, "the template to be filled in", |h| has_label(h, "Fill in the template to commit"));
    assert!(!harness.state().can_commit(), "an untouched template is not a commit message");
    harness.state_mut().repo_mut().expect("repository").draft.summary = "Fix: the notes".into();
    harness.run_steps(2);
    wait(&mut harness, "the commit button", |h| has_label(h, "Commit 1 file to main"));
    harness.get_by_label_contains("Commit 1 file to main").click();
    idle(&mut harness);
    assert_eq!(git(repo.path(), &["log", "-1", "--format=%B"]), "Fix: the notes\n\nDetails");

    // The box starts from the template again once the commit has been made.
    wait(&mut harness, "the template for the next commit", |h| h.state().repo().is_some_and(|r| r.draft.summary == "Fix:"));
    assert_eq!(harness.state().repo().expect("repository").draft.description, "Details");
}
