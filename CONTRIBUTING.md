# Contributing to NiceGit

NiceGit is a free, MIT-licensed native macOS Git client. Small, focused fixes and
well-tested workflow improvements are welcome. Discuss substantial features in
an issue before starting a large change.

## Development

Use a Mac with Swift 6.3 or newer and Git. From the repository root:

```sh
swift package resolve --force-resolved-versions
swift run --build-system native NiceGit
swift test --build-system native
bash scripts/build-app.sh release
```

The native SwiftPM backend is currently required by the validated packaging path;
it emits a deprecation warning. See README for the Xcode 27 shader-build caveat.
CI checks Xcode 26.6 on GitHub's macOS 26 runner; a successful hosted run must be
confirmed after publishing, not inferred from local results.

## Project Layout

- `Sources/NiceGit`: SwiftUI/AppKit interface and application state.
- `Sources/NiceGitCore`: Git subprocesses, parsers, graph layout, and GitHub reads.
- `Tests`: isolated repository integration tests and app-model tests.
- `scripts`: local packaging, icons, and disposable preview fixtures.

## Safety and Tests

Use temporary repositories in tests. Never exercise reset, cleanup, or conflict
resolution against a contributor's real working tree. Test staged and unstaged
changes separately; stale selections must not silently target a different HEAD.
History-changing UI actions need clear confirmations. Git operations must remain
cancellable and must not run synchronously on the UI thread.

App-model tests are serialized because their synchronous fixture setup shares
the main actor. Keep ordinary core tests independent and parallelizable.

The normal suite requires no GitHub account. The optional read-only live check is:

```sh
NICEGIT_LIVE_GITHUB_TEST=1 swift test --build-system native --filter liveGitHubSidebarLoadsPublicRepository
```

It requires your existing `gh` login and reads the public `cli/cli` repository.
Do not add tokens, shell histories, private repository contents, signing keys,
generated app bundles, or local Xcode preferences to a pull request.

## Pull Requests

Describe the behavior changed, tests run, and any known limits. Include screenshots
for interface changes with private paths, repository names, and account data
removed. Run the full test suite and release packaging before requesting review.
Contributions are made under this repository's MIT license.
