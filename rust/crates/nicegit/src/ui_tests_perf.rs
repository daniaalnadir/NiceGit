//! Frame time for the Changes panel with thousands of changed files, measured headless: the app
//! draws each frame through egui_kittest, with no GPU renderer.
//!
//! The test is ignored by default. Run it with:
//!
//! ```text
//! cargo test -p nicegit --release changes_panel_frame_time -- --ignored --nocapture
//! ```

use std::time::Instant;

use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;

use crate::app::NiceGitApp;
use crate::ui_tests::{git, idle, loaded, open, repository, settle, wait};

/// Tracked files committed, then all modified.
const TRACKED: usize = 2000;
/// Of the modified files, how many are staged.
const STAGED: usize = 500;
/// Untracked files, in folders of `UNTRACKED_PER_FOLDER` each.
const UNTRACKED_FOLDERS: usize = 80;
const UNTRACKED_PER_FOLDER: usize = 100;
/// Frames timed in each file view.
const FRAMES: usize = 60;

/// A repository with `TRACKED` modified files, `STAGED` of them staged, and the untracked files.
fn changed_repository() -> tempfile::TempDir {
    let dir = repository();
    let path = dir.path();
    std::fs::create_dir_all(path.join("tracked")).unwrap();
    for n in 0..TRACKED {
        std::fs::write(path.join(format!("tracked/f{n:04}.txt")), "base\n").unwrap();
    }
    git(path, &["add", "tracked"]);
    git(path, &["commit", "-q", "-m", "Add tracked files"]);
    for n in 0..TRACKED {
        std::fs::write(path.join(format!("tracked/f{n:04}.txt")), format!("changed {n}\n")).unwrap();
    }
    let staged: Vec<String> = (0..STAGED).map(|n| format!("tracked/f{n:04}.txt")).collect();
    let mut arguments = vec!["add", "--"];
    arguments.extend(staged.iter().map(String::as_str));
    git(path, &arguments);
    for folder in 0..UNTRACKED_FOLDERS {
        let directory = path.join(format!("untracked/d{folder:02}"));
        std::fs::create_dir_all(&directory).unwrap();
        for n in 0..UNTRACKED_PER_FOLDER {
            std::fs::write(directory.join(format!("n{n:03}.txt")), "new\n").unwrap();
        }
    }
    dir
}

/// Average milliseconds per frame over `FRAMES` frames, after the panel has settled in its view.
fn average_frame_ms(harness: &mut Harness<'static, NiceGitApp>) -> f64 {
    settle(harness);
    let start = Instant::now();
    for _ in 0..FRAMES {
        harness.step();
    }
    start.elapsed().as_secs_f64() * 1000.0 / FRAMES as f64
}

/// Rows scrolled out of view are described to screen readers but not drawn. The first unstaged file
/// is on screen; one at the far end of the list is described, and lies below the screen.
#[test]
fn changes_panel_draws_only_the_rows_on_screen() {
    let dir = changed_repository();
    let mut harness = open(dir.path());
    loaded(&mut harness);
    let expected = TRACKED + UNTRACKED_FOLDERS * UNTRACKED_PER_FOLDER;
    wait(&mut harness, "every changed file to be listed", |h| h.state().snapshot().is_some_and(|s| s.status.len() == expected));
    idle(&mut harness);
    settle(&mut harness);

    // The unstaged list starts with the modified files after the staged ones, then the untracked
    // files, so its first row is tracked/f0500.txt and the last is in the final untracked folder.
    let first = harness.query_by_label("tracked/f0500.txt").expect("the first unstaged file is listed");
    assert!(first.rect().top() < harness.ctx.content_rect().bottom(), "the first unstaged file is on screen");
    let far = harness.query_by_label("untracked/d79/n099.txt").expect("the far file is still described");
    assert!(far.rect().top() > harness.ctx.content_rect().bottom(), "the far file lies below the screen");
}

#[test]
#[ignore = "benchmark: run with --ignored --nocapture"]
fn changes_panel_frame_time_with_many_files() {
    let dir = changed_repository();
    let mut harness = open(dir.path());
    loaded(&mut harness);
    let expected = TRACKED + UNTRACKED_FOLDERS * UNTRACKED_PER_FOLDER;
    wait(&mut harness, "every changed file to be listed", |h| h.state().snapshot().is_some_and(|s| s.status.len() == expected));
    idle(&mut harness);

    harness.get_by_label("Path").click();
    let path_ms = average_frame_ms(&mut harness);
    harness.get_by_label("Tree").click();
    let tree_ms = average_frame_ms(&mut harness);
    eprintln!();
    eprintln!("Changes panel with {expected} changed files ({STAGED} staged), {FRAMES} frames each:");
    eprintln!("  Path view: {path_ms:.2} ms per frame");
    eprintln!("  Tree view: {tree_ms:.2} ms per frame");
    eprintln!();
}
