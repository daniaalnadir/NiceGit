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

Two more benchmarks time what a person waits for, headless and with real Git: switching
between twenty open repositories, and staging then committing 5,000 files. Each time runs from
the action to the first frame that shows its result:

```sh
cargo test --release -p nicegit ui_bench_speed -- --ignored --nocapture --test-threads 1
```

To see the same timings in the running app, start it with `NICEGIT_TIMING=1`. It prints, to
standard error, when the window is ready, when the first repository shows, how long each switch
takes to show a repository, and for each action how long Git took and how long the refresh read:

```sh
NICEGIT_TIMING=1 ./target/release/nicegit <folder>
```

Measure before and after any performance change against the same generated repository:
run the benchmark on the code as it was, make the change, run it again, and report both
tables in the pull request. Changes to how the history is drawn also need the interface
benchmark's frame times before and after.

## Translations

The interface text is written in English and translated in `rust/crates/nicegit/locales/`,
one file per language (`de.tsv`, `fr.tsv`, `es.tsv`, `pt-BR.tsv`, `ja.tsv`, `zh-Hans.tsv`).
Each line holds an English string, a tab, and its translation:

```text
# Comments start with a hash.
Check for new versions<TAB>Auf neue Versionen prüfen
```

Inside a field, `\t`, `\n` and `\\` stand for a tab, a newline and a backslash. Keep
placeholders such as `{count}` or `{branch}` exactly as the English has them. In the code,
wrap user-visible English with `tr("...")`, `tr_fmt!("...", name = value)` or
`tr_count("one", "other", n)` (see `rust/crates/nicegit/src/i18n.rs`). Do not change English
text while wrapping it, because English is the key. Branch names, command names, keyboard
shortcuts and product names stay as they are.

The test `every_catalog_matches_the_strings_in_the_source` in `i18n.rs` reads every `tr`
call in the source and fails when a catalog lacks a key, keeps a key no call uses any
more, or has a translation whose placeholders differ from its English. Run it with
`cargo test -p nicegit i18n`.

To add a language, add a variant to `Locale` and `Language` and its tag and name, a catalog
file with every key, and its line in the Settings menu. Fonts for Chinese and Japanese are
read from the system; add a path to `cjk_font_candidates` if your system keeps one elsewhere.

## Pull Requests

Describe the behavior changed, tests run, and any known limits. Include screenshots
for interface changes with private paths, repository names, and account data
removed. Run the full test suite and the checks above before requesting review.
Contributions are made under this repository's MIT license.
