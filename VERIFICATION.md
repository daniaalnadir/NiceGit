# NiceGit Verification

## Repository Action Bar

RepositoryActionBar replaces the old large header with repository/branch menus
and labeled Fetch, Push, Branch, Stash, Pop, Terminal, and Refresh controls.
Fetch has a separate dropdown containing fast-forward Pull. Native inspection
verified the compact bar and dropdown; Fetch/Pull are correctly disabled in the
preview checkout without a remote/upstream. The layout can wrap selectors above
actions when horizontal space is limited. Git undo/redo is not implemented and
has no placeholder buttons. Native inspection with both sidebars expanded showed
the single-row controls still fitting without overlapping. Clicking Terminal
produced no NiceGit error, but computer-use access to com.apple.Terminal was
explicitly denied, so its working directory was not visually verified. The
two-row fallback still needs a narrower-window check. The full regression suite
passed with 46 declared tests and the optional live GitHub test skipped.

Subsequently, Undo/Redo was added for the last non-initial NiceGit commit in the
current session. It uses soft reset, captures branch and before/after hashes, and
requires confirmation. The earlier note about absent Undo/Redo describes the
initial action-bar build. General Git-operation undo and restart persistence are
not supported by this commit-only control.

## Reset Modes: 15 September 2026

Branch context menus also expose reset of the current checkout to the selected
branch's captured tip. Branch and graph entry points share ResetConfirmation and
the same expected-branch/HEAD checks. Native inspection of main's context menu
showed all three modes. Mixed reset named feature/conflicts as the destination
and 6bf0bc16 as the target, with index/working-file effects explained. Cancelling
retained HEAD f5417e1 and the same three changed files.

The commit graph now exposes soft/mixed/hard reset with mode-specific confirmation,
including a destructive hard-reset action and warning about obstructing untracked
paths. The core validates the captured branch/HEAD and absence of an operation.
A real-repository test passed for all three modes: HEAD moved to the selected
commit, soft preserved staged content, mixed reset the index while preserving
working files, and hard restored tracked content. Stale expected HEAD was rejected
without moving HEAD or changing working files. Release packaging passed. Native
inspection showed all three reset modes and the hard-reset warning for a802aff,
including potential deletion of obstructing untracked paths. Cancelling retained
HEAD f5417e1 and three changed files; no user checkout was reset. Soft and mixed
mode behavior is covered by the integration test, not separate native execution.

## Cherry-Pick Confirmation: 15 September 2026

The commit menu now confirms cherry-pick and offers a mainline parent for merge
commits. It captures destination branch/HEAD for the existing stale-checkout guard.
The merge integration test passed after reverting a merge then cherry-picking its
first-parent changes, restoring feature.txt while retaining main.txt and leaving
no operation in progress. Native inspection of merge 5d6b25d showed both parent
choices (d40f3c41 and cac48dd9), the destination feature/conflicts, and the baseline
explanation. Cancelling left the preview checkout unchanged.

The complete suite passed on 15 September with 45 declared tests, including the
new patch and stale-checkout cases; the optional live GitHub test was skipped.

## Patch Import: 15 September 2026

Repository > Apply Patch uses a native file picker and destination confirmation.
The core captures file bytes into one temporary patch used for both Git's check
and apply commands, without enabling partial application or unsafe paths.
The real-repository round-trip test passed for text and binary changes, repeated
application rejection, malformed content, and parent-directory traversal rejection.
Native inspection selected the earlier exported patch through the file picker,
which displayed its contents. The confirmation named preview-repository's full
path and explained that changes remain unstaged without a new commit. Cancelling
returned to the workspace with the same three changed files and HEAD f5417e1.
Actual application remains covered by the real-repository integration test rather
than a mutation of the preview checkout during this UI check.

## Commit Patch Export: 14 September 2026

The graph commit menu opens a native save panel for a `.patch` export generated
by Git format-patch. A real-repository test exported a root commit containing text
and binary files, applied it to a separate empty repository, and compared restored
contents byte-for-byte. The source HEAD and clean status were unchanged. That
test passed. Native save-panel testing exported commit 6bf0bc1 to a temporary file;
`git apply --stat` recognized preview.txt with two insertions. Testing an empty
root commit exposed message-only Git output, so the exporter now checks for actual
file changes before exporting. The regression test also verifies rejection of an
empty commit. Merge commits are disabled in the menu and rejected by the core
exporter. The temporary exports do not change the source repository.

## Branch Integration Confirmations: 14 September 2026

Sidebar Merge and Rebase commands now request confirmation and name the source,
destination, and selected tip. Rebase explicitly warns that commit IDs change.
The confirmed target is the selected commit hash rather than a moving branch
reference. Both actions are disabled during an existing Git operation.
The release package built successfully. Native inspection verified both dialogs
for main into/onto feature/conflicts; both were cancelled without changing history.

The confirmation captures the destination branch and HEAD as well as the selected
source tip. Before starting, GitClient checks these expectations against a fresh
snapshot and rejects stale confirmations. A real-repository regression test passed
for switching to a different branch at the same hash, advancing the original
branch, and accepting an unchanged checkout. Merge and rebase rejection leaves
HEAD and operation state unchanged. The release package rebuilt successfully.

## Separate Repository Sidebar: 14 September 2026

The packaged release was checked with preview-linked and preview-repository open.
The repository list occupies a separate leftmost panel. Collapsing it hides the
list while keeping branch navigation and the graph visible. After quitting and
reopening NiceGit, the panel remained collapsed and preview-repository was the
active checkout. Expanding restored both tabs in their original order.

The stash dialog exposes separate Apply, Pop, and Delete controls. The Pop
confirmation and its failure-retention warning were inspected and cancelled;
the existing preview stash was not applied or deleted. Real temporary-repository
tests cover successful Pop restoring staged, unstaged, and untracked changes,
removing the saved stash, rejecting a missing stash, and retaining a stash when
application fails against conflicting committed content.

The current `swift test` run passed on 14 September: 43 declared tests, with the
optional live GitHub test skipped. The release package built successfully before
the native checks above.

## Open Tabs and Draft Restart Check

The sidebar now lists open repositories separately from recents, with active
highlighting and individual close controls. A model test verifies deduplication,
inactive/active/final-tab closure, neighbour selection, and draft preservation.
Native UI inspection verified two preview tabs and switching between them.

A temporary preview commit draft survived quitting and relaunching the packaged
app, then was cleared through the UI. The HEAD-message editor and its rewrite
warning were visually inspected with a multiline message and cancelled without
amending history. The release build and tab-specific test passed after the full
41-test run (which skipped the optional live GitHub test).

## GitHub Sidebar

The full suite passed with the optional live GitHub test skipped. The live test
was then enabled separately and passed for both PR and issue loading from the
public cli/cli repository. Remote-address and response-link validation have
offline tests. Native UI inspection of the packaged app displayed 56 open PRs,
including draft labels and authors, and the first 100 open issues with real links.
These counts are observations, not fixtures or guaranteed current totals.

The fixture at .build/github-preview is an empty local repository pointing at
the public cli/cli remote, not a clone. No remote mutations were performed.
GitHub CLI must be installed and signed in; no separate token is stored by NiceGit.
Only github.com is supported. Missing-auth, cancellation, pagination, and
repository-switch UI paths still need broader end-to-end verification.
The older unconnected-state notes below describe the pre-integration build.

## September 12 Interface Check

The packaged app was relaunched and inspected through native accessibility and
screenshots using the preview repository. The standalone title/toolbar row is
absent; an HSplitView replaces the navigation sidebar wrapper to reduce reserved
top space. The repository selector visibly displays preview-repository (the old
multi-line native Menu label incorrectly displayed only REPOSITORY).

The graph, working-tree connection, local branches, worktrees, and section counts
render together. Branch context menus expose the selected-branch actions. The
tag dialog displays the selected branch tip, both tag types, and an expanding
annotation message field. The dialog was cancelled without creating a tag.
Hosted PR/issue data remains unimplemented, and those sections correctly display
an unconnected state. This is not verification of all native workflows or sizes.

The 35-test suite passed before these layout-only corrections; the subsequent
merge-revert assertion passed separately. The corrected release app builds and
passes the packaging script's signature check.

## Automated Checks

Worktree-creation integration coverage verifies a missing branch creates no destination, an occupied folder retains its contents, and duplicate branch checkout is refused.

A real-repository test verifies opening a folder whose name ends with a space and newline preserves the exact root path.

Per-checkout draft isolation is covered by an app-model test. All five app-model tests pass after moving drafts out of the transient workbench view.

The full 29-test suite passed after worktree creation was added. The optimized release bundle builds successfully and passes strict ad-hoc signature verification and Info.plist validation.

App-model tests also verify activation refresh waits for review dismissal, is consumed once, and is not queued during an active operation. All four app-model tests pass.

Environment isolation tests verify inherited repository/index/config overrides are removed while SSH agent and authentication settings remain available.

An app-model regression test performs a real commit with an injected snapshot failure, verifying one commit, one success callback, a cleared draft, and a visible error.

A companion test verifies a failed commit preserves its draft and never reports success. Both app-model tests passed after adding explicit completed-action refresh warnings.

`swift test` currently contains 31 tests. Integration coverage uses real temporary Git repositories for initialization, local identity, remote creation, cloning, tracking checkout, publishing, staging, unstaging, stashing, rename diffs, detached history, branch/tag management, binary conflict resolution, and merge/rebase/cherry-pick interruption handling. Process tests verify cancellation, timeouts, and termination of a child in the launched command's isolated process group.

Conflict tests cover reading index versions, rejecting unresolved markers, refusing stale editor writes, staging a resolution, and continuing a merge. Graph tests cover linear history, merges, converging tips, and independent roots. Parser tests cover exact filenames and remote references.

## Interactive Checks

Linked-worktree navigation was verified in both directions: `preview-linked` opens on clean `main`; returning to `preview-repository` restores `feature/conflicts` with its three untracked files. The sidebar filter resets on checkout navigation.

Merge-inspector integration coverage checks that the changed-file list and filtered/full patches agree against the first parent of a real two-parent merge.

Snapshot integration coverage verifies that saving a stash does not add Git's stash implementation commits to the branch history. Stashes remain available separately.

Verified in the locally built macOS app on 10 September 2026:

- Open a repository using the native folder picker and reopen it from Recents.
- Display branch tips and a merge graph from a real repository.
- Search loaded history and open a matching commit's details.
- Open an untracked file's diff with highlighted additions and unwrapped text.
- Verify old/new line-number gutters in the rebuilt diff viewer on 11 September 2026; parser tests cover hunk offsets and file boundaries.
- Stage that file, enter a message, and commit it through the UI.
- Observe the new commit count, clean working tree, and cleared message after success.
- Verify dark-mode contrast and top-left history/diff alignment after relaunch.
- Verify the redesigned working-tree node connects to HEAD when a different branch has a newer commit.
- Display nested changed-file folders and select a commit to populate the persistent file inspector.
- On 11 September, verify an external file addition appears on app activation without manual refresh; with a diff open, verify review remains visible and the deferred update runs on dismissal.
- On 11 September, save three untracked preview files through the stash dialog, inspect their numbered patches, and apply the stash. Git status confirms all three files returned and the stash remains available.
- Verify stash inspect/apply/delete controls appear separately in the accessibility tree, and open the patch using its accessible button.
- Verify Command-F focuses diff search, three matching lines cycle in both directions, the current line is visibly marked, and a missing term disables navigation.
- Verify search scrolls between lines 3 and 171 in a 180-line patch, with the matching line visible in both directions.
- Verify case-insensitive sidebar branch filtering, no-match state, and clear-button restoration without changing the current branch or graph.

The disposable repository is `.build/preview-repository`; it is excluded from the project repository. `bash scripts/create-preview-repository.sh` creates its initial branch graph without modifying an existing preview repository.

## Remaining Verification and Work

### GitHub Source Publication Preparation (2026-09-16)

- Added preview status, requirements, contributing/security guidance, third-party
  notices, changelog, issue/PR templates, and a maintainer release checklist.
- Added read-only GitHub CI for locked resolution, tests, release packaging, and
  plist validation on macOS 26/Xcode 26.6. YAML structure and local commands were
  validated; the hosted workflow has not run because no remote is configured.
- Full local test run passed: 50 tests in one suite, 44.490 seconds. The optional
  live test was skipped in this run; its separate successful run is recorded below.
- Release packaging, strict ad-hoc signature verification, and plist validation
  passed. Bundled NiceGit and SwiftTerm license contents match their originals.
- Expanded ignores exclude generated Xcode state, environment files, and common
  signing credentials. Reviewed the publishable file list and scanned common
  credential patterns/personal home paths with no matches. This is a basic
  publication hygiene check, not a comprehensive security audit.
- No source was staged, committed, pushed, or published. Public binary releases
  still need maintainer signing/notarization and clean-machine verification.

### Live GitHub Reads (2026-09-16)

- Native UI follow-up: opened the disposable github-preview fixture, selected
  origin under Pull Requests, and observed live open results. Switching State
  to Merged replaced the list with merged PRs. Filtering by `#14462` left exactly
  that PR visible, with the correct GitHub URL, author, and merged label; layout
  was checked in a screenshot. No links were opened and no remote data changed.

- Ran the opt-in `liveGitHubSidebarLoadsPublicRepository` test against public
  `cli/cli` using the existing GitHub CLI authentication. It passed with one
  result each for open PRs, open issues, merged PRs, and closed issues.
- Strengthened the test to require a non-empty result and the requested state,
  then reran successfully (2.693 seconds). This verifies the read integration,
  not hosted Git push/pull authentication, private repositories, or native UI.

### Current Regression Baseline (2026-09-16)

- Full `swift test --build-system native` passed with 50 tests in one suite in
  42.872 seconds. The optional live GitHub test remains skipped.
- New coverage verifies terminal toggling without a repository, while busy, and
  after external file changes, including session identity preservation.
- GitHub commit-link tests verify full SHA-1/SHA-256 hashes and reject short,
  malformed, and path-like inputs. Clipboard delivery through the branch menu
  still needs native UI verification; no remote availability claim is made.

### Worktree Controls (2026-09-16)

- Native UI verification found transparent areas of sidebar buttons did not
  consistently receive right-clicks. Added a rectangular content shape to their
  full row; rebuilt and verified the worktree menu opens from the row center.
- Verified Open worktree switches from preview-repository to preview-linked.
- Verified Control-backtick opens the bottom terminal for preview-linked and
  displays that checkout's full path. Reveal in Finder and Copy worktree path
  are present in the menu but have not been exercised interactively.

### App-Test Scheduling (2026-09-16)

- Grouped main-actor app-model tests in a serialized Swift Testing suite. Their
  synchronous Git fixture setup previously blocked other app tests' completion
  callbacks while those tests' wall-clock deadlines continued counting down.
- Kept all assertions and deadlines unchanged; core tests remain parallel.
- Two consecutive full `swift test --build-system native` runs succeeded,
  reporting 48 tests in one suite in 40.320 and 40.457 seconds. The optional live
  GitHub test remained skipped. These results supersede the unresolved parallel
  test failures recorded below; the global `--no-parallel` runner is not needed.

### macOS 27 and Embedded Terminal (2026-09-15)

- Follow-up: terminal isolation now covers the exit callback and replacement of
  an ended session while preserving another checkout's session. The focused
  test passed. Restart requests cannot replace an active session.
- Repeat release packaging succeeded after fixing replacement of the read-only
  bundled SwiftTerm license. The package's ad-hoc signature verified.
- The latest 48-test parallel runs exposed timing failures in existing app-model
  tests (load waits expired); these runs were not green. A sequential run stopped
  reporting results and was interrupted. The earlier 47-test result below is
  historical, not proof of a clean current full-suite run.

- Release packaging and ad-hoc signature verification succeeded with Xcode 27
  on macOS 27.0 (26A428), using the native SwiftPM build backend.
- Full `swift test --build-system native` run passed: 47 tests.
- Launched the packaged app and verified the bottom terminal panel visually.
  Entering `pwd` returned the selected preview repository's full path.
- Hiding and reopening the terminal restored the visible shell panel.
- Older macOS runtime compatibility, terminal foreground-process termination,
  and terminal sessions across checkout switches still need runtime coverage.
- These checks supersede the earlier external Terminal launch limitation:
  the action bar now embeds SwiftTerm inside NiceGit.

- Verify long commit-message layout in the inspector interactively; integration coverage checks multiline body retrieval.
- Verify typed commit drafts survive linked-checkout navigation and app restarts interactively. An isolated-preferences test now verifies restoration and per-checkout deletion across model recreation.
- Exercise the worktree destination picker interactively. Integration coverage creates a real linked checkout and verifies duplicate checkout of its branch is refused.
- Verify sidebar filtering with remote branches and tags interactively.
- Exercise return-to-app refresh suppression during a running operation.
- Exercise the conflict comparison/editor, clone, publish, and settings dialogs interactively.
- Exercise stash deletion confirmation interactively; integration coverage checks dropping stashes.
- Verify light appearance and narrower window layouts.
- Verify hosted HTTPS/SSH authentication; local remote tests do not establish this.
- Exercise whole-file binary conflict controls interactively; byte preservation and deletion are covered by integration tests.
- Exercise cancellation controls interactively, including a stalled hosted network operation. Commands have a ten-minute timeout; descendants that deliberately leave the launched process group are outside group cancellation.
- Distribution signing/notarization remains separate from local packaging. The build generates the complete icon set and verifies its ad-hoc signature; app metadata passed `plutil -lint`. Current bundles are for local use.

This is a working client under development, not a claim of full GitKraken feature parity.
