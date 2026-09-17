# NiceGit

**A free, open-source Git client for macOS.**

NiceGit brings your repositories, branches, worktrees, and changes into one native
desktop workspace. See how commits connect, review your changes, and keep a real
terminal close by without leaving the app. No subscription or paid feature tiers.

Built with SwiftUI and AppKit. Released under the [MIT License](LICENSE).

> **Preview software:** NiceGit is under active development. Back up important
> work before trying history-changing operations. The current version is 0.1.0.

## Features

| Workspace | What you can do |
| --- | --- |
| Visual history | Follow a parent-linked branch and merge graph, inspect commits, and see the working tree connected to HEAD. |
| Repositories | Open, initialize, or clone repositories; switch between saved tabs in a separate collapsible sidebar. |
| Branches and worktrees | Browse local and remote branches, create linked worktrees, switch checkouts, and use context menus for branch actions. |
| Changes and commits | Separate resizable Unstaged and Staged panes, inline red/green diffs, editable working-file lines, individual-line staging and unstaging, and per-checkout commit drafts. |
| Everyday Git | Fetch, pull, push, publish branches, manage stashes and tags, and import or export patches. |
| History and conflicts | Merge, rebase, cherry-pick, revert, reset with confirmation, and resolve text or binary conflicts. |
| Embedded terminal | Open a resizable bottom panel with a separate shell for each checkout. Toggle with Control-backtick. |
| GitHub | Load and filter pull requests and issues through your existing GitHub CLI login. |

See the [feature guide](docs/FEATURES.md) for behavior, safeguards, and limitations.

## Get Started

### Requirements

- macOS 14 or newer is the deployment target. Interactive testing currently
  covers macOS 27 on Apple Silicon; older macOS versions and Intel Macs still
  need runtime verification.
- Xcode with **Swift 6.3 or newer**, its command-line tools selected, and initial
  setup completed. Local builds have been verified with Xcode 27.
- Git available on your PATH.
- An internet connection for the first dependency download.

### Build and Run

```sh
git clone https://github.com/daniaalnadir/NiceGit.git
cd NiceGit
swift run --build-system native NiceGit
```

To create the macOS app bundle:

```sh
bash scripts/build-app.sh release
open dist/NiceGit.app
```

The build creates an ad-hoc-signed app for the host architecture. It is not
Developer ID signed or notarized, and downloaded copies may be blocked by macOS.
Building the source locally is the currently documented installation route.
Publishing source on GitHub does not require Apple signing.

The native SwiftPM build backend avoids a SwiftTerm shader-build issue encountered
with the command-line SwiftPM default backend in Xcode 27. It currently emits a deprecation warning.

### Run in Xcode

Open `Package.swift`, select the **NiceGit** scheme and **My Mac**, then press
**Command-R**. Select the executable scheme, not `NiceGitCore` or `NiceGit-Package`.
Xcode runs a bare executable rather than the packaged `.app`; NiceGit explicitly
activates its desktop interface for this launch mode. Use **Command-.** to stop an
older run before restarting. Use the packaging script above for the distributable app.

### Optional GitHub Integration

Install GitHub CLI separately and authenticate locally:

```sh
gh auth login
```

Then select a remote under **Pull Requests** or **Issues** in NiceGit. Only
`github.com` is currently supported. NiceGit reuses the CLI's authentication and
does not create its own token store. Ordinary Git operations use your existing
Git authentication setup.

## Development

```sh
swift test --build-system native
```

Tests use disposable repositories. Live GitHub tests are opt-in; the normal suite
requires no GitHub login. See [CONTRIBUTING.md](CONTRIBUTING.md) for details.

- `Sources/NiceGit`: native interface and application state.
- `Sources/NiceGitCore`: Git commands, parsers, commit graph, and GitHub reads.
- `Tests`: integration and regression tests.
- `scripts`: packaging, icons, and preview fixtures.

## Current Limitations

- This is not full GitKraken feature parity. NiceGit is an independent project
  and is not affiliated with GitKraken.
- Undo/Redo covers the last eligible commit made in the current NiceGit session,
  not every Git operation. Commit-message editing is limited to HEAD.
- GitHub Enterprise and other providers' issue/PR APIs are not supported.
- Public app signing, notarization, and automatic updates are not configured.
- The embedded terminal is a real local shell. Git hooks, filters, and shell
  startup files can execute code; only open repositories you trust.

## Contributing and Support

Bug reports and focused pull requests are welcome. Include your macOS version,
NiceGit version, and steps to reproduce using a disposable repository. Remove
private paths, repository content, and credentials from screenshots and logs.

- [Contribution guide](CONTRIBUTING.md)
- [Security policy](SECURITY.md)
- [Changelog](CHANGELOG.md)
- [Release checklist](docs/RELEASING.md)
- [Verification notes](VERIFICATION.md)

## License

Copyright (c) 2026 Daniaal. Licensed under the [MIT License](LICENSE).

MIT permits use, modification, redistribution, and commercial use, provided its
copyright and license notices are retained. It does not require forks to remain
open source or free of charge. NiceGit itself is offered free of charge.

Dependencies retain their own licenses. See [Third-Party Notices](THIRD_PARTY_NOTICES.md).
