# Contributing to NiceGit

NiceGit is a free, MIT-licensed Git client for macOS, Windows, and Linux. Small, focused
fixes and well-tested workflow improvements are welcome. Discuss substantial features in
an issue before starting a large change.

## Development

Install Rust 1.95 or newer and Git. On Linux, first install the packages listed in the
[README](README.md#build-from-source). From the repository root:

```sh
cd rust
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

`rust/scripts/test-all.sh` runs the test suite on your Mac and in a Linux Docker container
at the same time. See [rust/README.md](rust/README.md) for how it works.

CI runs the same checks on Ubuntu, macOS, and Windows. A hosted run confirms each platform;
local results do not prove the hosted runners behave the same way.

## Project Layout

- `rust/crates/nicegit`: the interface and application state.
- `rust/crates/nicegit-core`: Git subprocesses, parsers, graph layout, and GitHub reads.
- `rust/crates/nicegit-core/tests`: isolated repository integration tests.
- `rust/scripts`: packaging, the test runner, and a disposable demo repository script.

## Safety and Tests

Use temporary repositories in tests. Never exercise reset, cleanup, or conflict
resolution against a contributor's real working tree. Test staged and unstaged
changes separately; stale selections must not silently target a different HEAD.
History-changing UI actions need clear confirmations. Git operations must remain
cancellable and must not run synchronously on the UI thread.

The normal suite requires no GitHub account. The one test that reads GitHub is ignored by
default; it requires your existing `gh` login and reads the public `cli/cli` repository:

```sh
cd rust
cargo test -p nicegit -- --ignored github_pull_requests_and_issues_load_from_github
```

Do not add tokens, shell histories, private repository contents, signing keys,
generated app bundles, or local IDE preferences to a pull request.

## Performance

The benchmarks time the reads that run on every repository refresh, and the frame time of
the commit history, against a generated repository of 20,000 commits, 200 branches, 100
tags, and 3,000 files, with 200 modified files, 20 staged changes, and 50 untracked files.
Generating it takes a few seconds and needs Git and bash:

```sh
cd rust
scripts/benchmark-repository.sh <folder>
```

Then run the Git benchmark and the interface benchmark against it. Both are ignored by
default and print a table; the Git benchmark also prints the Git version:

```sh
NICEGIT_BENCH_REPO=<folder> cargo test --release -p nicegit-core --test benchmark -- --ignored --nocapture
NICEGIT_BENCH_REPO=<folder> cargo test --release -p nicegit ui_bench -- --ignored --nocapture
```

Without `NICEGIT_BENCH_REPO`, both generate a repository in a temporary folder. On Windows,
or without bash, they skip with a message. Each operation runs once to warm up and then five
times, and the table shows the median, the fastest, and the slowest run. Timings vary with
what else the machine is doing, so compare runs made under similar conditions.

Measure before and after any performance change against the same generated repository:
run the benchmark on the code as it was, make the change, run it again, and report both
tables in the pull request. Changes to how the history is drawn also need the interface
benchmark's frame times before and after.

## Pull Requests

Describe the behavior changed, tests run, and any known limits. Include screenshots
for interface changes with private paths, repository names, and account data
removed. Run the full test suite and the checks above before requesting review.
Contributions are made under this repository's MIT license.
