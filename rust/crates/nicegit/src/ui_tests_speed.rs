//! Interface tests for the work NiceGit does ahead of time so it responds at once.

use crate::app::Repo;
use crate::ui_tests::{git, idle, loaded, open, repository, wait};

/// Other open tabs are read in the background once the active one shows, so switching to one shows
/// it in the next frame; reading them does not count as opening them in the recent list.
#[test]
fn other_tabs_are_read_ahead_without_reordering_recent_repositories() {
    let first = repository();
    let second = repository();
    std::fs::write(second.path().join("note.txt"), "changed\n").unwrap();
    git(second.path(), &["add", "note.txt"]);
    let mut harness = open(first.path());
    loaded(&mut harness);
    idle(&mut harness);
    let second_path = second.path().canonicalize().unwrap();
    harness.state_mut().repos.push(Repo::new(crate::app::strip_verbatim(second_path.clone())));
    let recent_before = harness.state().settings.recent.clone();

    wait(&mut harness, "the second tab to be read in the background", |h| h.state().repos[1].snapshot.is_some());
    assert_eq!(harness.state().active, 0, "reading ahead does not switch tabs");
    assert_eq!(harness.state().settings.recent, recent_before, "a tab read ahead is not marked as opened");

    harness.state_mut().switch_to(1);
    harness.step();
    let state = harness.state();
    assert_eq!(state.active, 1);
    let snapshot = state.snapshot().expect("the second tab shows in the first frame after switching");
    assert!(snapshot.status.iter().any(|entry| entry.path == "note.txt"), "it shows the second repository's changes");
    let opened = crate::app::strip_verbatim(second_path);
    assert_eq!(state.settings.recent.first(), Some(&opened), "switching to it counts as opening it");
}
