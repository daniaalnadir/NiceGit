//! Interface tests for automatic refresh, drag-to-merge, diff options, image comparison, the
//! file editor, paging through history, GitHub links, and patches.

use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use egui::accesskit::Role;
use egui::{Key, Modifiers};
use egui_kittest::kittest::{By, Queryable};
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

    harness.get_by_role_and_label(Role::CheckBox, "Hide whitespace").click();
    wait(&mut harness, "the full diff again", |h| !h.state().settings.ignore_whitespace && diff_has_changes(h) == Some(true));
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
    harness.get_by_label(&edit).click();
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
