# NiceGit Feature Guide

NiceGit is a free Git client for macOS, Windows, and Linux, built with Rust. The goal is to make the everyday GitKraken-style workflow available without paid seats or feature gates.

**Status: 1.5.0 preview.** A working desktop client under active development, not
a drop-in replacement for every GitKraken feature. Back up important work before
trying history-changing operations. NiceGit is an independent project and is not
affiliated with GitKraken.

## Requirements

- Downloads are available for macOS 11 or newer, Windows 10 or newer (64-bit), and
  x86_64 Linux. See the [README](../README.md#download).
- Building from source needs Rust 1.95 or newer, and on Linux the development packages
  listed in the [README](../README.md#build-from-source).
- Git available on your PATH. GitHub CLI (`gh`) is optional, needed only for the
  GitHub pull-request and issue sidebar; sign in using `gh auth login` yourself.
- Cargo downloads dependencies on the first build. NiceGit has no paid tier.

Start from a clone of this repository and run the commands below from its `rust/`
folder. Keyboard shortcuts written with Command use Ctrl on Windows and Linux.
See [Contributing](../CONTRIBUTING.md), [Security](../SECURITY.md), and
[Publishing](RELEASING.md) for development and release guidance.

## Current Features

- Open any local Git repository.
- Initialize repositories and configure a repository-local commit identity.
- Add remotes through Repository Settings.
- Clone repositories from a remote URL or local path.
- Clones go into a default folder: set it under Clone into in Settings, or let NiceGit remember the parent folder of your first clone. The Clone dialog then fills in the destination from the repository's name.
- Pick one of your GitHub repositories when cloning, through the GitHub CLI (`gh`), including GitHub Enterprise hosts listed in Settings.
- Run Git operations in the background with a busy indicator.
- View local and remote branches.
- Browse linked worktree folders and branches, and open another checkout from the sidebar.
- Open a worktree, show it in the file manager, or copy its path from the worktree context menu.
- Create a linked worktree for an existing local branch from its context menu.
- Keep separate commit-message drafts for each checkout across app restarts, stored in local preferences (not sent to a server).
- Filter local branches, remote branches, and tags by name in the sidebar.
- See commit history with a parent-linked branch and merge graph.
- See the working tree connected to its actual HEAD in the graph, even when another branch has newer commits.
- Browse changed files as an expandable folder tree or a flat path list.
- Load older history and search loaded commits by message, author, hash, or reference.
- Choose which branches the graph shows from the "Branches shown" menu in the history header:
  all branches (the default), local branches only, or the current branch and its upstream. A
  note above the graph says what is shown, with a link back to all branches. Searching all
  history still covers every branch. Individual branches can also be hidden from the graph
  from their right-click menu, in the sidebar or on the graph's branch label. Commits that
  other shown branches reach still appear, and hidden branches are remembered for each
  repository.
- Review staged and unstaged file changes.
- Filter changed files by path in the Changes panel once there are more than a few: type a few words, or press Command-Shift-L, and both lists show only matching files. Stage all and the other bulk buttons still act on every file, and say so while a filter is on.
- Inspect color-highlighted diffs for staged, unstaged, and untracked files.
- Search diffs with previous/next matching-line navigation and Command-F.
- Select a commit to inspect its metadata, changed files, and patches in a persistent side panel. The inspector also lists the branches that contain the commit and the first tag that includes it.
- Read the complete commit message, including multiline descriptions, in the inspector.
- Restore a file from a commit's changed-file list to that commit's version, or to its
  version before the commit, then review and commit the staged result.
- Amend the last commit with staged changes from the commit panel, with a warning when it
  is already on a remote.
- Add trailers from the commit box's menu: Signed-off-by from your Git identity, and
  Co-authored-by from recent authors (the current user and bots left out). The menu edits
  the description text, so the exact lines show before you commit.
- An empty commit message starts from the repository's commit.template, with comment
  lines removed. As with Git, an unedited template cannot be committed.
- Show diffs side by side (old left, new right, long lines wrapped) or in one column; the
  choice is remembered.
- See what a merge or rebase would do before confirming it: already up to date, a
  fast-forward, a clean merge with the number of files that would change, or the files
  expected to conflict. Rebase predictions are marked as estimates.
- Refresh automatically when files, staging, branches, or tags change outside NiceGit,
  such as edits in another app or commits and fetches in a terminal (Settings to turn off).
- Resize the repositories column and the branches sidebar by dragging their dividers;
  double-click a divider to restore its default width. Widths are remembered.
- Undo steps back through the last 20 NiceGit actions that moved the current branch (commit,
  amend, reset, merge, rebase, cherry-pick, revert, or pull), deleted branches, or deleted
  stashes, and Redo steps forward again, from the toolbar or the command palette. The arrow
  beside Undo lists the steps it will take back. A restored stash comes back as the newest one.
- Clean up branches (Repository > Clean Up Branches): delete local branches merged into the
  current branch, and optionally unmerged ones with no recent commits, as one undoable step.
- Search file contents (Option-Command-F, or "Search files in this commit" in the graph):
  find literal text in tracked files at any commit or in the working files, and open blame at
  a match.
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
  oldest first. Squash… and Drop… open the interactive rebase window with the selected
  commits arranged, so you can review them before clicking Rewrite commits. Revert… undoes
  them with new commits, newest first, in one sequence.
- Undo a discard from the Changes panel: the file's staged and unstaged versions are
  saved before discarding and restored exactly, until the file changes again.
- See submodules in the sidebar with their state (not checked out, at the recorded commit,
  or on another commit, and whether they have uncommitted changes); open one, or check out
  its recorded commit.
- See whether a commit's GPG or SSH signature is verified, untrusted, bad, or cannot be
  checked on this computer, in the commit inspector.
- Hide whitespace-only changes in any diff; the choice is remembered.
- Choose 3, 10, or 25 context lines, or the whole file, for every diff; line staging works with
  each. File history, blame, and compare follow the setting too.
- Compare changed images side by side with their dimensions and file sizes.
- Recover lost work from Repository > Recover Lost Work: every position HEAD has had,
  with commits on no branch marked, and a branch can be created at any of them.
- Stash only selected files (including untracked ones) from the Stashes window.
- Remove a clean linked worktree, and forget worktrees whose folders were deleted.
- Command palette (Shift-Command-P) for repository actions, branch switching, and recent
  repositories, with fuzzy matching.
- Go to file (Command-P, Control-P elsewhere) finds a tracked file by name, with fuzzy matching
  and file names ranked first. Enter opens its file history, Command-Enter its blame, and
  Option-Enter (Alt-Enter elsewhere) the built-in editor. Up and Down move through the matches,
  and Escape closes the finder. The right-click menu also reveals the file or copies its path.
- Search every branch's history by message, author, or code change (Shift-Command-F),
  including commits not yet loaded in the graph, or paste a commit ID.
- Interactive rebase from a commit's context menu: drag to reorder, and pick, reword,
  edit, squash, fix up, or drop each commit, with a warning for commits already on a remote.
  Edit stops the rebase after that commit so you can change its files; Continue folds in
  what you have staged. A row whose choice is not Pick says what that choice does.
- Compare any commit with the working files, or two commits with each other (mark one,
  then compare from another), with a changed-file list and per-file diffs.
- Blame a file (working version or at a commit), with author, age tinting, an
  ignore-whitespace option, and each line's commit change one click away. A line's
  right-click menu blames the file before that line's change ("Blame before this change"),
  and Back returns to the newer blame. "History of these lines" lists every commit that
  changed a group of consecutive lines from one commit, each with its change to just those
  lines.
- Rename or remove remotes and edit their fetch URL in Repository Settings. A separate push URL
  can be set, changed, or cleared so pushes use the fetch URL.
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
- Fetch a single remote from its right-click menu in the sidebar, instead of every remote.
- Pull only fast-forwards. When the branch and its upstream have both moved on, Pull explains
  why it stopped and offers Merge or rebase… to integrate the upstream's commits.
- Show outgoing and incoming commit counts against the configured upstream.
- Keep recently opened repositories in the sidebar.
- Refresh repository state when returning to the app, unless an operation or review dialog is active.
- Save, apply, pop, and delete stashes, including untracked files. Pop removes the
  saved stash only after applying succeeds; failed applies retain it. The Stash dialog can
  keep staged changes in place (Keep staged changes) and leave untracked files out (Include
  untracked files, on by default); both choices are remembered. Create branch from stash…
  in a stash's right-click menu makes a branch at the commit the stash was made on, applies
  the stash there, and removes it once it applies.
- Preview a stash's tracked and untracked changes before applying it. Selecting a stash lists
  its files in the inspector; click one to see just its changes, or restore that file from the
  stash.
- Merge or rebase from branch context menus; cherry-pick from commit context menus.
- The Merge confirmation can always create a merge commit, or squash the branch's changes into
  the staged changes so you can commit them as one new commit. A squash that would conflict is
  refused, so you can merge normally and resolve the conflicts.
- Continue or abort interrupted operations after resolving and staging conflicts.
- Edit UTF-8 text conflicts in-app, with marker checks and external-edit protection.
- Compare base, current, and incoming conflict versions alongside the editable result.
- Resolve whole-file conflicts by keeping either side or deleting the file, including binary files.
- Take either side of a conflicted file from its right-click menu in the Changes panel, or of
  every conflicted file at once from the Resolve all menu beside Continue and Abort.

## Run

```sh
cd rust
cargo run -p nicegit --release
```

## Tests

```sh
cd rust
cargo test --workspace
```

`rust/scripts/test-all.sh` runs the same tests on macOS and Linux at the same time. See
[rust/README.md](../rust/README.md).

## Behavior and Limitations

The toolbar's Undo and Redo cover the actions listed under Current Features and step through
the last 20 of them in turn. A new action ends the redo steps, and a change made outside
NiceGit stops the chain: the next Undo is refused, and the older steps are dropped. For a
commit, Undo uses soft reset and preserves the index and working files; Redo restores that
commit.
Both confirm before changing local history and reject a changed branch or HEAD. This is
not a general undo stack for all Git operations, and it does not persist across restarts.

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

Undo is offered only while the branch still points where the action left it. Commits and
amends undo softly, keeping their changes staged; soft and mixed resets undo in kind; other
actions undo with `git reset --keep`, which keeps unrelated local edits and refuses to
overwrite any. Edits discarded by a hard reset cannot be restored. Deleted branches come back
at their old commits with their upstream settings, and only if no branch has taken the name.
Branch clean-up never lists the current branch or branches checked out in other worktrees,
selects merged branches only by default, and deletes unmerged ones only after a separate
choice, each atomically against the tip that was listed.

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
on demand from a selected GitHub remote using an installed, signed-in GitHub
CLI (gh). NiceGit reuses that authentication and does not store a separate token.
The sections support open/closed/all state selection (plus merged PRs), filtering
loaded titles/authors/numbers, refresh, cancellation, links to GitHub, and loading up to
1,000 items in batches of 100. Counts reflect loaded items, not a server-side total.
GitHub Enterprise Server hosts can be listed in Settings, and their remotes then work like
github.com remotes, including gh-based lists that use your `gh auth login --hostname` sign-in.
Other hosting providers aren't supported.

Branch context menus provide checkout, fetch, current-branch pull/push, selected-branch
push to an explicit same-named remote branch with confirmation, an upstream
selector with current-tracking indicators and removal,
worktree creation, branch creation at the selected tip without switching checkouts,
merge/rebase, safe local rename/delete, deleting a remote branch from its remote
(checked against the tip shown and the remote's address, and refused if the remote moved
since your last fetch), and copying the branch name or full commit SHA. They do not yet reproduce every GitKraken action.
The Copy GitHub commit link submenu uses an explicitly selected remote and a full
commit hash. Only GitHub remotes are supported (github.com, and the Enterprise hosts listed
in Settings); creating a link neither publishes the commit nor verifies that it already
exists on the remote. Commits, branches, and the files in a commit can also be opened on
GitHub from their menus; with several GitHub remotes, you choose which one. Opening does not check that the commit or branch
has been pushed.

Create pull request… in a branch's menu, or the command palette's Create pull request on GitHub
entry, opens a dialog for a local branch whose upstream is on a GitHub remote. NiceGit creates the
pull request through your GitHub CLI login, so it works for github.com and the Enterprise hosts
listed in Settings. The dialog reads the repository's default branch and fills in the base,
which you can change. The title is the subject of the branch's only commit ahead of the base, or
the branch name in words. The body lists the commit subjects when there are two to twenty of them.
You can edit all three, and choose a draft pull request. A warning appears when the branch has
commits its upstream does not have, since the pull request will not include them. Until the
pull request is created, nothing on GitHub changes. When it is created, NiceGit shows its number
in the notice bar, with an Open button that opens it in your browser. If gh is missing or signed
out, the dialog says how to fix that.
Open in browser instead is the fallback: it opens GitHub's compare page, as before, and creates
nothing.

The status bar shows the current branch's pull request when its upstream is on a GitHub remote
and gh is installed and signed in: its number and state (open, draft, merged, or closed), and its
checks as a count of failing, pending, or passing ones. Clicking it opens the pull request in your
browser, and hovering shows its title and the check totals. The status is read through gh when a
repository opens, after fetching, pulling, pushing, publishing, or creating a pull request, and
every five minutes while NiceGit is focused. Without gh, or when signed out, the status bar shows
nothing.

Force push… in the current branch's menu replaces the remote branch with yours, for example
after you rewrite commits that were already pushed. The confirmation lists the remote commits
it will remove. The push uses a lease: it is refused, and nothing changes, if the remote
branch moved since your last fetch.

Create lightweight or annotated local tags from a commit or branch menu. Annotated
tags require a message and use the repository's Git identity. A tag can be pushed to a
chosen remote as it is created, with Push to remote in the New tag dialog, or later from
its sidebar menu, and it can be deleted from a remote there. An annotated tag can be signed
with "Sign with your signing key", which uses your Git signing setup (user.signingKey and
gpg.format) as `git tag --sign` does; unsigned tags stay unsigned even when tag.gpgSign is
set. Hovering a tag in the sidebar shows an
annotated tag's tagger, date, and message, or a lightweight tag's commit. A signed tag's
signature is checked locally and shown as verified, untrusted, bad, or not checkable, like
a commit's.

Commit menus support revert with confirmation, merge-parent selection, and
conflict Continue/Abort. Revert requires a clean working tree and retains the
original commits.

Cherry-pick asks for confirmation and lets you choose the baseline parent for a
merge commit. The destination branch and HEAD are checked before starting. The
confirmation can also note the original commit in each new message, as
`git cherry-pick -x` does, which suits backporting a fix; the choice is remembered
in Settings. It applies to a merge commit too, where the parent choice has the
same checkbox. A revert does not offer it.

The commit graph and branch context menus offer soft, mixed, and hard reset with a confirmation explaining
the index and working-file effects. Hard reset explicitly warns about losing
uncommitted changes, including obstructing untracked paths. The destination branch
and HEAD must still match the confirmed checkout, and no Git operation may be in
progress. Reset does not push or modify remote branches.

Create a shareable `.patch` file from a non-merge commit using its graph context
menu. The export includes author/message metadata and binary changes and does not
change the repository. Merge commits and commits without an exportable patch are
not supported by this action.

Save as ZIP… from the same menu writes the commit's files as a `.zip` archive, with
them inside a top folder named after the repository and the short hash, as GitHub's
Download ZIP does. Submodule contents are not included, and files marked
`export-ignore` in `.gitattributes` are left out. The archive does not change the
repository.

The Changes panel saves uncommitted work as a `.patch` file: the staged or unstaged
changes from a section's icon button, or one changed file from its right-click menu.
The unstaged export includes untracked files, and a section's button covers every file
in it, including files hidden by the filter. An unstaged patch is made against the index,
so it applies to a checkout of HEAD when nothing is staged; Apply Patch or `git apply` can
apply it.

Use Repository > Apply Patch to choose a patch file and confirm the destination
checkout. Git checks the captured patch before applying it; resulting changes
remain unstaged and uncommitted. Invalid patches and paths outside the repository
are rejected. Finish any existing merge, rebase, cherry-pick, or revert first.

Edit the current HEAD commit message from its graph menu, including the message
body. Amending changes the commit hash and requires confirmation; staged and
unstaged file edits are excluded. Older commits on the current branch can be edited from
their graph menu too: Edit message… opens the interactive rebase window with that commit set
to Reword, which rewrites it and the commits after it, so their hashes change.
Fix up with staged changes… in the same menu folds what you have staged into an older
commit. It needs staged changes and no unstaged ones (untracked files may stay). The
commits from that one through HEAD are rewritten, so their hashes change, and Undo restores
them with the folded changes staged again.

Remote checkout reuses a unique existing local tracking branch, including renamed
branches, without resetting local commits. Ambiguous tracking branches must be
selected explicitly from Local.

NiceGit has no accounts or sign-in of its own: fetching, pulling, and pushing use your
Git credentials, and the GitHub sidebar uses your GitHub CLI login.

## Embedded Terminal

The toolbar's Terminal button, Control-backtick, and the command palette's Show terminal
entry open a resizable bottom panel. Hiding it refreshes repository state so changes made
in the shell appear in the workbench.
Each checkout gets its own shell starting in that checkout's directory. Hiding
the panel keeps its shell running for the current app session. Use the stop
button to end a session, with confirmation; an exited shell can be restarted.

## License

NiceGit is released under the MIT License.
Dependencies retain their own licenses; see [Third-Party Notices](../THIRD_PARTY_NOTICES.md).
