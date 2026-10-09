# Changelog

## 0.6.1 - Nothing cut off

- Every diff wraps long lines, in one column as well as side by side, so no line is cut off
  at the panel edge. The built-in editor wraps too, numbering only each line's first row.
- A new or deleted file's diff uses one column, and the Split control says why it is
  unavailable; a file that only gained lines still shows side by side.
- Toolbar groups are separated by clear rules, the More menu matches the other toolbar
  buttons, and the commit button reads as a button even before a summary is written.
- Design-review screenshots render offscreen from a test, with no display needed.

## 0.6.0 - Every feature checked by an interface test

- Side-by-side diffs wrap long lines within each side, as in the Mac app, and the diff
  panel opens at a readable height.
- Hide whitespace applies to every diff: Compare, blame and file history commits, and the
  stash preview, as well as the Changes panel.
- TIFF, ICO, and Apple icon images are previewed when comparing, and HEIC photos on macOS.
- Returning to NiceGit always refreshes, as in the Mac app; the automatic refresh setting
  covers changes noticed while it is in use.
- Commit message drafts belong to the checkout, so switching branches keeps the message
  being written. The inspector shows commit messages exactly as written.
- Visual polish from an independent review: the Settings window is centred, toolbar groups
  are separated, checkboxes no longer look like radio buttons, zero counts are muted,
  Unstage All is no longer drawn as destructive, palette swatches line up, the editor's line
  numbers stay beside their lines, and long branch names in the inspector stay on one line.
- Icon buttons and form fields have accessible names for screen readers.
- On Linux, selecting a file no longer triggers an extra repository refresh: the watcher
  ignores NiceGit's own file reads. A new or deleted file's diff uses one column.
- Every feature in the Mac app's guide is now exercised by an interface test, an
  integration test, or both; see [docs/PARITY.md](docs/PARITY.md).

## 0.5.0 - Interface polish and verified parity

- A calmer, more readable interface after an independent design review: the Inter typeface,
  a one-row toolbar that switches to compact icons and a More menu when space is short,
  tool windows centred in three sizes, a wider graph label column with branch, remote, and
  tag icons, larger avatars, and stronger contrast in the light theme.
- Pull requests and issues load in the sidebar 100 at a time, and a slow request can be
  cancelled. Tags can be created lightweight or annotated. Double-clicking a panel divider
  restores its default width. Popping a stash and deleting a remote tag ask first, and
  editing a published commit's message warns before rewriting it.
- Branch clean-up never preselects main, master, develop, trunk, or a remote's default
  branch, and a hard reset is shown as a destructive action.
- [docs/PARITY.md](docs/PARITY.md) names the test behind every Mac app feature. Interface
  tests now clone, create and remove worktrees, track LFS patterns, reopen recent
  repositories, apply identity profiles, clean up branches with Undo, bisect, cherry-pick
  marked commits, recover lost work, and set up GitFlow through the real window.

## 0.4.0 - Complete parity and cross-platform fixes

- Every feature in the Mac app's guide is now in the cross-platform app; see
  [docs/PARITY.md](docs/PARITY.md). New since 0.3.0: a Stashes window with selected files,
  interactive rebase with drag-to-reorder, the terminal as a bottom panel, the bisect bar,
  sidebar sections for submodules, pull requests, issues, and recent repositories, branch
  menu fetch, pull, push, reset, and GitHub links, editing the HEAD message, choosing a
  merge's parent, confirmations for undo, redo, restores, and patches, twenty undoable
  discards, and dragging graph labels to merge.
- Windows draws with wgpu, so NiceGit runs on machines without OpenGL drivers such as
  virtual machines and Remote Desktop. The Linux package declares libxkbcommon-x11 and
  OpenGL. Small screens get narrower panels, a wrapping toolbar, and a window that fits.
- Interface tests click through the real app, and CI runs the app on macOS, Windows, and
  Linux.

## 0.3.0 - Cross-platform feature parity

- The cross-platform app now matches the Mac app: repository tabs, a branches sidebar
  with filtering and drag-to-merge, a commit graph with branch labels and author
  initials, a commit inspector with signatures, path and tree change views, staging
  individual lines, undo and redo, interactive rebase, reset, cherry-pick and revert,
  merge and rebase previews, a conflict editor, blame, file history, history and file
  content search, comparisons with image previews, bisect, recover lost work, branch
  clean-up, worktrees, submodules, GitFlow, Git LFS, repository settings and identity
  profiles, GitHub pull requests and issues, a command palette, settings with light,
  dark, and colour-blind safe palettes, per-checkout drafts, a file editor, automatic
  refresh, and an embedded terminal.

## 0.2.0 - Cross-platform preview

- New cross-platform NiceGit for macOS, Windows, and Linux, written in Rust with
  egui, in `rust/`. It covers the everyday workflow: the commit graph, branches,
  remotes, tags, stashes, staging, discarding, committing and amending, undoing
  the last commit, switching branches with changes carried in a stash, merging,
  fetch, pull, push, and publish, with the same safety checks as the Mac app.
- Release builds for all three platforms are published from tags by GitHub
  Actions: a universal macOS DMG, a Windows ZIP, and a Linux tarball and .deb.

## 0.1.0 - Unreleased Preview

- Native macOS repository browser with a parent-linked commit graph and working
  tree visualization.
- Separate collapsible open-repository sidebar, branch/remote navigation,
  worktree switching, and repository action bar.
- Staging, commits, diffs, stash management, branch/tag actions, and confirmations
  for history-changing operations.
- Merge, rebase, cherry-pick, revert, and conflict-resolution workflows.
- Restore individual files to their version in, or before, a selected commit.
- File history that follows renames, with per-commit diffs and restore.
- Interactive rebase with drag-to-reorder, reword, squash, fixup, and drop.
- Blame view, full-history commit search, and commit comparison.
- Signature status, whitespace-insensitive diffs, and image comparison.
- Undoable discards, a submodules sidebar section, and multi-commit cherry-pick.
- Keyboard navigation in the graph and drag-and-drop merge or rebase.
- Merge and rebase previews, automatic refresh after outside changes, and bisect.
- Resizable repositories column and branches sidebar, with remembered widths.
- Everyday actions are 4 to 40 times faster on a 3,000-file repository: committing went
  from about 1,050 ms to 57 ms, undo from 2,030 ms to 49 ms, staging a file from 370 ms to
  29 ms, and a refresh from 1,030 ms to 29 ms.
- Undo for merges, rebases, resets, pulls, cherry-picks, and branch deletions; branch
  clean-up; and file content search at any commit.
- Identity profiles, a Settings window with appearance and colour-blind-safe graph colours,
  GitFlow, and Git LFS tracking.
- Amend with staged changes, side-by-side diffs, reflog recovery, stashing selected
  files, and worktree removal.
- Command palette, remote rename/remove/URL editing, tag push and remote tag deletion,
  and ignoring untracked files.
- Embedded per-checkout terminal with Control-backtick toggling.
- GitHub pull-request and issue lists through an existing GitHub CLI login.
- MIT-licensed source, test suite, local app packaging, and GitHub CI.

This is an initial preview, not a stable release. Local packages are ad-hoc signed
and are not notarized. See README for supported behavior and known limitations.
