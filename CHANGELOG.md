# Changelog

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
