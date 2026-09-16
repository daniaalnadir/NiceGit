# NiceGit Feature Guide

NiceGit is a free native macOS Git client built with SwiftUI. The goal is to make the everyday GitKraken-style workflow available without paid seats or feature gates.

**Status: 0.1.0 preview.** A working desktop client under active development, not
a drop-in replacement for every GitKraken feature. Back up important work before
trying history-changing operations. NiceGit is an independent project and is not
affiliated with GitKraken.

## Requirements

- macOS 14 or newer (deployment target). Interactive verification currently covers
  macOS 27 on Apple Silicon; older systems and Intel Macs need runtime testing.
- Xcode with Swift 6.3 or newer, with its command-line tools selected and initial
  setup completed. The local release build was verified with Xcode 27.
- Git available on your PATH. GitHub CLI (`gh`) is optional, needed only for the
  GitHub pull-request and issue sidebar; sign in using `gh auth login` yourself.
- SwiftPM downloads dependencies on the first build. NiceGit has no paid tier.

Start from a clone of this repository and run the commands below in its root.
See [Contributing](../CONTRIBUTING.md), [Security](../SECURITY.md), and
[Publishing](RELEASING.md) for development and release guidance.

## Current Features

- Open any local Git repository.
- Initialize repositories and configure a repository-local commit identity.
- Add remotes through Repository Settings.
- Clone repositories from a remote URL or local path.
- Run Git operations in the background with a busy indicator.
- View local and remote branches.
- Browse linked worktree folders and branches, and open another checkout from the sidebar.
- Open a worktree, reveal it in Finder, or copy its path from the worktree context menu.
- Create a linked worktree for an existing local branch from its context menu.
- Keep separate commit-message drafts for each checkout across app restarts, stored in local preferences (not sent to a server).
- Filter local branches, remote branches, and tags by name in the sidebar.
- See commit history with a parent-linked branch and merge graph.
- See the working tree connected to its actual HEAD in the graph, even when another branch has newer commits.
- Browse changed files as an expandable folder tree or a flat path list.
- Load older history and search loaded commits by message, author, hash, or reference.
- Review staged and unstaged file changes.
- Inspect color-highlighted diffs for staged, unstaged, and untracked files.
- Search diffs with previous/next matching-line navigation and Command-F.
- Select a commit to inspect its metadata, changed files, and patches in a persistent side panel.
- Read the complete commit message, including multiline descriptions, in the inspector.
- Stage, unstage, and commit changes.
- Create and checkout local branches.
- Rename branches and delete merged branches from their context menu.
- Create local tags from commits, inspect their targets, and delete local tags.
- Create local tracking branches from remote branches.
- Publish new branches to a chosen remote and set their upstream.
- Fetch, pull, push, and refresh from the toolbar.
- Show outgoing and incoming commit counts against the configured upstream.
- Keep recently opened repositories in the sidebar.
- Refresh repository state when returning to the app, unless an operation or review dialog is active.
- Save, apply, pop, and delete stashes, including untracked files. Pop removes the
  saved stash only after applying succeeds; failed applies retain it.
- Preview a stash's tracked and untracked changes before applying it.
- Merge or rebase from branch context menus; cherry-pick from commit context menus.
- Continue or abort interrupted operations after resolving and staging conflicts.
- Edit UTF-8 text conflicts in-app, with marker checks and external-edit protection.
- Compare base, current, and incoming conflict versions alongside the editable result.
- Resolve whole-file conflicts by keeping either side or deleting the file, including binary files.

## Run

```sh
swift run --build-system native NiceGit
```

## Build a macOS App

```sh
bash scripts/build-app.sh
open dist/NiceGit.app
```

The bundle is ad-hoc signed. Downloaded copies may be blocked by Gatekeeper;
Developer ID signing and notarization are not yet configured.

## Tests

```sh
swift test --build-system native
```

## Behavior and Limitations

The action bar includes Undo/Redo for the last successful non-initial commit made
in NiceGit during this session. Undo uses soft reset and preserves the index and
working files; Redo restores that commit. Both confirm before changing local
history and reject a changed branch or HEAD. This is not a general undo stack for
all Git operations, and it does not persist across restarts.

Open tabs have their own collapsible leftmost repository sidebar, separate from
the branch and remote navigation panel. Its collapsed state persists on restart.
Switch or close individual repository tabs without deleting files or saved commit
drafts. Closing the active tab selects a neighbour; closing the last tab returns
to the repository picker. Open tabs and the last active repository restore on
launch; closing a tab removes it from the saved session.

The repository sidebar includes a repository switcher and collapsible Local,
Remote, Worktrees, Tags, Stashes, Pull Requests, and Issues sections. Counts for
local Git data come from the repository. Pull Requests and Issues load open items
on demand from a selected github.com remote using an installed, signed-in GitHub
CLI (gh). NiceGit reuses that authentication and does not store a separate token.
The sections support open/closed/all state selection (plus merged PRs), filtering
loaded titles/authors/numbers, refresh, cancellation, links to GitHub, and loading up to
1,000 items in batches of 100. Counts reflect loaded items, not a server-side total.
GitHub Enterprise and other hosting providers are not yet supported.

Branch context menus provide checkout, fetch, current-branch pull/push, selected-branch
push to an explicit same-named remote branch with confirmation, an upstream
selector with current-tracking indicators and removal,
worktree creation, branch creation at the selected tip without switching checkouts,
merge/rebase, safe local rename/delete, and copying the
branch name or full commit SHA. They do not yet reproduce every GitKraken action.
The Copy GitHub commit link submenu uses an explicitly selected remote and a full
commit hash. Only github.com remotes are supported; creating a link neither
publishes the commit nor verifies that it already exists on the remote.

Create lightweight or annotated local tags from a commit or branch menu. Annotated
tags require a message and use the repository's Git identity. Tags are unsigned;
signing and publishing tags are not yet available in the interface.

Commit menus support revert with confirmation, merge-parent selection, and
conflict Continue/Abort. Revert requires a clean working tree and retains the
original commits.

Cherry-pick asks for confirmation and lets you choose the baseline parent for a
merge commit. The destination branch and HEAD are checked before starting.

The commit graph and branch context menus offer soft, mixed, and hard reset with a confirmation explaining
the index and working-file effects. Hard reset explicitly warns about losing
uncommitted changes, including obstructing untracked paths. The destination branch
and HEAD must still match the confirmed checkout, and no Git operation may be in
progress. Reset does not push or modify remote branches.

Create a shareable `.patch` file from a non-merge commit using its graph context
menu. The export includes author/message metadata and binary changes and does not
change the repository. Merge commits and commits without an exportable patch are
not supported by this action.

Use Repository > Apply Patch to choose a patch file and confirm the destination
checkout. Git checks the captured patch before applying it; resulting changes
remain unstaged and uncommitted. Invalid patches and paths outside the repository
are rejected. Finish any existing merge, rebase, cherry-pick, or revert first.

Edit the current HEAD commit message from its graph menu, including the message
body. Amending changes the commit hash and requires confirmation; staged and
unstaged file edits are excluded. Editing older messages is not yet supported.

Remote checkout reuses a unique existing local tracking branch, including renamed
branches, without resetting local commits. Ambiguous tracking branches must be
selected explicitly from Local.

- Authentication and remote account integrations.
- Developer ID signing and notarization for public distribution.

## macOS 27 and Embedded Terminal

The app has been built and launched on macOS 27 with Xcode 27. The minimum
deployment target remains macOS 14; older-system runtime testing is separate.
The packaging script uses SwiftPM's native build backend to avoid the Xcode 27
default backend's SwiftTerm Metal shader build failure. This backend currently
emits a deprecation warning.

The action bar's Terminal button opens a resizable bottom panel using SwiftTerm.
Control-backtick or View > Show/Hide Terminal toggles the same panel. Hiding it
refreshes repository state so changes made in the shell appear in the workbench.
Each checkout gets its own shell starting in that checkout's directory. Hiding
the panel keeps its shell running for the current app session. Use the stop
button to end a session, with confirmation; an exited shell can be restarted.
SwiftTerm is MIT-licensed and its license is included in the packaged app.

## License

NiceGit is released under the MIT License.
Dependencies retain their own licenses; see [Third-Party Notices](../THIRD_PARTY_NOTICES.md).
