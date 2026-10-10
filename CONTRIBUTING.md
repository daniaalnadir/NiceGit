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

## Pull Requests

Describe the behavior changed, tests run, and any known limits. Include screenshots
for interface changes with private paths, repository names, and account data
removed. Run the full test suite and the checks above before requesting review.
Contributions are made under this repository's MIT license.
