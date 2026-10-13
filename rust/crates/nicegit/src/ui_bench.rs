//! Frame time for the commit history with 1000 commits loaded, measured headless: the app draws
//! each frame through egui_kittest, with no GPU renderer, while the history is scrolled.
//!
//! The test is ignored by default. Run it with (see CONTRIBUTING.md, Performance):
//!
//! ```text
//! NICEGIT_BENCH_REPO=<folder> cargo test --release -p nicegit ui_bench -- --ignored --nocapture
//! ```
//!
//! Without `NICEGIT_BENCH_REPO`, a repository is generated with `scripts/benchmark-repository.sh`,
//! which needs bash; on Windows, or without bash, the test prints why it skipped and passes.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use egui::{vec2, Key, Modifiers, MouseWheelUnit, TouchPhase};
use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;

use crate::app::{NiceGitApp, PAGE};
use crate::ui_tests::{idle, loaded, open, settle, wait};

/// Frames timed while the history scrolls.
const FRAMES: usize = 30;
/// The commits the history shows: two pages, which the app reads by scrolling to its end.
const LOADED: usize = 2 * PAGE;

/// Keeps a generated repository alive until the test ends.
struct Generated(#[allow(dead_code)] tempfile::TempDir);

/// The repository to draw: `NICEGIT_BENCH_REPO` if set, otherwise one generated now.
fn benchmark_repository() -> Option<(Option<Generated>, PathBuf)> {
    if let Some(path) = std::env::var_os("NICEGIT_BENCH_REPO") {
        return Some((None, PathBuf::from(path)));
    }
    if cfg!(windows) {
        println!("Skipped: the generated benchmark repository needs bash, which this platform lacks. Set NICEGIT_BENCH_REPO instead.");
        return None;
    }
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/benchmark-repository.sh");
    let bash_available = Command::new("bash").arg("-c").arg("exit 0").output().is_ok_and(|output| output.status.success());
    if !script.is_file() || !bash_available {
        println!(
            "Skipped: bash or {} is not available to generate the benchmark repository. Set NICEGIT_BENCH_REPO instead.",
            script.display()
        );
        return None;
    }
    let folder = tempfile::tempdir().expect("temporary folder");
    let target = folder.path().join("benchmark");
    let output = Command::new("bash").arg(&script).arg(&target).output().expect("run benchmark-repository.sh");
    assert!(output.status.success(), "benchmark-repository.sh failed: {}", String::from_utf8_lossy(&output.stderr));
    Some((Some(Generated(folder)), target))
}

fn loaded_commits(harness: &Harness<'static, NiceGitApp>) -> usize {
    harness.state().snapshot().map(|snapshot| snapshot.commits.len()).unwrap_or(0)
}

/// The centre of a visible commit row, where the wheel scrolls the history.
fn a_visible_row(harness: &Harness<'static, NiceGitApp>) -> egui::Pos2 {
    harness.query_all_by_label_contains("Change ").next().expect("a commit row is on screen").rect().center()
}

#[test]
#[ignore = "benchmark: run with --ignored --nocapture (see CONTRIBUTING.md, Performance)"]
fn history_frame_time_with_a_thousand_commits() {
    let Some((_generated, path)) = benchmark_repository() else { return };
    let mut harness = open(&path);
    loaded(&mut harness);

    // Load the second page as the app does when history grows: a larger history limit, reloaded
    // (Command-R reloads with the limit the repository has).
    if let Some(repo) = harness.state_mut().repo_mut() {
        repo.history_limit = LOADED;
    }
    harness.key_press_modifiers(Modifiers::COMMAND, Key::R);
    idle(&mut harness);
    wait(&mut harness, "the second page of history", |h| loaded_commits(h) == LOADED);
    settle(&mut harness);

    harness.hover_at(a_visible_row(&harness));
    let mut frames: Vec<Duration> = Vec::with_capacity(FRAMES);
    for _ in 0..FRAMES {
        harness.event(egui::Event::MouseWheel {
            unit: MouseWheelUnit::Point,
            delta: vec2(0.0, -300.0),
            phase: TouchPhase::Move,
            modifiers: Modifiers::NONE,
        });
        let start = Instant::now();
        harness.step();
        frames.push(start.elapsed());
    }
    frames.sort();
    let median = (frames[FRAMES / 2 - 1] + frames[FRAMES / 2]) / 2;
    let ms = |duration: Duration| duration.as_secs_f64() * 1000.0;
    println!();
    println!("History panel with {} commits loaded, scrolled over {FRAMES} frames:", loaded_commits(&harness));
    println!("  median {:.2} ms per frame, fastest {:.2} ms, slowest {:.2} ms", ms(median), ms(frames[0]), ms(frames[FRAMES - 1]));
    println!();
}
