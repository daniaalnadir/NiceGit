//! Times the reads that run on each repository refresh, against a large generated repository.
//!
//! The test is ignored by default. Run it with (see CONTRIBUTING.md, Performance):
//!
//! ```text
//! NICEGIT_BENCH_REPO=<folder> cargo test --release -p nicegit-core --test benchmark -- --ignored --nocapture
//! ```
//!
//! Without `NICEGIT_BENCH_REPO`, the repository is generated into a temporary folder by
//! `scripts/benchmark-repository.sh`, which needs bash. On Windows, or without bash, the test
//! prints why it skipped and passes.
//!
//! Each operation runs once to warm up and then five times; the table shows the median, the
//! fastest, and the slowest of the five.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use nicegit_core::graph::layout_with_working_tree;
use nicegit_core::parsers::parse_status_with_checkout;
use nicegit_core::{GitClient, HistoryScope, StatusKind};

/// The history limit the app loads with: the snapshot's `history_limit` is a page's worth of
/// commits, and this is the upper end of that range the benchmark covers.
const HISTORY_LIMIT: usize = 1000;
/// Timed runs per operation, after one warm-up run. An odd count makes the median a real sample.
const RUNS: usize = 5;
/// The graph's colour count, as `theme::GRAPH_COLOR_COUNT` sets it in the app.
const GRAPH_COLOR_COUNT: usize = 8;

/// One operation's timings across its timed runs.
struct Timing {
    name: &'static str,
    median: Duration,
    min: Duration,
    max: Duration,
}

/// Runs `operation` once to warm up, then `RUNS` times, and keeps the sorted timings.
fn measure<T>(name: &'static str, mut operation: impl FnMut() -> T) -> Timing {
    std::hint::black_box(operation());
    let mut samples: Vec<Duration> = (0..RUNS)
        .map(|_| {
            let start = Instant::now();
            std::hint::black_box(operation());
            start.elapsed()
        })
        .collect();
    samples.sort();
    Timing { name, median: samples[RUNS / 2], min: samples[0], max: samples[RUNS - 1] }
}

/// Keeps a generated repository alive until the benchmark ends.
struct Generated(#[allow(dead_code)] tempfile::TempDir);

/// The repository to measure: `NICEGIT_BENCH_REPO` if it is set, otherwise one generated now.
/// `None` means the benchmark is skipped, with the reason printed.
fn benchmark_repository() -> Option<(Option<Generated>, PathBuf)> {
    if let Some(path) = std::env::var_os("NICEGIT_BENCH_REPO") {
        let path = PathBuf::from(path);
        assert!(path.join(".git").exists() || path.join("HEAD").exists(), "NICEGIT_BENCH_REPO is not a Git repository: {}", path.display());
        return Some((None, path));
    }
    if cfg!(windows) {
        println!("Skipped: the generated benchmark repository needs bash, which this platform lacks. Set NICEGIT_BENCH_REPO instead.");
        return None;
    }
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/benchmark-repository.sh");
    assert!(script.is_file(), "missing {}", script.display());
    let bash_available = Command::new("bash").arg("-c").arg("exit 0").output().is_ok_and(|output| output.status.success());
    if !bash_available {
        println!("Skipped: bash is not available to generate the benchmark repository. Set NICEGIT_BENCH_REPO instead.");
        return None;
    }
    let folder = tempfile::tempdir().expect("temporary folder");
    let target = folder.path().join("benchmark");
    let started = Instant::now();
    let output = Command::new("bash").arg(&script).arg(&target).output().expect("run benchmark-repository.sh");
    assert!(output.status.success(), "benchmark-repository.sh failed: {}", String::from_utf8_lossy(&output.stderr));
    println!("Generated the benchmark repository in {:.1} s with {}", started.elapsed().as_secs_f64(), script.display());
    print!("{}", String::from_utf8_lossy(&output.stdout));
    Some((Some(Generated(folder)), target))
}

/// Prints the repository's own size, as Git reports it.
fn print_size(client: &GitClient, root: &Path) {
    if let Ok(counts) = client.run(&["count-objects", "-vH"], root) {
        for line in counts.lines().filter(|line| line.starts_with("size")) {
            println!("Repository {line}");
        }
    }
}

fn print_table(timings: &[Timing]) {
    println!();
    println!("{:<58} {:>10} {:>10} {:>10}", "Operation", "median ms", "min ms", "max ms");
    println!("{}", "-".repeat(90));
    for timing in timings {
        println!(
            "{:<58} {:>10.1} {:>10.1} {:>10.1}",
            timing.name,
            timing.median.as_secs_f64() * 1000.0,
            timing.min.as_secs_f64() * 1000.0,
            timing.max.as_secs_f64() * 1000.0
        );
    }
    println!();
}

#[test]
#[ignore = "benchmark: run with --ignored --nocapture (see CONTRIBUTING.md, Performance)"]
fn refresh_reads() {
    let Some((_generated, root)) = benchmark_repository() else { return };
    let client = GitClient::new();
    println!("Git {}", client.git_version().unwrap_or_else(|error| format!("unknown ({error})")));
    println!("Measuring {}, history limit {HISTORY_LIMIT}, median of {RUNS} runs after one warm-up", root.display());
    print_size(&client, &root);

    let snapshot = client.load_snapshot_scoped(&root, HISTORY_LIMIT, HistoryScope::All).expect("snapshot");
    let changed: Vec<String> = snapshot.status.iter().map(|entry| entry.path.clone()).collect();
    let untracked: Vec<String> =
        snapshot.status.iter().filter(|entry| entry.kind == StatusKind::Untracked).map(|entry| entry.path.clone()).collect();
    println!(
        "Snapshot: {} commits shown, older history {}, {} branch entries, {} tags, {} changed files ({} untracked), {} stashes",
        snapshot.commits.len(),
        if snapshot.has_more_commits { "beyond the limit" } else { "fully shown" },
        snapshot.branches.len(),
        snapshot.tags.len(),
        changed.len(),
        untracked.len(),
        snapshot.stashes.len()
    );

    let mut timings = Vec::new();
    timings.push(measure("load_snapshot_scoped, scope All", || {
        client.load_snapshot_scoped(&root, HISTORY_LIMIT, HistoryScope::All).expect("snapshot")
    }));
    timings.push(measure("load_snapshot_scoped, scope Local", || {
        client.load_snapshot_scoped(&root, HISTORY_LIMIT, HistoryScope::Local).expect("snapshot")
    }));
    timings.push(measure("load_snapshot_scoped, scope Current", || {
        client.load_snapshot_scoped(&root, HISTORY_LIMIT, HistoryScope::Current).expect("snapshot")
    }));
    timings.push(measure("quick status: load_status (porcelain v1)", || client.load_status(&root).expect("status")));
    timings.push(measure("quick status: status porcelain v2 and parse", || {
        let output = client.run(&["status", "--porcelain=v2", "-z", "--branch", "--untracked-files=all"], &root).expect("status");
        parse_status_with_checkout(&output)
    }));
    timings.push(measure("line_counts for the changed files", || client.line_counts(&untracked, &changed, &root).expect("line counts")));
    timings.push(measure("branch_tracking", || client.branch_tracking(&root).expect("tracking")));
    timings.push(measure("tracked_files", || client.tracked_files(&root).expect("tracked files")));
    timings.push(measure("submodules", || client.submodules(&root).expect("submodules")));
    timings.push(measure("bisect_status", || client.bisect_status(&root).expect("bisect")));
    timings.push(measure("graph layout, working tree, loaded commits", || {
        layout_with_working_tree(&snapshot.commits, snapshot.head_hash.as_deref(), GRAPH_COLOR_COUNT)
    }));
    print_table(&timings);
}
