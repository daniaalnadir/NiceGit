//! What a person waits for, measured headless with real Git: switching between twenty open
//! repositories, and committing thousands of staged files. Each time runs from the call to the
//! first frame that shows the result. The tests are ignored by default. Run them with:
//!
//! ```text
//! cargo test -p nicegit --release ui_bench_speed -- --ignored --nocapture --test-threads 1
//! ```
//!
//! NICEGIT_TIMING=1 also prints how long Git took inside each step.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use egui_kittest::Harness;

use crate::app::{NiceGitApp, Repo};
use crate::ui_tests::{arm_watchdog, git, idle, loaded, open, repository, wait};

/// Open repositories switched between.
const REPOSITORIES: usize = 20;
/// Files staged for the commit.
const STAGED: usize = 5000;

fn milliseconds(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1000.0
}

fn summary(name: &str, mut times: Vec<f64>) {
    times.sort_by(f64::total_cmp);
    let median = times[times.len() / 2];
    let worst = times[times.len() - 1];
    eprintln!("{name}: median {median:.1} ms, worst {worst:.1} ms over {} runs", times.len());
}

/// A repository with `commits` commits of a few files each, made by the benchmark script when
/// bash is available, so the history is a realistic size.
fn repository_with_history(folder: &Path, commits: usize) {
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/benchmark-repository.sh");
    let status = std::process::Command::new("bash")
        .arg(script)
        .arg(folder)
        .args([commits.to_string(), "20".into(), "500".into()])
        .stdout(std::process::Stdio::null())
        .status()
        .expect("bash runs the benchmark script");
    assert!(status.success(), "the benchmark script made {}", folder.display());
}

/// Runs frames until the active repository shows a snapshot, returning the time from `start`.
fn until_shown(harness: &mut Harness<'static, NiceGitApp>, index: usize, start: Instant) -> Duration {
    loop {
        harness.step();
        let state = harness.state();
        if state.active == index && state.repo().is_some_and(|repo| repo.snapshot.is_some()) {
            return start.elapsed();
        }
        assert!(start.elapsed() < Duration::from_secs(30), "repository {index} was never shown");
    }
}

#[test]
#[ignore = "benchmark: run with --ignored --nocapture"]
fn switching_between_twenty_repositories() {
    arm_watchdog();
    let folder = tempfile::tempdir().unwrap();
    let paths: Vec<PathBuf> = (0..REPOSITORIES).map(|n| folder.path().join(format!("repo{n:02}"))).collect();
    std::thread::scope(|scope| {
        for path in &paths {
            scope.spawn(|| repository_with_history(path, 2000));
        }
    });
    let mut harness = open(&paths[0]);
    loaded(&mut harness);
    harness.state_mut().repos.extend(paths[1..].iter().cloned().map(Repo::new));
    idle(&mut harness);
    // Give background work started by opening the tabs time to finish, as a person would before
    // reaching for the next tab.
    let settle = Instant::now();
    while settle.elapsed() < Duration::from_secs(3) {
        harness.step();
        std::thread::sleep(Duration::from_millis(10));
    }

    let mut first = Vec::new();
    for index in 1..REPOSITORIES {
        let start = Instant::now();
        harness.state_mut().switch_to(index);
        first.push(milliseconds(until_shown(&mut harness, index, start)));
        idle(&mut harness);
    }
    // A second round, back through every tab once each has been seen.
    let mut again = Vec::new();
    for index in (0..REPOSITORIES).rev() {
        let start = Instant::now();
        harness.state_mut().switch_to(index);
        again.push(milliseconds(until_shown(&mut harness, index, start)));
        idle(&mut harness);
    }
    eprintln!();
    summary("Switch to a repository not yet visited", first);
    summary("Switch back to a visited repository", again);
}

#[test]
#[ignore = "benchmark: run with --ignored --nocapture"]
fn committing_thousands_of_staged_files() {
    let mut times = Vec::new();
    for run in 0..3 {
        let dir = repository();
        let path = dir.path();
        std::fs::create_dir_all(path.join("many")).unwrap();
        for n in 0..STAGED {
            std::fs::write(path.join(format!("many/f{n:05}.txt")), format!("file {n} of run {run}\n")).unwrap();
        }
        git(path, &["add", "many"]);
        let mut harness = open(path);
        loaded(&mut harness);
        idle(&mut harness);
        let before = harness.state().snapshot().and_then(|s| s.head_hash.clone());
        harness.state_mut().repo_mut().unwrap().draft.summary = "Add many files".into();
        let start = Instant::now();
        harness.state_mut().commit();
        wait(&mut harness, "the commit to be shown", |h| {
            h.state().busy.is_none() && h.state().snapshot().is_some_and(|s| s.head_hash != before && s.status.is_empty())
        });
        times.push(milliseconds(start.elapsed()));
    }
    eprintln!();
    summary(&format!("Commit {STAGED} staged files, until shown"), times);
}
