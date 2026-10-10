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
    let start = Instant::now();
    while harness.query_by_role_and_label(Role::TextInput, "File contents").is_none() {
        harness.step();
        std::thread::sleep(Duration::from_millis(15));
        let state = harness.state();
        assert!(
            start.elapsed() < Duration::from_secs(20),
            "the editor did not open: windows {:?}, busy {:?}, selection {:?}, Edit button shown {}, notice {:?}",
            state.tools.iter().map(|t| t.title()).collect::<Vec<_>>(),
            state.busy,
            state.repo().map(|r| r.selection.clone()),
            harness.query_by_label(&edit).is_some(),
            state.notice.as_ref().map(|n| n.text.clone()),
        );
    }
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
fn the_main_window_stays_usable_while_the_editor_is_open() {
    // As in the Mac app, where the file editor sits in the workspace rather than a sheet, the
    // editor does not block or dim the window behind it.
    let repo = repository();
    let path = repo.path();
    std::fs::write(path.join("notes.txt"), "changed\n").unwrap();
    let mut harness = open(path);
    loaded(&mut harness);
    harness.get_by_label("notes.txt").click();
    let edit = format!("{}  Edit", icon::PENCIL_SIMPLE);
    wait(&mut harness, "the Edit button", |h| h.query_by_label(&edit).is_some());
    idle(&mut harness);
    harness.get_by_label(&edit).click_accesskit();
    wait(&mut harness, "the editor", |h| h.query_by_role_and_label(Role::TextInput, "File contents").is_some());

    harness.get_all_by_label("Start the notes").next().expect("the first commit's row").click_accesskit();
    wait(&mut harness, "the commit to be selected", |h| {
        h.state().repo().is_some_and(|r| matches!(&r.selection, crate::app::Selection::Commit { .. }))
    });
    assert!(harness.query_by_role_and_label(Role::TextInput, "File contents").is_some(), "the editor stays open");
}

#[test]
fn load_older_history_reads_the_next_page() {
    let dir = temp_dir();
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
    let folder = temp_dir();
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
    let remote = temp_dir();
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
    let remote = temp_dir();
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
fn copy_reveal_and_open_a_worktree_from_the_sidebar() {
    let repo = repository();
    let path = repo.path();
    let folder = temp_dir();
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

    // Show in file manager reveals the worktree folder.
    harness.hover_at(row(&harness));
    let at = row(&harness);
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
    harness.get_by_label(&format!("{}  Show in file manager", icon::FOLDER)).click();
    harness.run_steps(2);
    let revealed = crate::ui::changes::REVEALED.with(|revealed| revealed.borrow().clone());
    assert_eq!(revealed.iter().map(|path| std::fs::canonicalize(path).unwrap()).collect::<Vec<_>>(), std::slice::from_ref(&linked));

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

#[test]
fn filter_loaded_commits_by_message() {
    let repo = repository();
    let mut harness = open(repo.path());
    loaded(&mut harness);
    assert!(harness.query_by_label("Start the notes").is_some());

    type_into(&mut harness, "Filter loaded commits", "second");
    harness.run_steps(3);
    assert!(harness.query_by_label("Add a second line").is_some(), "the matching commit stays");
    assert!(harness.query_by_label("Start the notes").is_none(), "other commits are hidden");
}

#[test]
fn command_f_moves_to_find_in_diff() {
    let repo = repository();
    let path = repo.path();
    std::fs::write(path.join("notes.txt"), "first\nchanged\n").unwrap();
    let mut harness = open(path);
    loaded(&mut harness);
    harness.get_by_label("notes.txt").click();
    wait(&mut harness, "the diff", |h| diff_has_changes(h) == Some(true));

    harness.key_press_modifiers(Modifiers::COMMAND, Key::F);
    wait(&mut harness, "the find field to take focus", |h| {
        h.get_by_role_and_label(Role::TextInput, "Find in diff").accesskit_node().is_focused()
    });

    // The focused field takes typed text; the one deleted line "second" is the only match.
    harness.event(egui::Event::Text("second".into()));
    wait(&mut harness, "the match count for the typed text", |h| h.query_by_label("1 of 1").is_some());
}

#[test]
fn publish_a_branch_through_its_dialog() {
    let repo = repository();
    let path = repo.path();
    let remote = temp_dir();
    git(remote.path(), &["init", "-q", "--bare"]);
    git(path, &["remote", "add", "origin", &remote.path().to_string_lossy()]);
    let mut harness = open(path);
    loaded(&mut harness);

    repository_menu(&mut harness, &folder_name(path), "Publish branch");
    wait(&mut harness, "the publish dialog", |h| shows(h, "Push main to a remote and track it there."));
    // The toolbar has a Publish button too; the dialog's is drawn last.
    harness.query_all(By::new().role(Role::Button).label("Publish")).last().expect("the dialog's Publish").click();
    idle(&mut harness);
    wait(&mut harness, "the upstream", |_| {
        Command::new("git")
            .args(["rev-parse", "--abbrev-ref", "main@{upstream}"])
            .current_dir(path)
            .output()
            .is_ok_and(|o| o.status.success())
    });
    assert_eq!(git(path, &["rev-parse", "--abbrev-ref", "main@{upstream}"]), "origin/main");
    assert_eq!(git(remote.path(), &["rev-parse", "main"]), git(path, &["rev-parse", "main"]), "the remote has the branch");
}

#[test]
fn drag_a_branch_onto_the_current_branch_to_rebase_onto_it() {
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
    harness.get_by_label(&format!("{}  Rebase…", icon::GIT_PULL_REQUEST)).click();
    wait(&mut harness, "the rebase confirmation", |h| shows(h, "Rebase main onto feature?"));
    harness.get_by_role_and_label(Role::Button, "Rebase").click();
    idle(&mut harness);
    wait(&mut harness, "the rebase", |_| {
        Command::new("git").args(["merge-base", "--is-ancestor", "feature", "main"]).current_dir(path).status().is_ok_and(|s| s.success())
    });
    assert_eq!(git(path, &["rev-list", "--parents", "-n", "1", "HEAD"]).split_whitespace().count(), 2, "history stays linear");
    assert_eq!(git(path, &["log", "-1", "--format=%s"]), "Add a second line", "main's own commit is replayed on top");
}

/// A bare repository holding `path`'s main branch, which `path` tracks as origin.
fn publish_to_bare(path: &Path) -> tempfile::TempDir {
    let remote = temp_dir();
    // Clones of it check out main, whatever this machine's default branch name is.
    git(remote.path(), &["init", "-q", "--bare", "-b", "main"]);
    git(path, &["remote", "add", "origin", &remote.path().to_string_lossy()]);
    git(path, &["push", "-q", "-u", "origin", "main"]);
    remote
}

#[test]
fn undo_a_pull_from_the_toolbar() {
    let repo = repository();
    let path = repo.path();
    let remote = publish_to_bare(path);
    // Someone else pushes a commit.
    let other = temp_dir();
    let clone = other.path().join("clone");
    git(other.path(), &["clone", "-q", &remote.path().to_string_lossy(), "clone"]);
    git(&clone, &["config", "user.name", "Other"]);
    git(&clone, &["config", "user.email", "other@example.invalid"]);
    commit_file(&clone, "theirs.txt", b"from the other clone\n", "Work from elsewhere");
    git(&clone, &["push", "-q", "origin", "main"]);
    let theirs = git(&clone, &["rev-parse", "HEAD"]);
    let before = git(path, &["rev-parse", "HEAD"]);
    let mut harness = open(path);
    loaded(&mut harness);

    toolbar_button(&mut harness, "Pull");
    idle(&mut harness);
    wait(&mut harness, "the pull", |_| git(path, &["rev-parse", "HEAD"]) == theirs);

    toolbar_button(&mut harness, "Undo");
    wait(&mut harness, "the undo confirmation", |h| h.state().dialog.is_some());
    harness.get_all_by_label("Undo").last().expect("the confirm button").click();
    idle(&mut harness);
    wait(&mut harness, "the branch back before the pull", |_| git(path, &["rev-parse", "HEAD"]) == before);
    assert!(!path.join("theirs.txt").exists(), "the pulled file goes with the pull");
}

#[test]
fn publish_a_branch_from_the_command_palette() {
    let repo = repository();
    let path = repo.path();
    let remote = temp_dir();
    git(remote.path(), &["init", "-q", "--bare"]);
    git(path, &["remote", "add", "origin", &remote.path().to_string_lossy()]);
    let mut harness = open(path);
    loaded(&mut harness);

    harness.key_press_modifiers(Modifiers::COMMAND | Modifiers::SHIFT, Key::P);
    wait(&mut harness, "the palette", |h| h.state().palette.is_some());
    harness.event(egui::Event::Text("publish".into()));
    wait(&mut harness, "the publish command", |h| h.query_by_label_contains("Publish branch").is_some());
    harness.key_press(Key::Enter);
    wait(&mut harness, "the publish dialog", |h| shows(h, "Push main to a remote and track it there."));
    harness.query_all(By::new().role(Role::Button).label("Publish")).last().expect("the dialog's Publish").click();
    idle(&mut harness);
    wait(&mut harness, "the published branch", |_| {
        Command::new("git").args(["rev-parse", "--verify", "-q", "main"]).current_dir(remote.path()).status().is_ok_and(|s| s.success())
    });
    assert_eq!(git(path, &["rev-parse", "--abbrev-ref", "main@{upstream}"]), "origin/main");
}

#[test]
fn search_files_in_a_commit_then_open_blame_at_a_match() {
    let repo = repository();
    let path = repo.path();
    // The working file no longer has the text; only the commit does.
    std::fs::write(path.join("notes.txt"), "rewritten\n").unwrap();
    let mut harness = open(path);
    loaded(&mut harness);

    harness.get_by_label("Start the notes").click_secondary();
    settle(&mut harness);
    harness.get_by_label_contains("Search files in this commit").click();
    wait(&mut harness, "the search window", |h| h.query_by_role_and_label(Role::TextInput, "Text in files").is_some());
    type_into(&mut harness, "Text in files", "first");
    harness.key_press(Key::Enter);
    let blame = format!("{}  Blame", icon::USER);
    wait(&mut harness, "a match", |h| h.query_by_label(&blame).is_some());
    assert!(shows(&harness, "notes.txt"), "the match is in the commit's notes.txt");

    harness.get_all_by_label(&blame).next().unwrap().click_accesskit();
    wait(&mut harness, "blame at the match", |h| {
        h.state().tools.iter().any(|t| t.title().contains("notes.txt") && t.id().contains("blame"))
    });
}

#[test]
fn check_out_a_submodule_at_its_recorded_commit() {
    let library = repository();
    let repo = repository();
    let path = repo.path();
    let add = Command::new("git")
        .args(["-c", "protocol.file.allow=always", "submodule", "add", "-q"])
        .arg(library.path())
        .arg("library")
        .current_dir(path)
        .output()
        .unwrap();
    assert!(add.status.success(), "{}", String::from_utf8_lossy(&add.stderr));
    git(path, &["commit", "-qm", "Add the library"]);
    let recorded = git(path, &["rev-parse", "HEAD:library"]);
    // The submodule's checkout moves away from the commit the superproject records.
    let checkout = path.join("library");
    git(&checkout, &["config", "user.name", "Test"]);
    git(&checkout, &["config", "user.email", "test@example.invalid"]);
    git(&checkout, &["checkout", "-q", "HEAD~1"]);
    assert_ne!(git(&checkout, &["rev-parse", "HEAD"]), recorded);
    let mut harness = open(path);
    loaded(&mut harness);

    repository_menu(&mut harness, &folder_name(path), "Submodules");
    let button = format!("{}  Check out recorded commit", icon::GIT_COMMIT);
    wait(&mut harness, "the submodule", |h| {
        h.query_by_role_and_label(Role::Button, &button).is_some_and(|b| !b.accesskit_node().is_disabled())
    });
    harness.get_by_role_and_label(Role::Button, &button).click_accesskit();
    idle(&mut harness);
    wait(&mut harness, "the recorded commit", |_| git(&checkout, &["rev-parse", "HEAD"]) == recorded);
}

#[test]
fn the_inspector_shows_a_verified_signature() {
    let repo = repository();
    let path = repo.path();
    let keys = temp_dir();
    let key = keys.path().join("signing");
    let Ok(made) = Command::new("ssh-keygen").args(["-q", "-t", "ed25519", "-N", "", "-C", "test", "-f"]).arg(&key).output() else {
        eprintln!("ssh-keygen is not installed; skipping");
        return;
    };
    assert!(made.status.success());
    let public = std::fs::read_to_string(key.with_extension("pub")).unwrap();
    let allowed = keys.path().join("allowed_signers");
    std::fs::write(&allowed, format!("test@example.invalid {public}")).unwrap();
    git(path, &["config", "gpg.format", "ssh"]);
    git(path, &["config", "user.signingkey", &key.to_string_lossy().replace('\\', "/")]);
    git(path, &["config", "gpg.ssh.allowedSignersFile", &allowed.to_string_lossy().replace('\\', "/")]);
    std::fs::write(path.join("signed.txt"), "signed\n").unwrap();
    git(path, &["add", "signed.txt"]);
    let empty = keys.path().join("empty-config");
    std::fs::write(&empty, "").unwrap();
    let signed = Command::new("git")
        .args(["commit", "-q", "-S", "-m", "A signed commit"])
        .current_dir(path)
        .env("GIT_CONFIG_GLOBAL", &empty)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .unwrap();
    assert!(signed.status.success(), "{}", String::from_utf8_lossy(&signed.stderr));
    // NiceGit checks signatures with the user's own Git settings, as Git does.
    if !Command::new("git").args(["log", "-1", "--format=%G?"]).current_dir(path).output().unwrap().status.success() {
        eprintln!("Git cannot check signatures with this machine's settings; skipping");
        return;
    }
    let mut harness = open(path);
    loaded(&mut harness);

    harness.get_by_label("A signed commit").click();
    wait(&mut harness, "the signature line", |h| shows(h, "Verified signature"));
}

#[test]
fn drag_a_graph_label_onto_the_current_row_to_rebase_onto_it() {
    let repo = repository();
    let path = repo.path();
    git(path, &["switch", "-q", "feature"]);
    commit_file(path, "side.txt", b"from feature\n", "Work on the feature");
    git(path, &["switch", "-q", "main"]);
    let mut harness = open(path);
    loaded(&mut harness);

    let from = harness.get_by_label("Branch label feature").rect().center();
    let to = harness.get_by_label("Add a second line").rect().center();
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
    harness.get_by_label(&format!("{}  Rebase…", icon::GIT_PULL_REQUEST)).click();
    wait(&mut harness, "the rebase confirmation", |h| shows(h, "Rebase main onto feature?"));
    harness.get_by_role_and_label(Role::Button, "Rebase").click();
    idle(&mut harness);
    wait(&mut harness, "the rebase", |_| {
        Command::new("git").args(["merge-base", "--is-ancestor", "feature", "main"]).current_dir(path).status().is_ok_and(|s| s.success())
    });
    assert_eq!(git(path, &["rev-list", "--parents", "-n", "1", "HEAD"]).split_whitespace().count(), 2, "history stays linear");
}
