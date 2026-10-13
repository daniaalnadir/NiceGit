# Changelog

## 1.6.0 - Tidy tabs

- Middle-click a repository tab to close it, as in a browser.
- Command-W (Control-W elsewhere) closes the open window, such as the editor, or else the
  repository tab; a window with unsaved text stays open.
- A repository tab's right-click menu can close it or every other tab, copy its path, or
  show it in the file manager.
- Clicking a tab's name now switches to it; before, only the tab's edges responded.
- Every download includes THIRD_PARTY_LICENSES.txt, the license texts and copyright notices of
  the open-source components built into NiceGit.
- Command-F keeps the cursor in the diff's find field when line staging finishes loading just
  after it; before, the field could lose focus.
- The bisect bar no longer flickers, with its buttons briefly gone, on every refresh.
- Edit message… now works on older commits too: it opens the interactive rebase window with
  that commit set to Reword.
- Open a commit, branch, or a commit's file on GitHub from its right-click menu.
- Fix up with staged changes… folds what you've staged into an older commit from its
  right-click menu; Undo puts the changes back.
- The New tag dialog can push the tag to a remote as soon as it is created.
- An annotated tag can be signed with your signing key from the New tag dialog, as
  `git tag --sign` does.
- An Add trailers menu in the commit box adds Signed-off-by and Co-authored-by lines,
  choosing co-authors from recent history.
- Selecting a stash lists its files in the inspector; click one to see just its changes, or
  restore it from the stash.
- Ignore whitespace now applies to stash diffs too.
- Filter the Changes panel's files by path once there are more than a handful; Stage all still
  stages every file.
- Force push… replaces the remote branch after you rewrite pushed commits, listing what it
  removes and refusing if someone else pushed since your last fetch.
- A hovered change keeps its Stage and Discard buttons while a refresh reloads the diff;
  before, they vanished until the reload finished.
- GitHub Enterprise: list your hosts in Settings and their remotes get pull requests, issues,
  and Open on GitHub like github.com.
- Save as ZIP… exports a commit's files as a ZIP archive from its right-click menu.
- An empty commit box starts from your commit.template, without its comment lines; like Git,
  it must be edited before it can be committed.
- Choose whether the commit graph shows all branches, only local branches, or just the current
  branch and its upstream.
- Hide a single branch from the commit graph from its right-click menu, in the sidebar or on
  its label; commits other shown branches reach still appear.
- Choose how many unchanged lines diffs show around each change, up to the whole file. The
  setting also applies to file history, blame, and compare.
- Choose how many unchanged lines diffs show around each change, up to the whole file.
- Save staged or unstaged changes, or one changed file, as a patch from the Changes panel.
- Interactive rebase can stop at a commit (Edit) so you can change its files; Continue folds
  the staged changes in.
- Delete a branch from its remote from the branch's right-click menu; it is refused if the
  branch moved since you last fetched.
- The Clone dialog can list your GitHub repositories through the GitHub CLI, so you can pick
  one instead of pasting its URL.
- In a narrow Changes panel the section titles and bulk buttons shorten so the file counts
  stay visible, and the diff's line-staging hint wraps instead of being cut off.
- Undo and Redo step back and forth through the last 20 recorded actions instead of only the
  most recent one, each named by its commit or target.
- In Blame, right-click a line to blame the file as it was before that line's change, and
  step back with Back.
- Create pull requests from the branch menu or command palette: NiceGit fills in the title,
  base, and commit list, and creates it through the GitHub CLI.
- Resolve conflicts faster: take the current or incoming version of a file from its right-click
  menu, or of every conflicted file at once.
- Clones go into a default folder, remembered from your first clone or set in Settings, so the
  destination fills itself in.
- Go to file (Command-P) finds any tracked file by name and opens its history, blame, or
  the editor.
- Interactive rebase shows what each row's choice will do.
- Hovering an annotated tag in the sidebar shows its message, tagger, and date.
- Squash or drop the commits you've selected in the graph: the interactive rebase window opens
  with them arranged for review.
- Saving a stash can keep your staged changes in place or leave untracked files out; the
  choices are remembered.
- Revert several selected commits at once, newest first, from the graph's selection bar.
- In Blame, see the history of a group of lines: every commit that changed them, with each
  change to just those lines.
- The status bar shows the current branch's pull request and how its checks are doing; click it
  to open the pull request.
- Refreshes no longer recount every branch's ahead and behind commits when no branch or
  upstream moved, which saved about 0.2 s of background work per refresh in a repository
  with 200 tracked branches.
- Refreshes skip the bisect and submodule checks in repositories that are not bisecting or have
  no submodules.
- Saving a file in another app refreshes only the changed-file list when no branch, commit, or
  operation changed, instead of reloading the whole history.
- Commits, staging, and branch switches made in another app in a linked worktree now appear
  without waiting for the periodic refresh.
- Create branch from stash… from a stash's right-click menu makes a branch at the commit the
  stash was made on, switches to it, and applies the stash there. The stash is removed once
  it applies.
- The Merge confirmation can always create a merge commit, or squash a branch's changes into
  the staged changes to commit as one new commit. A squash that would conflict is refused.

## 1.5.0 - Fewer surprises

- A changed file of 50 MB or more shows its size with a warning in the Changes panel, before
  it is committed by mistake; GitHub refuses files over 100 MB.
- Discard the open file's changes from the diff's header, after confirming; the discard can
  be undone from the Changes panel.
- When a push is refused because the remote has newer commits, the message says so plainly
  and offers Merge or rebase…, which fetches and opens the usual choice with its preview.
- A commit Git refuses because it does not know your name and email is explained, with Set
  your identity… to open Repository Settings.
- A commit that fails gives its message back to the commit box; before, it was lost.
- A remote that cannot be reached, refuses your sign-in or SSH key, or cannot be found is
  explained in plain words, with Git's own message in the tooltip; the Couldn't fetch note
  explains it too.
- Away from any branch, as after checking out a tag, the commit box warns that a commit there
  could be lost and offers Create branch here….
- A pull, merge, or other change stopped because it would overwrite uncommitted changes is
  explained, with Stash changes… to set them aside first.
- Stashing from the toolbar works without a message, as its dialog says; OK used to stay
  disabled until one was typed.
- A merge, rebase, cherry-pick, or revert that stops at conflicts says so and what to do next,
  instead of reporting that Git exited with status 1.
- A branch or tag name Git refuses or that is already taken, a clone into a folder that is not
  empty, and a tag the remote would not replace are explained in plain words.
- New branch, rename, and tag dialogs check the name as it is typed, say what Git would not
  accept, and keep OK disabled until the name is valid.
- The original Swift app for macOS is removed from the repository; NiceGit is the Rust app
  for macOS, Windows, and Linux.

## 1.4.0 - Sizes everywhere

- A commit's files in the inspector show how many lines each adds and removes, as changed
  files do in the Changes panel.
- Each repository tab shows how many uncommitted changes it has, so work in progress is
  visible in every open repository.
- Set an external editor in Settings, such as `code` or `zed`, and open changed files in it
  from their menu or the diff's header.
- Stage or unstage the open file from the diff's header, or with Shift-Command-S; the file
  stays open on its other side instead of closing.
- With a summary written and nothing staged, Stage all and commit stages every change and
  commits it in one step.

## 1.3.0 - Line by line

- Each added or removed line in a diff has a + to its left, before the line numbers, that
  stages just that line; in a staged diff a − unstages it.
- New files can be staged line by line, and their changes staged or discarded from the diff,
  like changes to tracked files.
- With lines selected, Discard selected lines puts them back after confirming; the discard
  can be undone from the Changes panel.
- Make everything larger or smaller with Command-plus and Command-minus (Control elsewhere),
  or under Layout in Settings; the size is remembered, and the panels are fitted to the
  window again when it changes.
- Restore default panel widths also restores the history's column widths.
- Reuse a recent commit summary from the button beside the summary's character count.
- When the toolbar's button fetches, a badge on it counts the commits waiting to be pulled.
- Copy the open diff as a patch from the button in the diff's header.
- Scrolling to the end of the history loads older commits by itself; the Load older history
  button stays for a history that fits without scrolling.
- Every local branch in the sidebar shows how far it is from its upstream (↑ to push, ↓ to
  pull), or that its upstream is gone from the remote; before, only the current branch did.
- Clean Up Branches also lists branches whose remote branch was deleted, as after a merged
  pull request, marked Upstream gone; unmerged ones still need Include unmerged to delete.

## 1.2.0 - At a glance

- Each changed file shows how many lines it adds and removes, such as +12 −3, or that it is
  binary; new files count their lines too.
- With more than ten changed files, a filter narrows the Changes panel by path; each list
  shows how many of its files match.
- Open a changed file in its default app from its menu in the Changes panel.
- Drag a folder onto the window to open it as a repository.
- Create a pull request on GitHub for a published branch, from the branch's menu or the
  command palette; it opens GitHub's compare page in your browser.
- Command-1 to Command-9 (Control elsewhere) go to the repository tabs, Command-9 to the last.
- A changed image shows its versions side by side in the diff, with their dimensions and
  file sizes, for unstaged and staged changes and for a commit's files.
- Commit and push in one step from the commit box, or with Shift-Command-Enter
  (Shift-Control-Enter elsewhere), on a branch with an upstream. If the push fails, the
  commit is kept and the message says so.

## 1.1.0 - Change by change

- Hover a change in a diff to stage it, unstage it, or discard it on its own. Discarding asks
  first and can be undone from the Changes panel.
- Staging individual lines now works in files of any length; before, it was available only
  when the whole file fit in the diff.
- Step through the changed files from the open diff with its arrows or Option-Up and
  Option-Down (Alt elsewhere), on through the staged files.
- A file stays open after staging part of it, showing what is left.
- The open repository is fetched in the background every 15 minutes, so ahead and behind
  counts stay current. The status bar shows when it last fetched, or quietly says it could
  not. Choose how often, or turn it off, in Settings.
- Every keyboard shortcut is listed in one place: Command-/ (Control-/ elsewhere), the
  command palette, or the link in Settings.
- The diff's view controls move to a second row in a narrow panel, so the file's name keeps
  its room.

## 1.0.0 - NiceGit everywhere

The first release of NiceGit for macOS, Windows, and Linux, rewritten in Rust with every
feature of the Mac app. Earlier versions below were development builds and were not
released.

- Everything the Mac app does, from staging single lines and resolving conflicts to
  interactive rebase, bisect, worktrees, GitFlow, and Git LFS. `docs/PARITY.md` lists each
  feature and the tests behind it.
- A file's diff fills the main area in place of the history, side by side when there is room.
- Fetch and Pull share one toolbar button whose menu chooses what it does.
- The history's columns resize from their headings and are chosen from the headings' menu.
- A universal Mac app (Apple silicon and Intel), a Windows zip, and Linux `.deb` and
  `.tar.gz` packages.

## 0.7.1 - Your columns

- The commit history has Branch / tag, Graph, Commit message, Author, Date, and Commit
  columns. Drag a heading's edge to resize its column; double-click the edge to restore it.
- Right-click the headings to show or hide columns, or restore every column's width. The
  choices are remembered.
- A narrow history leaves out Author, then Date, then Commit, until there is room again.

## 0.7.0 - Room to read

- A file's diff takes the whole main area in place of the commit history. History, or
  Escape, goes back, keeping the commit or working tree it came from selected.
- Choosing the working tree lists its changes without opening the first file.
- Fetch and Pull share one toolbar button. The arrow beside it chooses which one the button
  does, and the choice is remembered.

## 0.6.20 - Soft edge

- The commit history fades out at its bottom edge while more commits are below, so a row cut
  by the diff panel reads as more history rather than a broken row.

## 0.6.19 - Whole names

- A diff file header too long for its row is shortened in the middle of its paths, at a folder
  boundary, so the file's name is never cut.

## 0.6.18 - Tidy edges

- Long file headers in a diff stay on one line and end in an ellipsis, rather than wrapping
  through the file's path.
- Search fields show their magnifying glass inside the field, lined up with the text, and the
  diff's find field lines up with the staging hint above it.
- Settings' warning that side-by-side diffs are not in effect is set at body size.

## 0.6.17 - One band

- A selected commit's tint spans the whole row, labels and graph included.

## 0.6.16 - Quieter history

- Only the selected or hovered commit row is tinted; every row keeps its lane-coloured bar,
  so the selection stands out.
- Small warning text in the light theme is darker, for readable contrast.
- The editor closes from its window's own close button, without a second Close button.

## 0.6.15 - Settings that explain themselves

- When "Show diffs side by side" is on but the diff panel is too narrow, Settings says so in
  the warning colour and how to change it.
- Interface tests leave temporary repositories for the Windows runner to discard; deleting
  them while handles were still closing could hang a CI run.

## 0.6.14 - Aligned

- The history's column headings line up with their columns: labels, the first graph lane,
  and the commit text.

## 0.6.13 - Every control readable

- Checkboxes have the same clear outline as radio buttons.
- Settings explains that side-by-side diffs need a diff panel at least 560 points wide.
- The editor has a divider between its line numbers and the text.
- Tests check that the Repository Settings window, as well as Stashes, holds refreshes.

## 0.6.12 - Clear controls

- Radio buttons have a clear outline, so unselected choices read as controls in the light
  theme.
- A watchdog stops the test run with a hung interface test's name, last step, and (on
  Windows) every thread's stack.

## 0.6.11 - Refresh like the Mac app

- As in the Mac app, automatic refreshes and the refresh for returning to NiceGit wait while
  the Stashes, Repository Settings, or conflict window is open, or the editor holds unsaved
  text, and run once it closes.
- The history filter field has a border in the light theme.
- Signature colours are tested for every state.

## 0.6.10 - Clear choices

- Only the Split choice is dimmed when a diff cannot be split; the active Unified choice keeps
  full emphasis.
- Tests now open repositories from the Repository menu and the empty window, check every
  signature state's wording, colour, and tooltip, and check that returning to NiceGit during
  an action refreshes once the action finishes.

## 0.6.9 - Honest controls

- When a diff can only be shown in one column, the Split and Unified control shows Unified as
  active instead of the saved Split choice, which still applies when there is room again.

## 0.6.8 - Room to read

- A diff panel narrower than 560 points shows changes in one column, since each side would
  wrap most lines; the Split control says to widen the panel.
- Long branch names in the graph drop their prefix first ("…/hourly-forecast"), keeping the
  part that tells branches apart.
- The diff's scroll bars appear only when it overflows.

## 0.6.7 - Nothing clipped

- Diff rows are as tall as their wrapped text measures, so the last row of a long wrapped
  line is no longer clipped; the diff has a solid scroll bar that shows when it continues.
- The history filter's placeholder fits, with what it matches in a tooltip.

## 0.6.6 - Last details

- Long branch names in the graph lose their middle rather than their end, and hovering a
  label shows the full name.
- The diff's scroll bar stays visible, floating windows in the dark theme have a lighter
  edge, and an unverifiable signature uses the Mac app's secondary colour.
- Tests now cover opening a repository with Command-O, the search shortcuts, refreshing
  after a running action, the Fetch and Refresh buttons, and diff colours. An independent
  audit found all 71 of the Mac app's features implemented.

## 0.6.5 - Every detail checked

- Commit signatures read as in the Mac app, name the signer only when verified, and show the
  key and any problem in a tooltip.
- The diff header's controls keep their identity while a diff reloads, so a click or screen
  reader action on Edit no longer misses during a reload.
- Diff header lines align with the code text. Disabled toolbar buttons are legible in the
  light theme, the commit summary counter reads "used/72" with a tooltip, and the editor
  shows a file's path only when it adds to the title.
- Interface tests now cover the remaining details of the Mac app's features, from creating
  and cloning repositories to stash pop, conflict panes, and relaunch memory.
- `rust/scripts/test-all.sh` runs the tests on macOS and in a Linux container at once,
  optionally several times, in a few minutes instead of waiting on CI.

## 0.6.4 - Truer side-by-side diffs

- In a side-by-side diff, a short line beside a long wrapped one tints only its own rows;
  the rest is neutral, so a one-line deletion no longer looks like a longer one.
- Floating windows cast a stronger shadow in the dark theme.
- Interface tests now cover undoing a pull, publishing from the palette, searching a
  commit's files, submodule checkout, signature display, and rebasing by dragging a graph
  label.

## 0.6.3 - Fixes from deeper testing

- Image sizes from 1 KB to 1 MB were labelled as bytes when comparing; they now read in KB,
  and larger sizes in the right unit.
- Filtering references reveals matching branches under a collapsed remote.
- Undo and Redo from the command palette ask first, as the toolbar's buttons do, and are
  offered only when they apply to the current branch.
- Word highlights on wrapped diff rows stay apart, and floating windows cast a deeper shadow.
- 32 more interface tests cover the remaining details of the Mac app's features.

## 0.6.2 - Easier reading

- Diffs wrap long lines between words, measured from the text layout itself, so every row
  is exactly as tall as its wrapped text.
- Compact toolbar buttons show their icon alone instead of a label squeezed into a narrow
  cell; labelled buttons are as wide as their labels. Group rules are clearer in the light
  theme.

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
