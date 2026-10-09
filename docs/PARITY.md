# Cross-platform Feature Parity

Each feature in the Mac app's [feature guide](FEATURES.md), where the cross-platform app in
[`rust/`](../rust) provides it, and what checks it. Test names refer to:

- **UI tests**: `rust/crates/nicegit/src/ui_tests.rs`, `ui_tests_tools.rs`, and `ui_tests_more.rs`. They drive the real
  app window through egui_kittest, clicking and typing by accessible label against a temporary
  repository, then check the result with Git.
- **Core tests**: `rust/crates/nicegit-core/tests/<file>.rs`. They run the Git operation behind a
  feature against a temporary repository, including its stale-state and safety refusals.
- **Unit tests**: `#[cfg(test)]` modules next to the code.
- **CI run**: the `Run the app` job in `.github/workflows/rust-ci.yml` launches the built app on
  macOS, Windows, and Linux, screenshots it, and on Windows and Linux sends real keyboard input
  through the operating system (Shift-Ctrl-P, "stage all", Return) and checks that Git staged the
  file.

Two independent audits by separate agents compared the guide with the code and tests without
reading this file; the gaps they found are fixed and tested below.

`every_repository_tool_opens_from_the_menu` also opens and draws every window in the Repository
menu, and `smallest_window_lays_out_every_panel` lays out every panel at the minimum window size.

## Repositories and window

| Mac app feature | Cross-platform app | Verified by |
| --- | --- | --- |
| Open any local repository | Repository menu, empty-window button, Ctrl/Cmd-O | every UI test opens one |
| Initialize repositories; repository identity | Repository menu › New repository; Repository Settings › Identity | core `coverage::initialize_creates_repository_that_snapshot_can_open`, `settings::set_identity_*` |
| Add, rename, remove remotes; edit fetch URL | Repository Settings › Remotes | UI `add_a_remote_in_repository_settings`; core `settings::rename_remote_*`, `set_remote_address_*`, `remove_remote_*` |
| Clone from a URL or local path | Repository menu › Clone, empty window | UI `clone_a_repository_through_its_dialog` |
| Background Git with a busy indicator | Status bar spinner; actions refuse while busy | UI `the_status_bar_shows_a_running_action_and_actions_wait_for_it` |
| Repository tabs; restore on launch | Repositories column | UI `recent_repositories_reopen_after_their_tab_closes` |
| Recent repositories | Repositories column › Recent, empty window, palette | UI `recent_repositories_reopen_after_their_tab_closes` |
| Resizable, remembered columns; restore default width | Panel dividers; double-click a divider; Settings button | UI `double_clicking_a_panel_edge_restores_its_width`, `smallest_window_lays_out_every_panel` |
| Settings: appearance, graph colours, diff defaults, auto refresh | Settings (Ctrl/Cmd-,) | UI `settings_change_the_theme_and_diff_options` |
| Automatic refresh after outside changes; refresh on return to the app | File watcher, Settings toggle; returning always refreshes | UI `changes_made_in_another_app_appear_without_refreshing`, `returning_to_the_app_refreshes_even_with_automatic_refresh_off`, `an_idle_repository_does_not_keep_refreshing_itself`; unit `app::tests::*` (2) |
| Command palette with fuzzy matching | Shift-Ctrl/Cmd-P | UI `command_palette_runs_a_command`; unit `ui::palette::tests::word_starts_and_runs_rank_higher`; CI run (OS keyboard input) |
| Embedded terminal per checkout; hide keeps shells; stop with confirmation | Terminal panel (Ctrl-`) | UI `terminal_panel_opens_and_hides`; unit `tools::terminal::tests::*` (3) |
| Per-checkout commit-message drafts | Commit box, kept per checkout folder | UI `a_message_draft_stays_with_the_checkout_across_branch_switches`; unit `settings::tests::drafts_round_trip_summary_and_description` |

## Branches, remotes, tags, stashes

| Mac app feature | Cross-platform app | Verified by |
| --- | --- | --- |
| Local and remote branches; filter references | Sidebar sections and filter | UI `check_out_a_branch_from_its_menu`, `filter_references_narrows_the_sidebar`; unit `parsers::tests::branches_*`, `remote_name_prefers_longest_match` |
| Check out local and remote branches; tracking branches | Branch menu, double-click, palette | UI `check_out_a_branch_from_its_menu`; core `client::checkout_*`, `coverage::checkout_remote_*` (6) |
| Create, rename, delete branches; create at a selected tip | Toolbar Branch, branch and commit menus | UI `create_a_branch_from_the_toolbar`; core `client::create_branch_*`, `delete_*`, `coverage::rename_branch_*`, `create_branch_from_*` |
| Upstream selector and removal | Branch menu › Upstream | core `settings::set_and_unset_upstream_checks_branch_tip`, `set_upstream_refuses_missing_remote_branch` |
| Fetch, pull, push, publish; push a selected branch | Toolbar; branch menu | UI `publish_a_branch_through_its_dialog`; core `client::publish_sets_upstream_and_push_updates_remote`, `pull_*`, `push_refuses_stale_snapshot`, `coverage::fetch_*`, `push_branch_*` |
| Ahead and behind counts | Sidebar, toolbar Pull/Push labels | core `client::snapshot_counts_commits_ahead_of_and_behind_the_upstream` |
| Drag a branch onto the current branch to merge or rebase | Sidebar rows and graph labels | UI `drag_a_branch_onto_the_current_branch_to_merge_it`, `drag_a_branch_onto_the_current_branch_to_rebase_onto_it` |
| Merge and rebase with previews | Branch menu; confirmation shows the preview | core `rewrite::merge_preview_*` (4), `rebase_preview_is_marked_as_an_estimate`, `rebase_onto_branch_replays_the_current_branch`, `client::merge_refuses_*` |
| Worktrees: browse, open, reveal, copy path, create, remove, forget | Sidebar Worktrees, Worktrees window | UI `create_and_remove_a_worktree_from_the_interface`, `copy_reveal_and_open_a_worktree_from_the_sidebar`; core `settings::create_worktree_*`, `remove_worktree_*`, `prune_*` |
| Submodules with state; check out recorded commit | Sidebar Submodules, Submodules window | core `settings::submodule*` (3), `missing_submodule_folder_*`, `update_submodule_*` |
| Tags: create (lightweight or annotated), inspect, delete, push, delete from remote | Sidebar Tags, commit menu | UI `create_lightweight_and_annotated_tags_from_the_graph`; core `client::delete_tag_*`, `create_tag_*`, `settings::push_tag_*`, `delete_remote_tag_*` |
| Stashes: save selected files and untracked, preview, apply, pop, delete | Sidebar Stashes, Stashes window, toolbar | UI `stash_window_saves_selected_changes`, `stash_only_the_ticked_files_then_preview_the_stash`; core `client::*stash*` (4), `client::pop_that_conflicts_keeps_the_stash`, `settings::stash_selected_*` (3), `coverage::stash_diff_*`; unit `tools::stash::tests::*` (8) |
| GitHub pull requests and issues | Sidebar sections, GitHub window | UI `github_pull_requests_and_issues_load_from_github` (needs `gh` sign-in, so run with `--ignored`; passes against github.com); core `inspect::github_*` (3); unit `github::tests::*` (3) |
| Copy a GitHub commit link | Branch and commit menus | UI `copy_a_github_commit_link_from_the_graph`; core `inspect::github_remote_addresses_are_parsed_in_every_supported_form` |
| Clean up merged or inactive branches, undoable | Repository menu › Clean up branches; toolbar Undo | UI `clean_up_branches_deletes_a_merged_branch_and_undo_restores_it`; core `workflows::cleanup_*` (4) |
| GitFlow | Repository menu › GitFlow | UI `gitflow_sets_up_develop_and_starts_a_feature`; core `workflows::gitflow_*` (9, including a hotfix) |
| Git LFS | Repository menu › Git LFS | UI `track_and_untrack_an_lfs_pattern`; core `workflows::lfs_*`, `track_*`, `untrack_*` |
| Identity profiles | Repository Settings › Profiles, palette | UI `save_an_identity_profile_and_apply_it_from_the_palette`; core `settings::apply_identity_*` |

## History

| Mac app feature | Cross-platform app | Verified by |
| --- | --- | --- |
| Commit graph with stable lanes, labels, initials, working tree at HEAD | History | unit `graph::tests::*` (4), `graph_view::tests::initials_use_first_and_last_words`, `parsers::tests::log_keeps_commas_in_ref_names` |
| Keyboard navigation (Up, Down, Escape) | History | UI `arrow_keys_move_through_the_graph_and_escape_clears` |
| Load older history; filter loaded commits | History header and end of the graph | UI `load_older_history_reads_the_next_page`, `filter_loaded_commits_by_message` |
| Search every branch's history | Search history window (Shift-Ctrl/Cmd-F) | UI `search_history_finds_a_commit_by_its_message`; core `history::search_*` (5) |
| Search file contents; open blame at a match | Search file contents (Alt-Ctrl/Cmd-F) | core `history::content_search_*` (3); unit `search::tests::*` (3) |
| Commit inspector: message, metadata, parents, files, patches | Right panel | UI `the_inspector_shows_the_whole_commit_message_as_written`, `restore_a_file_from_the_commit_inspector`, `revert_and_edit_message_from_the_graph`; core `history::commit_file_diff_*` |
| Signature status | Inspector | core `inspect::ssh_signed_commit_is_verified_only_with_its_key_allowed`, `unsigned_commit_has_no_signature`, `commit_with_an_unchecked_signature_is_not_reported_as_verified`; unit `signature::tests::each_git_code_maps_to_the_status_the_mac_app_shows` |
| Restore a file to a commit's version or before it | Inspector file menu, with confirmation | UI `restore_a_file_from_the_commit_inspector`; core `history::restore_*` (7) |
| File history following renames | Changes and inspector menus | UI `file_history_lists_the_commits_that_changed_the_file`; core `history::file_history_*` (3) |
| Blame with age tint and ignore-whitespace | Changes and inspector menus | UI `blame_labels_each_line_with_its_commit_and_author`; core `history::blame_*` (6) |
| Compare commits, or a commit with working files; images | Commit menu › Compare / Mark for comparison | UI `compare_with_working_files_lists_the_changed_file`, `compare_shows_both_versions_of_a_changed_image`; core `inspect::compare_two_commits_and_commit_against_working_files`, `file_bytes_at_*`; unit `tools::compare::tests::*` (TIFF, ICO, ICNS) |
| Bisect from a good commit; bar while bisecting | Commit menu, Bisect window, bisect bar | UI `bisect_finds_the_first_bad_commit_through_the_bar`; core `inspect::bisect_*` (3) |
| Recover lost work from the reflog | Repository menu › Recover lost work | UI `recover_lost_work_creates_a_branch_at_a_reset_away_commit`; core `workflows::reflog_*`, `creating_a_branch_at_a_reflog_entry_*` |
| Export a commit as a patch; apply a patch | Commit menu, Repository menu (confirmed) | UI `save_a_commit_as_a_patch_and_apply_it_again`; core `coverage::exported_commit_applies_*`, `export_refuses_merge_commit`, `apply_patch_*` (2) |

## Changes and history rewriting

| Mac app feature | Cross-platform app | Verified by |
| --- | --- | --- |
| Staged and unstaged changes as a path list or folder tree | Changes panel | UI `stage_commit_and_undo_through_the_interface`, `folder_tree_groups_changed_files_by_folder`, `untracked_files_show_their_whole_content_as_added`; core `client::snapshot_*` (5) |
| Diffs: unified or side by side (long lines wrapped), hide whitespace in every diff, find with Ctrl/Cmd-F | Diff panel, Compare, blame, file history, stash preview | UI `hide_whitespace_leaves_out_whitespace_only_changes`, `find_in_diff_counts_and_steps_through_matches`, `command_f_moves_to_find_in_diff`; unit `diff_view::tests::split_rows_are_as_tall_as_the_wrapped_text` and 3 more, `diff::tests::*` (3), `inline::tests::*` (6) |
| Stage, unstage, and commit | Changes panel, commit box | UI `stage_commit_and_undo_through_the_interface`; core `client::stage_*`, `unstage_*`, `commit_*`; CI run (OS keyboard input stages through the palette) |
| Stage individual lines | Diff panel line selection | core `staging::*` (13 line-staging tests); unit `staging::tests::*` (4) |
| Discard with confirmation; undo recent discards | Changes panel | UI `discard_asks_first_and_can_be_undone`, `discard_all_restores_every_changed_file`; core `client::discard_*` (5), `rewrite::discard_undo_*` (4) |
| Ignore untracked files (.gitignore or this computer only) | Changes menu › Ignore | core `settings::ignore_*` (8) |
| Edit working files | Diff panel › Edit, changes menu; long lines wrap | UI `edit_and_save_a_working_file_in_the_built_in_editor`; unit `tools::editor::tests::*` (2) |
| Amend, with a warning when published | Commit box | UI `amend_the_last_commit_with_staged_changes`; core `rewrite::is_published_reports_commits_contained_by_a_remote_branch` |
| Edit the HEAD message | Commit menu › Edit message | UI `revert_and_edit_message_from_the_graph`; core `coverage::amend_message_*` (2) |
| Undo and redo commits, amends, resets, merges, deletions | Toolbar, with confirmation | UI `stage_commit_and_undo_through_the_interface`, `undo_a_merge_from_the_toolbar`, `clean_up_branches_deletes_a_merged_branch_and_undo_restores_it`; core `rewrite::undo_*`, `redo_*` (8) |
| Cherry-pick one or many (oldest first); revert; choose a merge's parent | Commit menu, Ctrl/Cmd-click selection | UI `command_click_cherry_picks_the_marked_commits_oldest_first`, `revert_and_edit_message_from_the_graph`; core `rewrite::cherry_pick_*`, `revert_*` |
| Reset soft, mixed, or hard | Commit and branch menus › Reset | core `rewrite::undo_hard_reset_*`, `undo_soft_reset_*`, `undo_mixed_reset_*` |
| Interactive rebase: drag to reorder, pick, reword, squash, fixup, drop | Commit menu › Interactive rebase | UI `interactive_rebase_drops_a_commit_through_its_window`; core `rewrite::plan_*` (3), `interactive_rebase_*` (9); unit `tools::interactive_rebase::tests::*` (6) |
| Continue or abort interrupted operations | Operation banner | UI `resolve_a_merge_conflict_in_the_editor` (Continue), `abort_an_interrupted_merge_from_the_banner`; core `client::snapshot_reports_merge_operation_during_conflict` |
| Conflict editor with base, current, incoming; whole-file resolutions | Conflict window | UI `resolve_a_merge_conflict_in_the_editor`; core `staging::load_*` and the conflict tests (10); unit `conflict::tests::*` |

## Details of each feature

A last independent audit listed the parts of features that no test yet exercised, such as
bisect's Skip, the System appearance, blame's interactions, continuing or aborting a stopped
rebase, cherry-pick, or revert, Undo after each kind of change, and drafts surviving a
relaunch. The interface tests in `ui_tests_parity_a.rs`, `ui_tests_parity_b.rs`, and
`ui_tests_parity_c.rs` cover each of them: `a_commit_draft_and_the_side_by_side_choice_survive_a_relaunch`, `amending_a_commit_already_on_a_remote_warns_before_amending`, `a_branch_or_tag_made_in_another_app_appears_without_refreshing`, `undo_a_cherry_pick_from_the_toolbar`, `undo_a_revert_from_the_toolbar`, `undo_a_rebase_from_the_toolbar`, `drag_a_graph_label_onto_the_current_row_to_merge_it`, `returning_to_the_app_waits_for_an_open_dialog_to_close`, `sidebar_filter_narrows_tags_and_remote_branches`, `filter_reveals_remote_branches_under_a_collapsed_remote`, `settings_choose_a_graph_palette_and_follow_the_system_theme`, `double_clicking_the_repositories_edge_restores_its_width`, `compare_shows_the_file_size_of_each_version_of_an_image`, `compare_shows_large_image_sizes_in_kilobytes`, `palette_switches_to_a_branch_by_its_name`, `palette_reopens_a_recent_repository_after_its_tab_closes`, `palette_undoes_the_last_commit`, `clicking_a_tag_selects_the_commit_it_points_to`, `bisect_skip_marks_a_commit_untestable_and_git_records_it`, `interactive_rebase_warns_about_commits_already_on_a_remote`, `mark_a_commit_then_compare_it_with_another`, `blame_ignore_whitespace_gives_the_line_back_to_its_earlier_commit`, `clicking_a_blame_line_selects_its_commit_and_shows_its_change`, `file_history_restores_an_earlier_version_after_confirmation`, `rebase_the_current_branch_onto_another_from_the_branch_menu`, `cherry_pick_one_commit_from_its_menu`, `abort_a_rebase_stopped_by_a_conflict_from_the_banner`, `continue_a_rebase_after_resolving_its_conflict_from_the_banner`, `abort_a_cherry_pick_stopped_by_a_conflict_from_the_banner`, `continue_a_cherry_pick_after_resolving_its_conflict_from_the_banner`, `abort_a_revert_stopped_by_a_conflict_from_the_banner`, `continue_a_revert_after_resolving_its_conflict_from_the_banner`.

Further interface tests cover undoing a pull, publishing from the command palette, searching
files in a commit and opening blame at a match, checking out a submodule's recorded commit,
the inspector's verified-signature line, and rebasing by dragging a graph label; unit tests
cover blame's age tint.

A further independent audit then found 70 of 71 features fully implemented, the exception
being the commit signature line's wording, which now matches the Mac app. Its list of
remaining untested details is covered by `ui_tests_parity_d.rs`, `ui_tests_parity_e.rs`, and
`ui_tests_parity_f.rs`: `create_a_repository_from_the_empty_window`, `clone_from_a_file_url_through_the_dialog`, `rename_a_branch_from_its_menu`, `delete_a_merged_branch_from_its_menu`, `check_out_a_remote_branch_creates_a_tracking_branch`, `rename_a_remote_and_change_its_fetch_address_in_settings`, `push_a_tag_to_a_remote_then_delete_it_there`, `double_click_a_branch_in_the_sidebar_to_check_it_out`, `ignore_an_untracked_file_in_the_shared_gitignore`, `ignore_every_file_with_an_extension_in_the_shared_gitignore`, `ignore_an_untracked_file_only_on_this_computer`, `delete_a_local_tag_from_the_sidebar_after_confirming`, `push_the_current_branch_with_the_toolbar_button`, `pop_a_stash_from_the_sidebar_menu_after_confirming`, `delete_one_stash_from_the_stashes_window_after_confirming`, `conflict_editor_shows_the_base_current_and_incoming_versions`, `delete_file_resolves_a_conflict_by_deleting_the_file`, `file_history_opens_from_the_changes_menu`, `open_a_submodule_as_a_repository_tab_from_its_window`, `the_history_filter_matches_author_hash_and_reference_names`, `panel_widths_and_the_whitespace_choice_survive_a_relaunch`, `undo_an_amend_restores_the_old_commit_and_keeps_the_change_staged`, `apply_an_identity_profile_from_repository_settings`, `search_history_finds_a_commit_older_than_the_loaded_page`, `compare_shows_the_detail_of_the_selected_file`, `restore_this_version_brings_back_the_files_content_at_that_commit`, `the_smallest_window_keeps_the_key_controls_on_screen`.

The final independent audit found all 71 features implemented. The five details it still
listed as untested now have tests: `command_o_opens_a_repository_chosen_in_the_folder_dialog`,
`option_command_f_opens_file_content_search`, `shift_command_f_opens_history_search`,
`returning_to_the_app_waits_for_a_running_action`, `fetch_and_refresh_from_the_toolbar`, and the
unit test `lines_and_changed_words_are_coloured_by_kind`. An unverifiable signature now uses the
Mac app's secondary colour.

They found two bugs, now fixed: image sizes from 1 KB to 1 MB were labelled as bytes, and a
filter did not reveal matching branches under a collapsed remote. Undo and Redo from the
command palette now ask first, as the toolbar's buttons do.

## Platform differences

- The Mac app is a native SwiftUI app. The cross-platform app uses egui and looks the same on
  every system, with the Inter typeface and Phosphor icons. Menus live in the Repository picker,
  the toolbar's More menu, and the command palette rather than a macOS menu bar.
- HEIC images preview on macOS, through the system's own `sips` tool. No pure-Rust HEIC
  decoder exists, so on Windows and Linux the Compare window shows their size only. PNG, JPEG,
  GIF, BMP, WebP, TIFF, ICO, and ICNS preview everywhere.
- Interaction tests run headless through egui_kittest on all three systems in CI. Keyboard input
  through the operating system is tested on Windows and Linux in CI; on macOS it was checked by
  hand.
