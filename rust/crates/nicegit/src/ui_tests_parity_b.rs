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

#[test]
fn interactive_rebase_warns_about_commits_already_on_a_remote() {
    let repo = repository();
    let path = repo.path();
    let remote = tempfile::tempdir().unwrap();
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
    wait(&mut harness, "the commit's files", |h| h.state().repo().is_some_and(|r| !r.commit_files.is_empty()));
    settle(&mut harness);
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
    wait(&mut harness, "the commit's files", |h| h.state().repo().is_some_and(|r| !r.commit_files.is_empty()));
    settle(&mut harness);
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
        selected_commit(h) == Some(head.clone()) && h.query_by_role_and_label(Role::CheckBox, "Side by side").is_some()
    });
    settle(&mut harness);

    // The first line belongs to the first commit, so that change replaces it.
    harness.get_by_label(&first_line).click();
    wait(&mut harness, "the change for the first line", |h| selected_commit(h) == Some(first.clone()));
    settle(&mut harness);
    assert!(harness.query_by_role_and_label(Role::CheckBox, "Side by side").is_some(), "the change stays open");
}

#[test]
fn file_history_restores_an_earlier_version_after_confirmation() {
    let repo = repository();
    let path = repo.path();
    let mut harness = open(path);
    loaded(&mut harness);

    harness.get_by_label("Add a second line").click();
    wait(&mut harness, "the commit's files", |h| h.state().repo().is_some_and(|r| !r.commit_files.is_empty()));
    settle(&mut harness);
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
