//! Interface tests for stacked branches. They amend the bottom branch of a stack, then restack
//! or dismiss the notice through the window, and check the repository Git holds. They use the
//! helpers in `ui_tests`.

use egui_kittest::kittest::Queryable;

use crate::ui::dialogs::Dialog;
use crate::ui_tests::*;

/// A repository on main with a stack main → a → b: `a` has one commit, and `b` is built on it.
/// `a` is checked out and then amended, so `b` still holds its old tip.
fn stack_with_an_amended_bottom() -> (tempfile::TempDir, String) {
    let repo = repository();
    let path = repo.path();
    git(path, &["checkout", "-q", "-b", "a"]);
    std::fs::write(path.join("a1.txt"), "a1\n").unwrap();
    git(path, &["add", "a1.txt"]);
    git(path, &["commit", "-q", "-m", "A1"]);
    git(path, &["checkout", "-q", "-b", "b"]);
    std::fs::write(path.join("b1.txt"), "b1\n").unwrap();
    git(path, &["add", "b1.txt"]);
    git(path, &["commit", "-q", "-m", "B1"]);
    git(path, &["checkout", "-q", "a"]);
    std::fs::write(path.join("a1.txt"), "a1 amended\n").unwrap();
    git(path, &["commit", "-q", "--amend", "-am", "A1 amended"]);
    let amended = git(path, &["rev-parse", "a"]);
    (repo, amended)
}

/// The sentence the notice above the graph shows for `b`, once the stack read has arrived.
const NOTICE: &str = "still based on the old a";

#[test]
fn restack_the_branches_left_on_an_amended_branch() {
    let (repo, amended) = stack_with_an_amended_bottom();
    let path = repo.path();

    let mut harness = open(path);
    loaded(&mut harness);
    wait(&mut harness, "the outdated branches notice", |h| h.query_by_label_contains(NOTICE).is_some());
    harness.get_all_by_label("Restack").next().expect("the notice's Restack button").click();
    wait(&mut harness, "the restack confirmation", |h| matches!(h.state().dialog, Some(Dialog::Confirm { .. })));
    settle(&mut harness);
    harness.get_all_by_label("Restack").last().expect("the confirm button").click();
    idle(&mut harness);

    // b now sits on the amended tip of a, with its own commit kept.
    assert_eq!(git(path, &["rev-parse", "b~1"]), amended);
    assert_eq!(git(path, &["log", "-1", "--format=%s", "b"]), "B1");
    assert_eq!(git(path, &["rev-parse", "--abbrev-ref", "HEAD"]), "a");
    assert_eq!(git(path, &["status", "--porcelain"]), "");
    wait(&mut harness, "the notice to go", |h| h.query_by_label_contains(NOTICE).is_none());
}

#[test]
fn dismissing_the_notice_hides_it_until_the_branch_moves_again() {
    let (repo, _) = stack_with_an_amended_bottom();
    let path = repo.path();

    let mut harness = open(path);
    loaded(&mut harness);
    wait(&mut harness, "the outdated branches notice", |h| h.query_by_label_contains(NOTICE).is_some());
    harness.get_all_by_label("Dismiss").next().expect("the notice's Dismiss button").click();
    wait(&mut harness, "the notice to go", |h| h.query_by_label_contains(NOTICE).is_none());
    // Nothing was moved: b still holds the commit it had before the amend.
    assert_eq!(git(path, &["log", "-1", "--format=%s", "b"]), "B1");
    assert_eq!(git(path, &["rev-parse", "--abbrev-ref", "HEAD"]), "a");
}
