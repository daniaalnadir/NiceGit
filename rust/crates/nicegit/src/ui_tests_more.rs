//! Interface tests for automatic refresh, drag-to-merge, diff options, image comparison, the
//! file editor, paging through history, GitHub links, and patches.

use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use egui::accesskit::Role;
use egui::{Key, Modifiers};
use egui_kittest::kittest::{By, NodeT, Queryable};
use egui_kittest::Harness;
use egui_phosphor::regular as icon;
use nicegit_core::diff::DiffLineKind;

use crate::app::NiceGitApp;
use crate::ui_tests::*;

type App = Harness<'static, NiceGitApp>;

/// Whether any widget's label contains `text`.
fn shows(harness: &App, text: &str) -> bool {
    harness.query_all_by_label_contains(text).next().is_some()
}

fn commit_file(dir: &Path, file: &str, contents: &[u8], message: &str) {
    std::fs::write(dir.join(file), contents).unwrap();
    git(dir, &["add", "--", file]);
    git(dir, &["commit", "-q", "-m", message]);
}

/// Whether the diff shown for the selected change has added or removed lines.
fn diff_has_changes(harness: &App) -> Option<bool> {
    let repo = harness.state().repo()?;
    if repo.diff_loading {
        return None;
    }
    let diff = repo.diff.as_ref()?;
    Some(diff.lines.iter().any(|line| matches!(line.kind, DiffLineKind::Addition | DiffLineKind::Deletion)))
}

#[test]
fn changes_made_in_another_app_appear_without_refreshing() {
    let repo = repository();
    let path = repo.path();
    let mut harness = open(path);
    loaded(&mut harness);

    // Changes within moments of a load are taken to be NiceGit's own, so wait past that first.
    std::thread::sleep(Duration::from_millis(600));
    let start = Instant::now();
    std::fs::write(path.join("outside.txt"), "made in another app\n").unwrap();
    wait(&mut harness, "the outside change", |h| {
        h.state().snapshot().is_some_and(|s| s.status.iter().any(|entry| entry.path == "outside.txt"))
    });
    // The periodic refresh is every 30 seconds; this one came from the file watcher.
    assert!(start.elapsed() < Duration::from_secs(10), "refreshed after {:?}", start.elapsed());
    assert!(shows(&harness, "outside.txt"), "the Changes panel lists the new file");
}

#[test]
fn drag_a_branch_onto_the_current_branch_to_merge_it() {
    let repo = repository();
    let path = repo.path();
    git(path, &["switch", "-q", "feature"]);
    commit_file(path, "side.txt", b"from feature\n", "Work on the feature");
    git(path, &["switch", "-q", "main"]);
    let mut harness = open(path);
    loaded(&mut harness);

    let from = harness.get_all_by_label("feature").next().expect("feature row").rect().center();
    let to = harness.get_all_by_label("main").next().expect("main row").rect().center();
    harness.hover_at(from);
    harness.step();
    harness.drag_at(from);
    harness.step();
    for step in 1..=6 {
        harness.hover_at(from + (to - from) * (step as f32 / 6.0));
        harness.step();
    }
    harness.drop_at(to);
    harness.step();
    wait(&mut harness, "the merge or rebase choice", |h| shows(h, "Integrate feature"));
    settle(&mut harness);

    harness.get_by_label(&format!("{}  Merge…", icon::GIT_MERGE)).click();
    wait(&mut harness, "the merge confirmation", |h| shows(h, "Merge feature into main?"));
    settle(&mut harness);
    harness.get_by_role_and_label(Role::Button, "Merge").click();
    idle(&mut harness);
    wait(&mut harness, "the merge", |_| git(path, &["rev-list", "--parents", "-n", "1", "HEAD"]).split_whitespace().count() == 3);
    assert_eq!(std::fs::read_to_string(path.join("side.txt")).unwrap(), "from feature\n");
}

#[test]
fn hide_whitespace_leaves_out_whitespace_only_changes() {
    let repo = repository();
    let path = repo.path();
    std::fs::write(path.join("notes.txt"), "first\n  second  \n").unwrap();
    let mut harness = open(path);
    loaded(&mut harness);

    harness.get_by_label("notes.txt").click();
    wait(&mut harness, "the diff", |h| diff_has_changes(h) == Some(true));
    settle(&mut harness);

    harness.get_by_role_and_label(Role::CheckBox, "Hide whitespace").click();
    wait(&mut harness, "the diff without whitespace", |h| h.state().settings.ignore_whitespace && diff_has_changes(h) == Some(false));
    // The reloaded diff lays the panel out again; click once it has settled.
    settle(&mut harness);

    harness.get_by_role_and_label(Role::CheckBox, "Hide whitespace").click();
    let start = Instant::now();
    while !(!harness.state().settings.ignore_whitespace && diff_has_changes(&harness) == Some(true)) {
        harness.step();
        std::thread::sleep(Duration::from_millis(15));
        let repo = harness.state().repo().unwrap();
        assert!(
            start.elapsed() < Duration::from_secs(20),
            "the full diff did not return: ignore_whitespace {}, diff loading {}, diff lines {:?}, selection {:?}",
            harness.state().settings.ignore_whitespace,
            repo.diff_loading,
            repo.diff.as_ref().map(|d| d.lines.iter().map(|l| l.text.clone()).collect::<Vec<_>>()),
            repo.selection,
        );
    }
}

#[test]
fn compare_shows_both_versions_of_a_changed_image() {
    let repo = repository();
    let path = repo.path();
    let image = |width, height| {
        let file = path.join("icon.png");
        image::RgbaImage::from_pixel(width, height, image::Rgba([40, 120, 200, 255])).save(&file).unwrap();
    };
    image(4, 3);
    git(path, &["add", "icon.png"]);
    git(path, &["commit", "-q", "-m", "Add the icon"]);
    image(8, 6);
    let mut harness = open(path);
    loaded(&mut harness);

    harness.get_by_label("Add the icon").click_secondary();
    settle(&mut harness);
    harness.get_by_label_contains("Compare with working files").click();
    wait(&mut harness, "the changed image", |h| h.query_by_role_and_label(Role::Button, "icon.png").is_some());
    settle(&mut harness);
    harness.get_by_role_and_label(Role::Button, "icon.png").click();
    wait(&mut harness, "both images", |h| shows(h, "4 × 3") && shows(h, "8 × 6"));
}

#[test]
fn edit_and_save_a_working_file_in_the_built_in_editor() {
    let repo = repository();
    let path = repo.path();
    std::fs::write(path.join("notes.txt"), "changed\n").unwrap();
    let mut harness = open(path);
    loaded(&mut harness);

    harness.get_by_label("notes.txt").click();
    let edit = format!("{}  Edit", icon::PENCIL_SIMPLE);
    wait(&mut harness, "the Edit button", |h| h.query_by_label(&edit).is_some());
    idle(&mut harness);
    // An accessibility click, like a screen reader's, does not depend on where the diff header
    // is laid out in the frame the click arrives.
    harness.get_by_label(&edit).click_accesskit();
    wait(&mut harness, "the editor", |h| h.query_by_role_and_label(Role::TextInput, "File contents").is_some());
    settle(&mut harness);

    harness.get_by_role_and_label(Role::TextInput, "File contents").focus();
    harness.step();
    harness.key_press_modifiers(Modifiers::COMMAND, Key::A);
    harness.step();
    harness.get_by_role_and_label(Role::TextInput, "File contents").type_text("rewritten");
    harness.step();
    let save = format!("{}  Save", icon::FLOPPY_DISK);
    wait(&mut harness, "unsaved edits", |h| h.state().tools.iter().any(|t| t.has_unsaved_changes()));
    harness.get_by_label(&save).click();
    wait(&mut harness, "the saved file", |_| std::fs::read_to_string(path.join("notes.txt")).unwrap() == "rewritten");
    wait(&mut harness, "the editor to be clean", |h| h.state().tools.iter().all(|t| !t.has_unsaved_changes()));
}

#[test]
fn load_older_history_reads_the_next_page() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path();
    git(path, &["init", "-q", "-b", "main"]);
    // One more page's worth of commits than a single load reads, made in one Git process.
    let total = crate::app::PAGE + 20;
    let mut stream = String::new();
    for index in 1..=total {
        let message = format!("Commit number {index}\n");
        stream.push_str(&format!(
            "commit refs/heads/main\ncommitter Test <test@example.invalid> {} +0000\ndata {}\n{message}\n",
            1_000_000_000 + index,
            message.len()
        ));
    }
    let mut import = Command::new("git").args(["fast-import", "--quiet"]).current_dir(path).stdin(Stdio::piped()).spawn().unwrap();
    import.stdin.take().unwrap().write_all(stream.as_bytes()).unwrap();
    assert!(import.wait().unwrap().success());
    let mut harness = open(path);
    loaded(&mut harness);
    let loaded_commits = |h: &App| h.state().snapshot().map(|s| (s.commits.len(), s.has_more_commits));
    assert_eq!(loaded_commits(&harness), Some((crate::app::PAGE, true)));

    // Scroll to the end of the graph, where the button is.
    let newest = harness.get_by_label(&format!("Commit number {total}")).rect().center();
    let button = format!("{}  Load older history", icon::CARET_DOUBLE_DOWN);
    for _ in 0..200 {
        if harness.query_by_label(&button).is_some() {
            break;
        }
        harness.hover_at(newest);
        harness.event(egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: egui::vec2(0.0, -4000.0),
            phase: egui::TouchPhase::Move,
            modifiers: Modifiers::NONE,
        });
        harness.step();
    }
    settle(&mut harness);
    harness.get_by_label(&button).click();
    idle(&mut harness);
    wait(&mut harness, "the older commits", |h| loaded_commits(h) == Some((total, false)));
    assert!(shows(&harness, "Commit number 1"), "the oldest commit is listed");
}

#[test]
fn copy_a_github_commit_link_from_the_graph() {
    let repo = repository();
    let path = repo.path();
    git(path, &["remote", "add", "origin", "git@github.com:octo/notes.git"]);
    let hash = git(path, &["rev-parse", "HEAD"]);
    let mut harness = open(path);
    loaded(&mut harness);

    harness.get_by_label("Add a second line").click_secondary();
    settle(&mut harness);
    harness.get_by_label_contains("Copy GitHub commit link").click();
    settle(&mut harness);
    // The submenu lists each GitHub remote; the sidebar's remote row has the same name.
    harness.query_all(By::new().role(Role::Button).label("origin")).last().expect("the origin menu item").click();
    let expected = format!("https://github.com/octo/notes/commit/{hash}");
    let mut copied = None;
    for _ in 0..5 {
        harness.step();
        for command in &harness.output().platform_output.commands {
            if let egui::OutputCommand::CopyText(text) = command {
                copied = Some(text.clone());
            }
        }
        if copied.is_some() {
            break;
        }
    }
    assert_eq!(copied.as_deref(), Some(expected.as_str()));
}

#[test]
fn save_a_commit_as_a_patch_and_apply_it_again() {
    let repo = repository();
    let path = repo.path();
    let folder = tempfile::tempdir().unwrap();
    let patch = folder.path().join("second-line.patch");
    let mut harness = open(path);
    loaded(&mut harness);

    harness.get_by_label("Add a second line").click_secondary();
    settle(&mut harness);
    crate::file_dialog::answer_next(&patch);
    harness.get_by_label_contains("Save as patch").click();
    wait(&mut harness, "the saved patch", |_| patch.exists());
    let saved = std::fs::read_to_string(&patch).unwrap();
    assert!(saved.contains("Subject: [PATCH] Add a second line") && saved.contains("+second"), "{saved}");

    // Take the commit away, then bring its change back from the patch.
    git(path, &["reset", "-q", "--hard", "HEAD~1"]);
    let head = git(path, &["rev-parse", "HEAD"]);
    harness.key_press_modifiers(Modifiers::COMMAND, Key::R);
    wait(&mut harness, "the refreshed checkout", |h| h.state().snapshot().and_then(|s| s.head_hash.clone()) == Some(head.clone()));
    idle(&mut harness);
    crate::file_dialog::answer_next(&patch);
    repository_menu(&mut harness, &folder_name(path), "Apply patch");
    wait(&mut harness, "the apply confirmation", |h| shows(h, "Apply second-line.patch?"));
    settle(&mut harness);
    harness.get_by_role_and_label(Role::Button, "Apply").click();
    idle(&mut harness);
    wait(&mut harness, "the applied change", |_| std::fs::read_to_string(path.join("notes.txt")).unwrap() == "first\nsecond\n");
    assert!(git(path, &["diff", "--cached", "--name-only"]).is_empty(), "the change stays unstaged");
    assert_eq!(git(path, &["diff", "--name-only"]), "notes.txt");
}

#[test]
fn abort_an_interrupted_merge_from_the_banner() {
    let repo = repository();
    let path = repo.path();
    git(path, &["switch", "-q", "feature"]);
    commit_file(path, "notes.txt", b"first\nfrom feature\n", "Change notes on feature");
    git(path, &["switch", "-q", "main"]);
    let head = git(path, &["rev-parse", "HEAD"]);
    // Another tool started the merge; Git stopped with a conflict.
    let merge = Command::new("git").args(["merge", "feature"]).current_dir(path).output().unwrap();
    assert!(!merge.status.success(), "the merge conflicts");
    let mut harness = open(path);
    loaded(&mut harness);

    wait(&mut harness, "the operation banner", |h| h.query_by_label("Abort…").is_some());
    harness.get_by_label("Abort…").click();
    wait(&mut harness, "the abort confirmation", |h| shows(h, "Abort the merge?"));
    settle(&mut harness);
    harness.get_by_role_and_label(Role::Button, "Abort").click();
    idle(&mut harness);
    wait(&mut harness, "the merge to end", |h| h.state().snapshot().is_some_and(|s| s.operation.is_none()));
    assert!(!path.join(".git/MERGE_HEAD").exists(), "Git has no merge in progress");
    assert_eq!(git(path, &["rev-parse", "HEAD"]), head);
    assert_eq!(std::fs::read_to_string(path.join("notes.txt")).unwrap(), "first\nsecond\n");
}

#[test]
fn amend_the_last_commit_with_staged_changes() {
    let repo = repository();
    let path = repo.path();
    let parent = git(path, &["rev-parse", "HEAD~1"]);
    std::fs::write(path.join("forgotten.txt"), "left out\n").unwrap();
    git(path, &["add", "forgotten.txt"]);
    let mut harness = open(path);
    loaded(&mut harness);

    // Choosing amend fills in the last commit's message.
    harness.get_by_role_and_label(Role::CheckBox, "Amend last commit").click();
    let amend = format!("{}  Amend last commit", icon::CHECK_CIRCLE);
    wait(&mut harness, "the amend button", |h| h.query_by_role_and_label(Role::Button, &amend).is_some());
    settle(&mut harness);
    harness.get_by_role_and_label(Role::Button, &amend).click();
    idle(&mut harness);
    wait(&mut harness, "the amended commit", |_| git(path, &["ls-tree", "--name-only", "HEAD"]).contains("forgotten.txt"));
    assert_eq!(git(path, &["log", "-1", "--format=%s"]), "Add a second line", "the message is kept");
    assert_eq!(git(path, &["rev-parse", "HEAD~1"]), parent, "the commit is replaced, not added to");
    assert!(git(path, &["status", "--porcelain"]).is_empty());
}

#[test]
fn returning_to_the_app_refreshes_even_with_automatic_refresh_off() {
    let repo = repository();
    let path = repo.path();
    let mut harness = open(path);
    loaded(&mut harness);
    harness.state_mut().settings.auto_refresh = false;
    let focus = |harness: &mut App, focused: bool| {
        harness.input_mut().viewports.entry(egui::ViewportId::ROOT).or_default().focused = Some(focused);
        harness.run_steps(3);
    };

    focus(&mut harness, false);
    std::thread::sleep(Duration::from_millis(600));
    std::fs::write(path.join("while-away.txt"), "made while NiceGit was in the background\n").unwrap();
    std::thread::sleep(Duration::from_millis(600));
    harness.run_steps(3);
    let listed = |h: &App| h.state().snapshot().is_some_and(|s| s.status.iter().any(|entry| entry.path == "while-away.txt"));
    assert!(!listed(&harness), "nothing refreshes while automatic refresh is off and the app is in the background");

    focus(&mut harness, true);
    wait(&mut harness, "the refresh on return", listed);
}

#[test]
fn settings_change_the_theme_and_diff_options() {
    let repo = repository();
    let mut harness = open(repo.path());
    loaded(&mut harness);

    harness.key_press_modifiers(Modifiers::COMMAND, Key::Comma);
    wait(&mut harness, "the Settings window", |h| h.query_by_role_and_label(Role::CheckBox, "Hide whitespace-only changes").is_some());
    settle(&mut harness);
    harness.get_by_role_and_label(Role::CheckBox, "Hide whitespace-only changes").click();
    harness.get_by_role_and_label(Role::CheckBox, "Show diffs side by side").click();
    harness.step();
    harness.get_by_label("Light").click();
    harness.run_steps(3);
    let settings = &harness.state().settings;
    assert!(settings.ignore_whitespace && settings.split_diff, "both diff options are on");
    assert_eq!(harness.ctx.theme(), egui::Theme::Light, "the light theme is in use");

    harness.get_by_label("Dark").click();
    harness.run_steps(3);
    assert_eq!(harness.ctx.theme(), egui::Theme::Dark);
}

#[test]
fn filter_references_narrows_the_sidebar() {
    let repo = repository();
    let path = repo.path();
    git(path, &["branch", "release/1.0"]);
    git(path, &["tag", "v1.0"]);
    let mut harness = open(path);
    loaded(&mut harness);
    assert!(harness.query_by_label("release/1.0").is_some() && harness.query_by_label("feature").is_some());

    type_into(&mut harness, "Filter references", "release");
    harness.run_steps(3);
    assert!(harness.query_by_label("release/1.0").is_some(), "the matching branch stays");
    assert!(harness.query_by_label("feature").is_none(), "other branches are hidden");
}

#[test]
fn folder_tree_groups_changed_files_by_folder() {
    let repo = repository();
    let path = repo.path();
    std::fs::create_dir_all(path.join("docs/guide")).unwrap();
    std::fs::write(path.join("docs/guide/start.md"), "start\n").unwrap();
    std::fs::write(path.join("docs/notes.md"), "notes\n").unwrap();
    let mut harness = open(path);
    loaded(&mut harness);

    // The path list names each file by its full path.
    assert!(shows(&harness, "docs/guide/start.md"));
    harness.get_by_label("Tree").click();
    harness.run_steps(3);
    assert_eq!(harness.state().settings.file_view, crate::settings::FileView::Tree);
    // The tree lists folders, with files by their own names beneath them.
    assert!(shows(&harness, "guide"), "the nested folder is listed");
    assert!(shows(&harness, "start.md") && !shows(&harness, "docs/guide/start.md"), "files show their own names");
}

#[test]
fn add_a_remote_in_repository_settings() {
    let repo = repository();
    let path = repo.path();
    let remote = tempfile::tempdir().unwrap();
    git(remote.path(), &["init", "-q", "--bare"]);
    let mut harness = open(path);
    loaded(&mut harness);

    repository_menu(&mut harness, &folder_name(path), "Repository settings");
    let remotes = format!("{}  Remotes", icon::CLOUD);
    wait(&mut harness, "the Remotes tab", |h| h.query_by_label(&remotes).is_some());
    harness.get_by_label(&remotes).click();
    wait(&mut harness, "the remote form", |h| h.query_by_role_and_label(Role::TextInput, "Remote name").is_some());
    settle(&mut harness);
    type_into(&mut harness, "Remote name", "upstream");
    type_into(&mut harness, "Remote URL", &remote.path().to_string_lossy());
    harness.get_by_label(&format!("{}  Add remote", icon::PLUS)).click();
    idle(&mut harness);
    wait(&mut harness, "the new remote", |_| git(path, &["remote"]).lines().any(|name| name == "upstream"));
    assert_eq!(git(path, &["remote", "get-url", "upstream"]), remote.path().to_string_lossy());
}

#[test]
fn find_in_diff_counts_and_steps_through_matches() {
    let repo = repository();
    let path = repo.path();
    std::fs::write(path.join("notes.txt"), "apple\nfirst\nbanana\napple pie\nsecond\n").unwrap();
    let mut harness = open(path);
    loaded(&mut harness);
    harness.get_by_label("notes.txt").click();
    wait(&mut harness, "the diff", |h| diff_has_changes(h) == Some(true));
    settle(&mut harness);

    type_into(&mut harness, "Find in diff", "apple");
    wait(&mut harness, "two matches, the first current", |h| shows(h, "1 of 2"));
    // Return in the field moves to the next match, as in the Mac app.
    let enter = |harness: &mut App| {
        harness.get_by_role_and_label(Role::TextInput, "Find in diff").focus();
        harness.step();
        harness.key_press(Key::Enter);
        harness.step();
    };
    enter(&mut harness);
    wait(&mut harness, "the second match", |h| shows(h, "2 of 2"));
    enter(&mut harness);
    wait(&mut harness, "stepping past the last match to wrap to the first", |h| shows(h, "1 of 2"));
    // The buttons step too. The pointer leaves first, so no tooltip is open over the button.
    harness.remove_cursor();
    settle(&mut harness);
    harness.get_by_role_and_label(Role::Button, "Previous match (Shift+Enter)").click();
    wait(&mut harness, "stepping back to wrap to the last", |h| shows(h, "2 of 2"));
}

#[test]
fn untracked_files_show_their_whole_content_as_added() {
    let repo = repository();
    let path = repo.path();
    std::fs::write(path.join("brand-new.txt"), "hello\nworld\n").unwrap();
    let mut harness = open(path);
    loaded(&mut harness);

    harness.get_by_label("brand-new.txt").click();
    wait(&mut harness, "the diff", |h| diff_has_changes(h) == Some(true));
    let added: Vec<String> = harness
        .state()
        .repo()
        .unwrap()
        .diff
        .as_ref()
        .unwrap()
        .lines
        .iter()
        .filter(|line| line.kind == DiffLineKind::Addition)
        .map(|line| line.text.clone())
        .collect();
    assert_eq!(added, ["+hello", "+world"]);
}

#[test]
fn a_message_draft_stays_with_the_checkout_across_branch_switches() {
    let repo = repository();
    let path = repo.path();
    let mut harness = open(path);
    loaded(&mut harness);

    type_into(&mut harness, "Commit summary", "Half-written message");
    harness.get_by_label("feature").click_secondary();
    settle(&mut harness);
    harness.get_by_label_contains("Check out").click();
    idle(&mut harness);
    wait(&mut harness, "the switch", |h| h.state().snapshot().is_some_and(|s| s.current_branch == "feature"));
    assert_eq!(harness.state().repo().unwrap().draft.summary, "Half-written message", "the draft belongs to the checkout");
}

#[test]
fn the_inspector_shows_the_whole_commit_message_as_written() {
    let repo = repository();
    let path = repo.path();
    std::fs::write(path.join("notes.txt"), "third\n").unwrap();
    let body = "Explain the change\n\nFirst line of the body,\nwrapped by hand at a short width.\n\n- a list item";
    git(path, &["commit", "-qam", body]);
    let mut harness = open(path);
    loaded(&mut harness);

    harness.get_by_label("Explain the change").click();
    let expected = "First line of the body,\nwrapped by hand at a short width.\n\n- a list item";
    wait(&mut harness, "the message body", |h| shows(h, expected));
}

#[test]
fn the_status_bar_shows_a_running_action_and_actions_wait_for_it() {
    let repo = repository();
    // With a remote, Fetch is available whenever nothing else is running.
    let remote = tempfile::tempdir().unwrap();
    git(remote.path(), &["init", "-q", "--bare"]);
    git(repo.path(), &["remote", "add", "origin", &remote.path().to_string_lossy()]);
    let mut harness = open(repo.path());
    loaded(&mut harness);
    let disabled = |harness: &App, label: &str| harness.get_by_role_and_label(Role::Button, label).accesskit_node().is_disabled();
    assert!(!disabled(&harness, "Fetch"), "Fetch starts out available");

    harness.state_mut().act("Waiting for the test", |_, _| {
        std::thread::sleep(Duration::from_millis(1500));
        Ok(None)
    });
    harness.run_steps(2);
    assert!(shows(&harness, "Waiting for the test…"), "the status bar names the running action");
    assert!(disabled(&harness, "Fetch"), "Fetch waits for the running action");
    idle(&mut harness);
    harness.run_steps(2);
    assert!(!shows(&harness, "Waiting for the test…"));
    assert!(!disabled(&harness, "Fetch"));
}

#[test]
fn copy_a_worktree_path_and_open_it_from_the_sidebar() {
    let repo = repository();
    let path = repo.path();
    let folder = tempfile::tempdir().unwrap();
    let linked = folder.path().join("linked");
    git(path, &["worktree", "add", "-q", "-b", "linked-work", &linked.to_string_lossy()]);
    let linked = std::fs::canonicalize(&linked).unwrap();
    let mut harness = open(path);
    loaded(&mut harness);

    // The worktree row comes after the branch of the same name.
    let row = |h: &App| h.get_all_by_label("linked-work").last().expect("the worktree row").rect().center();
    let at = row(&harness);
    harness.hover_at(at);
    harness.event(egui::Event::PointerButton {
        pos: at,
        button: egui::PointerButton::Secondary,
        pressed: true,
        modifiers: Modifiers::NONE,
    });
    harness.event(egui::Event::PointerButton {
        pos: at,
        button: egui::PointerButton::Secondary,
        pressed: false,
        modifiers: Modifiers::NONE,
    });
    settle(&mut harness);
    harness.get_by_label(&format!("{}  Copy path", icon::COPY)).click();
    let mut copied = None;
    for _ in 0..5 {
        harness.step();
        for command in &harness.output().platform_output.commands {
            if let egui::OutputCommand::CopyText(text) = command {
                copied = Some(text.clone());
            }
        }
    }
    let copied = copied.expect("a copied path");
    assert_eq!(std::fs::canonicalize(&copied).unwrap(), linked);

    // Clicking the row opens that checkout in its own tab.
    let at = row(&harness);
    harness.hover_at(at);
    harness.get_all_by_label("linked-work").last().unwrap().click();
    wait(&mut harness, "the linked checkout", |h| {
        h.state().repo().is_some_and(|r| std::fs::canonicalize(&r.path).ok().as_deref() == Some(linked.as_path()))
    });
    assert_eq!(harness.state().repos.len(), 2, "the first checkout stays open in its tab");
}

#[test]
fn undo_a_merge_from_the_toolbar() {
    let repo = repository();
    let path = repo.path();
    git(path, &["switch", "-q", "feature"]);
    commit_file(path, "side.txt", b"side\n", "Work on the feature");
    git(path, &["switch", "-q", "main"]);
    let before = git(path, &["rev-parse", "HEAD"]);
    let mut harness = open(path);
    loaded(&mut harness);

    harness.get_by_label("feature").click_secondary();
    settle(&mut harness);
    harness.get_by_label_contains("Merge into main").click();
    wait(&mut harness, "the merge confirmation", |h| shows(h, "Merge feature into main?"));
    settle(&mut harness);
    harness.get_by_role_and_label(Role::Button, "Merge").click();
    idle(&mut harness);
    wait(&mut harness, "the merge", |_| git(path, &["rev-list", "--parents", "-n", "1", "HEAD"]).split_whitespace().count() == 3);

    toolbar_button(&mut harness, "Undo");
    wait(&mut harness, "the undo confirmation", |h| h.state().dialog.is_some());
    settle(&mut harness);
    harness.get_all_by_label("Undo").last().expect("confirm button").click();
    idle(&mut harness);
    wait(&mut harness, "the branch back where it was", |_| git(path, &["rev-parse", "HEAD"]) == before);
    assert!(!path.join("side.txt").exists(), "the merged file is gone with the merge");
}

#[test]
fn stash_only_the_ticked_files_then_preview_the_stash() {
    let repo = repository();
    let path = repo.path();
    commit_file(path, "other.txt", b"other\n", "Add another file");
    std::fs::write(path.join("notes.txt"), "stash me\n").unwrap();
    std::fs::write(path.join("other.txt"), "keep me\n").unwrap();
    let mut harness = open(path);
    loaded(&mut harness);

    harness.get_by_label_contains("Manage stashes").click();
    wait(&mut harness, "the stash window", |h| h.query_by_role_and_label(Role::CheckBox, "other.txt").is_some());
    settle(&mut harness);
    harness.get_by_role_and_label(Role::CheckBox, "other.txt").click();
    harness.run_steps(2);
    harness.get_by_label_contains("Stash 1 file").click();
    idle(&mut harness);
    wait(&mut harness, "the stash", |_| !git(path, &["stash", "list"]).is_empty());
    assert_eq!(std::fs::read_to_string(path.join("notes.txt")).unwrap(), "first\nsecond\n", "the ticked file is stashed");
    assert_eq!(std::fs::read_to_string(path.join("other.txt")).unwrap(), "keep me\n", "the unticked file is untouched");

    // Selecting the stash in the sidebar previews its changes.
    harness.state_mut().tools.clear();
    let stash = git(path, &["stash", "list", "--format=%gs"]);
    wait(&mut harness, "the stash row", |h| h.query_by_label(&stash).is_some());
    harness.get_by_label(&stash).click();
    wait(&mut harness, "the stash preview", |h| {
        h.state().repo().and_then(|r| r.diff.as_ref()).is_some_and(|d| d.lines.iter().any(|line| line.text == "+stash me"))
    });
}

#[test]
fn interactive_rebase_drops_a_commit_through_its_window() {
    let repo = repository();
    let path = repo.path();
    commit_file(path, "keep.txt", b"keep\n", "Keep this commit");
    commit_file(path, "mistake.txt", b"oops\n", "Drop this commit");
    commit_file(path, "also.txt", b"also\n", "Keep this one too");
    let mut harness = open(path);
    loaded(&mut harness);

    harness.get_by_label("Add a second line").click_secondary();
    settle(&mut harness);
    harness.get_by_label_contains("Interactive rebase from here").click();
    let action = "Action for Drop this commit";
    wait(&mut harness, "the rebase plan", |h| h.query_by_role_and_label(Role::ComboBox, action).is_some());
    settle(&mut harness);
    harness.get_by_role_and_label(Role::ComboBox, action).click();
    settle(&mut harness);
    harness.get_all_by_label("Drop").last().expect("the Drop choice").click();
    settle(&mut harness);
    harness.get_by_label("Rewrite commits").click();
    if harness.query_by_role_and_label(Role::Button, "Rewrite").is_some() {
        harness.get_by_role_and_label(Role::Button, "Rewrite").click();
    }
    idle(&mut harness);
    wait(&mut harness, "the rewritten history", |_| !git(path, &["log", "--format=%s"]).contains("Drop this commit"));
    let log = git(path, &["log", "--format=%s"]);
    assert!(log.starts_with("Keep this one too\nKeep this commit\nAdd a second line"), "{log}");
    assert!(!path.join("mistake.txt").exists());
}

#[test]
fn an_idle_repository_does_not_keep_refreshing_itself() {
    let repo = repository();
    let path = repo.path();
    std::fs::write(path.join("notes.txt"), "changed\n").unwrap();
    let mut harness = open(path);
    loaded(&mut harness);
    // Let the opening load's own file activity pass.
    std::thread::sleep(Duration::from_millis(600));
    harness.run_steps(3);
    idle(&mut harness);
    let loads = harness.state().repo().unwrap().generation;

    // Showing a diff reads files, which Linux reports as file events. NiceGit's own reads must
    // not look like outside changes, or each one starts a refresh.
    harness.get_by_label("notes.txt").click();
    wait(&mut harness, "the diff", |h| diff_has_changes(h) == Some(true));
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(3) {
        harness.step();
        std::thread::sleep(Duration::from_millis(30));
    }
    assert_eq!(harness.state().repo().unwrap().generation, loads, "no refresh happened while nothing changed");
}
