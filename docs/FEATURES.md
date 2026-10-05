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
- Restore a file from a commit's changed-file list to that commit's version, or to its
  version before the commit, then review and commit the staged result.
- Amend the last commit with staged changes from the commit panel, with a warning when it
  is already on a remote.
- Show diffs side by side (old left, new right, long lines wrapped) or in one column; the
  choice is remembered.
- See what a merge or rebase would do before confirming it: already up to date, a
  fast-forward, a clean merge with the number of files that would change, or the files
  expected to conflict. Rebase predictions are marked as estimates.
- Refresh automatically when files, staging, branches, or tags change outside NiceGit,
  such as edits in another app or commits and fetches in a terminal (Settings to turn off).
- Find the commit that introduced a bug with bisect: start from a known-good commit in the
  graph, then mark each checked-out commit good, bad, or skipped until NiceGit names the
  first bad commit.
- Save commit identities as profiles (name, email, optional signing key) and apply one to
  a repository from Repository Settings or the command palette.
- Settings (Command-comma): System, Light, or Dark appearance; Standard, colour-blind safe,
  or Muted graph colours; and default diff options.
- GitFlow (Repository > GitFlow): set up, start feature, release, and hotfix branches, and
  finish them with no-fast-forward merges and version tags, compatible with git-flow.
- Git LFS (Repository > Git LFS): see tracked patterns and LFS files, including files whose
  content has not been downloaded, and track or untrack patterns.
- Move through the commit graph with the Up and Down arrow keys; Escape clears the
  selection.
- Drag a branch (from the sidebar or a graph label) onto the current branch in the sidebar
  to merge it in or rebase onto it, after confirming.
- Command-click commits in the graph to select several, then cherry-pick them together,
  oldest first.
- Undo a discard from the Changes panel: the file's staged and unstaged versions are
  saved before discarding and restored exactly, until the file changes again.
- See submodules in the sidebar with their state (not checked out, at the recorded commit,
  or on another commit, and whether they have uncommitted changes); open one, or check out
  its recorded commit.
- See whether a commit's GPG or SSH signature is verified, untrusted, bad, or cannot be
  checked on this Mac, in the commit inspector.
- Hide whitespace-only changes in any diff; the choice is remembered.
- Compare changed images side by side with their dimensions and file sizes.
- Recover lost work from Repository > Recover Lost Work: every position HEAD has had,
  with commits on no branch marked, and a branch can be created at any of them.
- Stash only selected files (including untracked ones) from the Stashes sheet.
- Remove a clean linked worktree, and forget worktrees whose folders were deleted.
- Command palette (Shift-Command-P) for repository actions, branch switching, and recent
  repositories, with fuzzy matching.
- Search every branch's history by message, author, or code change (Shift-Command-F),
  including commits not yet loaded in the graph, or paste a commit ID.
- Interactive rebase from a commit's context menu: drag to reorder, and pick, reword,
  squash, fix up, or drop each commit, with a warning for commits already on a remote.
- Compare any commit with the working files, or two commits with each other (mark one,
  then compare from another), with a changed-file list and per-file diffs.
- Blame a file (working version or at a commit), with author, age tinting, an
  ignore-whitespace option, and each line's commit change one click away.
- Rename or remove remotes and edit their fetch URL in Repository Settings.
- Push a tag to a remote, or delete it from a remote, from the tag's context menu.
- Ignore untracked files from the Changes panel: the exact path or its extension in
  `.gitignore`, or only on this computer via `info/exclude`.
- Show a file's history, following renames, from a changed file's context menu in the
  commit inspector or the Changes panel; review each commit's change to it and restore
  an earlier version.
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

Restoring a file from a commit replaces only that path, in both the index and the
working tree, and never moves HEAD or changes other files. The confirmation says
whether the file will be replaced or deleted (when it does not exist in the chosen
version); any staged or unstaged changes to that file are lost. NiceGit rejects the
restore when the branch or HEAD changed after the dialog opened, while an operation
such as a merge is unfinished, while the file has unresolved conflicts, when an
untracked or ignored local file or a folder occupies the path, and for submodules.
It verifies afterwards that the file matches the chosen version.

Interactive rebase lists the commits from the chosen commit through HEAD on the
current branch, newest first. Squash and fixup combine a commit into the nearest kept
commit below it; squash keeps both messages. NiceGit writes Git's rebase instructions
itself and stores new messages as Git objects, so a rebase that stops for a conflict
resumes with Continue and can be undone with Abort. It refuses to start with
uncommitted tracked changes, during another operation, when the branch or HEAD changed
since the editor opened, or when the range contains a merge commit. It does not move
other branches, stash automatically, or reorder commits on its own.

Amending checks that the branch and HEAD still match the moment amend was switched on,
and refuses during an unfinished operation. Stashing selected files verifies that only
those files changed. Worktree removal never forces: Git refuses a worktree with
uncommitted or untracked files, and the main worktree and the open checkout cannot be
removed.

Undo is available for the last 20 discards of individual files; renames, conflicts,
folders, and submodules are discarded without undo, and the confirmation says so. Saved
contents are kept as unreferenced Git objects, which Git's routine cleanup removes after
its usual grace period. Checking out a submodule's recorded commit uses Git's normal
submodule update, which refuses to overwrite uncommitted changes and keeps Git's default
protection against local-file transport.

Profiles apply repository-local settings only. A profile with a signing key also turns on
commit signing for that repository; one without a key leaves signing settings unchanged.
Finishing a GitFlow branch deletes it only after every merge succeeds; if a merge stops for
a conflict, resolve and continue it, then check out the branch and finish again, and
completed steps are skipped. Git LFS tracking requires git-lfs to be installed, because
without it Git would commit matching files in full; NiceGit refuses to track until it is.

Merge previews use Git's in-memory merge and never change files, the index, or refs.
Automatic refresh waits about a second after the last change, skips changes NiceGit's own
actions already refreshed, defers while a review dialog is open, and ignores Git's object
storage, logs, and lock files. NiceGit runs Git with `GIT_OPTIONAL_LOCKS=0` so read-only
commands do not rewrite the index. Bisect checks out commits with a detached HEAD, needs a
clean working tree to start, and returns to the original checkout when it ends.

Signature checks run locally with your Git signing setup and never contact a key server.
NiceGit only asks Git to verify commits whose objects carry a signature; if your Git
configuration prevents verification, the inspector says the signature cannot be checked.
Ignoring whitespace affects what diffs display, never how lines are staged.

Blame, history search, and compare are read-only. Commit search matches text
literally; message and author searches ignore case, and code-change search finds
commits that add or remove the exact text. Comparisons with working files include
staged and unstaged edits to tracked files but not untracked files.

Remote edits and tag pushes check that the remote's addresses still match what was
shown. Tags are pushed with an explicit refspec and never replace a different tag of
the same name on the remote; deleting a remote tag succeeds only while it still matches
the local tag.

File history lists up to 200 commits reachable from the current checkout that changed
the file, newest first, and follows renames. Commits recorded under an earlier name
show that name; restoring from them is unavailable because Git restores by path. A
staged rename shows the history of its original path until it is committed.

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
