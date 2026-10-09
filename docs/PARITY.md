# Cross-platform Feature Parity

Each feature in the Mac app's [feature guide](FEATURES.md), and where the cross-platform
app in [`rust/`](../rust) provides it. "UI test" means a test in
`rust/crates/nicegit/src/ui_tests.rs` clicks and types through the real interface;
"core test" means an integration test in `rust/crates/nicegit-core/tests` runs the Git
operation against a temporary repository. Every window is also opened and drawn by the
`every_repository_tool_opens_from_the_menu` UI test, and CI runs the app itself on macOS,
Windows, and Linux.

## Repositories and window

| Mac app feature | Cross-platform app | Verified by |
| --- | --- | --- |
| Open any local repository | Repository menu, empty-window button, Ctrl/Cmd-O | UI tests open repositories |
| Initialize repositories and set a repository identity | Repository menu › New repository; Repository Settings › Identity | core tests (`settings`) |
| Add, rename, remove remotes; edit fetch URL | Repository Settings › Remotes | core tests (`settings`) |
| Clone from a URL or local path | Repository menu › Clone, empty window | — |
| Background Git with a busy indicator | Status bar spinner; actions refuse while busy | UI tests wait on it |
| Open tabs in a collapsible repository column; restore on launch | Repositories column, remembered tabs | UI tests |
| Recent repositories | Repositories column › Recent, empty window, palette | — |
| Resizable columns, remembered; restore default widths | Panel dividers; Settings › Restore default panel widths | — |
| Settings: appearance, graph colours, diff defaults, auto refresh | Settings (Ctrl/Cmd-,) | screenshot |
| Automatic refresh after outside changes | File watcher, focus refresh, Settings toggle | — |
| Command palette with fuzzy matching | Shift-Ctrl/Cmd-P | UI test, unit test |
| Embedded terminal per checkout; hide keeps shells; stop with confirmation | Terminal panel (Ctrl-`) | UI test, unit tests |

## Branches, remotes, tags, stashes

| Mac app feature | Cross-platform app | Verified by |
| --- | --- | --- |
| Local and remote branches; filter references | Sidebar Local and Remote sections, filter | UI test |
| Check out local and remote branches; tracking branch reuse | Branch menu, double-click, palette | UI test, core tests |
| Create, rename, delete branches; create at a selected tip | Toolbar Branch, branch and commit menus | UI test, core tests |
| Upstream selector with indicators and removal | Branch menu › Upstream | core tests |
| Fetch, pull, push, publish; push a selected branch | Toolbar; branch menu | core tests |
| Ahead and behind counts | Sidebar current branch, toolbar Pull/Push labels | — |
| Drag a branch onto the current branch to merge or rebase | Sidebar rows and graph labels | — |
| Merge and rebase with previews | Branch menu, confirmation shows the preview | core tests (`rewrite`) |
| Worktrees: browse, open, reveal, copy path, create, remove, forget | Sidebar Worktrees, Worktrees window | core tests |
| Submodules with state; open; check out recorded commit | Sidebar Submodules, Submodules window | core tests |
| Tags: create (annotated), inspect, delete, push, delete from remote | Sidebar Tags, commit and branch menus | core tests |
| Stashes: save (selected files, untracked), preview, apply, pop, delete | Sidebar Stashes, Stashes window, toolbar | UI test, unit tests |
| GitHub pull requests and issues (github.com) | Sidebar sections, GitHub window | unit tests |
| Copy a GitHub commit link | Branch and commit menus | — |
| Clean up merged or inactive branches, undoable | Repository menu › Clean up branches; toolbar Undo | core tests |
| GitFlow | Repository menu › GitFlow | core tests (`workflows`) |
| Git LFS | Repository menu › Git LFS | core tests (`workflows`) |
| Identity profiles | Repository Settings › Profiles, palette | core tests |

## History

| Mac app feature | Cross-platform app | Verified by |
| --- | --- | --- |
| Commit graph with stable lanes, labels, initials, working tree row | History | core tests (`graph`), screenshots |
| Keyboard navigation (Up, Down, Escape) | History | UI test |
| Load older history; filter loaded commits | History header | — |
| Search every branch's history | Search history window (Shift-Ctrl/Cmd-F) | core tests (`history`) |
| Search file contents; open blame at a match | Search file contents window (Alt-Ctrl/Cmd-F) | core tests |
| Commit inspector: message, metadata, parents, files, patches | Right panel when a commit is selected | UI test |
| Signature status | Inspector | core tests (`inspect`) |
| Restore a file to a commit's version or before it | Inspector file menu, with confirmation | UI test, core tests |
| File history following renames | Changes and inspector menus | core tests |
| Blame with age tint and ignore-whitespace | Changes and inspector menus | core tests |
| Compare commits, or a commit with working files; images | Commit menu › Compare / Mark for comparison | core tests |
| Bisect from a good commit; bar while bisecting | Commit menu, Bisect window, bisect bar | core tests, screenshot |
| Recover lost work from the reflog | Repository menu › Recover lost work | core tests |
| Export a commit as a patch; apply a patch | Commit menu, Repository menu (confirmed) | — |

## Changes and history rewriting

| Mac app feature | Cross-platform app | Verified by |
| --- | --- | --- |
| Staged and unstaged changes as a path list or folder tree | Changes panel | UI tests |
| Diffs: unified or side by side, hide whitespace, find with Ctrl/Cmd-F | Diff panel | unit tests |
| Stage, unstage, and commit; per-checkout drafts | Changes panel and commit box | UI test |
| Stage individual lines | Diff panel line selection | core tests (`staging`) |
| Discard with confirmation; undo the last 20 discards | Changes panel | UI test, core tests |
| Ignore untracked files (.gitignore or this computer only) | Changes menu › Ignore | core tests |
| Edit working files | Diff panel › Edit, changes menu | — |
| Amend, with a warning when published | Commit box | UI test (commit) |
| Edit the HEAD message | Commit menu › Edit message | UI test |
| Undo and redo commits, amends, resets, merges, rebases, pulls, cherry-picks, deletions | Toolbar, with confirmation | UI test, core tests |
| Cherry-pick one or many (oldest first); revert; choose a merge's parent | Commit menu, Ctrl/Cmd-click selection | UI test, core tests |
| Reset soft, mixed, or hard | Commit and branch menus › Reset | core tests |
| Interactive rebase: drag to reorder, pick, reword, squash, fixup, drop | Commit menu › Interactive rebase | core tests, unit tests |
| Continue or abort interrupted operations | Operation banner | UI test |
| Conflict editor with base, current, incoming; whole-file resolutions | Conflict window | UI test, core tests |

## Platform differences

- The Mac app is a native SwiftUI app; the cross-platform app uses egui and looks the same
  on every system. Menus live in the Repository picker and command palette rather than a
  macOS menu bar.
- Restoring default panel widths is a button in Settings rather than a double-click on a
  divider.
- Side-by-side diffs scroll horizontally instead of wrapping long lines.
