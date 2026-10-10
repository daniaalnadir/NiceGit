//! Interface tests: they drive the real app through egui_kittest, finding widgets by their
//! accessible labels and clicking and typing as a person would, against temporary repositories.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use egui::{Key, Modifiers};
use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;

use crate::app::NiceGitApp;

pub(crate) fn git(dir: &Path, args: &[&str]) -> String {
    let output =
        Command::new("git").args(args).current_dir(dir).env_remove("GIT_DIR").env_remove("GIT_WORK_TREE").output().expect("run git");
    assert!(output.status.success(), "git {args:?} failed: {}", String::from_utf8_lossy(&output.stderr));
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

/// A temporary folder for a test. On Windows it is left for the runner to discard: deleting
/// it as the test ends can block indefinitely while the app's file watcher and finished Git
/// processes are still releasing their handles, which once held a CI job until it timed out.
pub(crate) fn temp_dir() -> tempfile::TempDir {
    #[allow(unused_mut)]
    let mut dir = tempfile::tempdir().expect("temporary folder");
    #[cfg(windows)]
    dir.disable_cleanup(true);
    dir
}

/// A repository with two commits on main, a `feature` branch, and local-only configuration.
pub(crate) fn repository() -> tempfile::TempDir {
    let dir = temp_dir();
    let path = dir.path();
    git(path, &["init", "-q", "-b", "main"]);
    for (key, value) in
        [("user.name", "Test"), ("user.email", "test@example.invalid"), ("commit.gpgsign", "false"), ("core.autocrlf", "false")]
    {
        git(path, &["config", key, value]);
    }
    std::fs::write(path.join("notes.txt"), "first\n").unwrap();
    git(path, &["add", "notes.txt"]);
    git(path, &["commit", "-q", "-m", "Start the notes"]);
    git(path, &["branch", "feature"]);
    std::fs::write(path.join("notes.txt"), "first\nsecond\n").unwrap();
    git(path, &["commit", "-qam", "Add a second line"]);
    dir
}

/// Stops the test run if one test thread runs for minutes, naming it, so a frame stuck in a
/// blocked call fails quickly instead of holding CI until its job times out.
struct Watchdog(std::sync::Arc<std::sync::atomic::AtomicBool>, std::sync::Arc<std::sync::Mutex<String>>);

impl Drop for Watchdog {
    fn drop(&mut self) {
        self.0.store(true, std::sync::atomic::Ordering::Relaxed);
    }
}

thread_local! {
    static WATCHDOG: std::cell::RefCell<Option<Watchdog>> = const { std::cell::RefCell::new(None) };
}

fn arm_watchdog() {
    WATCHDOG.with(|watchdog| {
        if watchdog.borrow().is_some() {
            return;
        }
        let done = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let finished = done.clone();
        let progress = std::sync::Arc::new(std::sync::Mutex::new(String::from("opening the app")));
        let last = progress.clone();
        let name = std::thread::current().name().unwrap_or("an interface test").to_string();
        std::thread::spawn(move || {
            let start = Instant::now();
            while !finished.load(std::sync::atomic::Ordering::Relaxed) {
                if start.elapsed() > Duration::from_secs(180) {
                    let last = last.lock().map(|text| text.clone()).unwrap_or_default();
                    // Written to stderr directly: the test harness captures eprintln output.
                    use std::io::Write;
                    let _ = writeln!(std::io::stderr(), "{name} has been stuck for three minutes, last {last}; stopping the test run");
                    print_all_thread_stacks();
                    std::process::exit(101);
                }
                std::thread::sleep(Duration::from_secs(1));
            }
        });
        *watchdog.borrow_mut() = Some(Watchdog(done, progress));
    });
}

/// On Windows, prints every thread's stack with the debugger the CI image ships, so a hang
/// shows which call blocked.
fn print_all_thread_stacks() {
    #[cfg(windows)]
    {
        let debugger = r"C:\Program Files (x86)\Windows Kits\10\Debuggers\x64\cdb.exe";
        if Path::new(debugger).exists() {
            let _ = Command::new(debugger).args(["-p", &std::process::id().to_string(), "-c", ".symfix; .reload; ~*kc 60; qd"]).status();
        }
    }
}

/// Records what the test is doing, for the watchdog to report if it stalls.
fn note_progress(text: impl FnOnce() -> String) {
    WATCHDOG.with(|watchdog| {
        if let Some(Watchdog(_, progress)) = watchdog.borrow().as_ref() {
            if let Ok(mut slot) = progress.lock() {
                *slot = text();
            }
        }
    });
}

pub(crate) fn open(path: &Path) -> Harness<'static, NiceGitApp> {
    arm_watchdog();
    let path: PathBuf = path.to_path_buf();
    Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_eframe(move |cc| NiceGitApp::new(cc, Some(path)))
}

/// Runs frames until `done` holds, failing after a generous timeout.
pub(crate) fn wait(harness: &mut Harness<'static, NiceGitApp>, what: &str, done: impl Fn(&Harness<'static, NiceGitApp>) -> bool) {
    let start = Instant::now();
    let mut frames = 0;
    loop {
        frames += 1;
        note_progress(|| {
            let busy = harness.state().busy.clone();
            format!("waiting for {what}: frame {frames}, {:.1}s in, busy {busy:?}", start.elapsed().as_secs_f32())
        });
        harness.step();
        if done(harness) {
            // A window or menu that just appeared spends its first frames measuring itself and
            // ignores clicks, so let it settle before the test acts on it.
            harness.run_steps(3);
            note_progress(|| format!("finished waiting for {what}"));
            return;
        }
        assert!(start.elapsed() < Duration::from_secs(30), "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(15));
    }
}

pub(crate) fn loaded(harness: &mut Harness<'static, NiceGitApp>) {
    wait(harness, "the repository to load", |h| h.state().snapshot().is_some() && h.state().busy.is_none());
}

pub(crate) fn idle(harness: &mut Harness<'static, NiceGitApp>) {
    wait(harness, "the action to finish", |h| h.state().busy.is_none() && h.state().repo().is_some_and(|r| !r.loading));
}

/// New menus and popups spend their first frame measuring themselves, disabled.
pub(crate) fn settle(harness: &mut Harness<'static, NiceGitApp>) {
    note_progress(|| "settling frames".into());
    harness.run_steps(3);
}

/// Clicks a toolbar button, through the More menu when it did not fit on the toolbar.
pub(crate) fn toolbar_button(harness: &mut Harness<'static, NiceGitApp>, label: &str) {
    settle(harness);
    let role = egui::accesskit::Role::Button;
    if harness.query_by_role_and_label(role, label).is_some() {
        harness.get_by_role_and_label(role, label).click();
    } else {
        harness.get_by_label("More").click();
        settle(harness);
        harness.get_by_label_contains(&format!("  {label}")).click();
    }
    harness.step();
}

pub(crate) fn has_label(harness: &Harness<'static, NiceGitApp>, label: &str) -> bool {
    harness.query_by_label_contains(label).is_some()
}

#[test]
fn stage_commit_and_undo_through_the_interface() {
    let repo = repository();
    std::fs::write(repo.path().join("notes.txt"), "first\nsecond\nthird\n").unwrap();
    let mut harness = open(repo.path());
    loaded(&mut harness);

    harness.get_by_label("Stage All Changes").click();
    idle(&mut harness);
    assert_eq!(git(repo.path(), &["diff", "--cached", "--name-only"]), "notes.txt");

    let summary = harness.get_by_label("Commit summary");
    summary.focus();
    harness.step();
    harness.get_by_label("Commit summary").type_text("Add a third line");
    wait(&mut harness, "the commit button", |h| has_label(h, "Commit 1 file to main"));
    harness.get_by_label_contains("Commit 1 file to main").click();
    idle(&mut harness);
    assert_eq!(git(repo.path(), &["log", "-1", "--format=%s"]), "Add a third line");

    // Undo asks first, then moves the branch back and keeps the change staged.
    toolbar_button(&mut harness, "Undo");
    wait(&mut harness, "the undo confirmation", |h| h.state().dialog.is_some());
    harness.get_all_by_label("Undo").last().expect("confirm button").click();
    idle(&mut harness);
    assert_eq!(git(repo.path(), &["log", "-1", "--format=%s"]), "Add a second line");
    assert_eq!(git(repo.path(), &["diff", "--cached", "--name-only"]), "notes.txt");

    // Redo puts the commit back.
    toolbar_button(&mut harness, "Redo");
    wait(&mut harness, "the redo confirmation", |h| h.state().dialog.is_some());
    harness.get_all_by_label("Redo").last().expect("confirm button").click();
    idle(&mut harness);
    assert_eq!(git(repo.path(), &["log", "-1", "--format=%s"]), "Add a third line");
}

#[test]
fn discard_asks_first_and_can_be_undone() {
    let repo = repository();
    std::fs::write(repo.path().join("notes.txt"), "changed\n").unwrap();
    let mut harness = open(repo.path());
    loaded(&mut harness);

    harness.get_by_label("notes.txt").click_secondary();
    settle(&mut harness);
    harness.get_by_label(&format!("{}  Discard changes…", egui_phosphor::regular::TRASH)).click();
    wait(&mut harness, "the discard confirmation", |h| h.state().dialog.is_some());
    harness.get_by_label("Discard").click();
    idle(&mut harness);
    assert_eq!(std::fs::read_to_string(repo.path().join("notes.txt")).unwrap(), "first\nsecond\n");

    harness.get_by_label_contains("Undo discard").click();
    idle(&mut harness);
    assert_eq!(std::fs::read_to_string(repo.path().join("notes.txt")).unwrap(), "changed\n");
}

#[test]
fn check_out_a_branch_from_its_menu() {
    let repo = repository();
    let mut harness = open(repo.path());
    loaded(&mut harness);

    harness.get_by_label("feature").click_secondary();
    settle(&mut harness);
    harness.get_by_label_contains("Check out").click();
    idle(&mut harness);
    assert_eq!(git(repo.path(), &["branch", "--show-current"]), "feature");
    assert_eq!(harness.state().snapshot().map(|s| s.current_branch.clone()).as_deref(), Some("feature"));
}

#[test]
fn command_palette_runs_a_command() {
    let repo = repository();
    std::fs::write(repo.path().join("extra.txt"), "new\n").unwrap();
    let mut harness = open(repo.path());
    loaded(&mut harness);

    harness.key_press_modifiers(Modifiers::COMMAND | Modifiers::SHIFT, Key::P);
    wait(&mut harness, "the palette", |h| h.state().palette.is_some());
    harness.event(egui::Event::Text("stage all".into()));
    harness.step();
    harness.key_press(Key::Enter);
    idle(&mut harness);
    assert_eq!(git(repo.path(), &["diff", "--cached", "--name-only"]), "extra.txt");
}

#[test]
fn every_repository_tool_opens_from_the_menu() {
    let repo = repository();
    let mut harness = open(repo.path());
    loaded(&mut harness);
    let name = repo.path().file_name().unwrap().to_string_lossy().into_owned();
    let items = [
        ("Repository settings", "Repository Settings"),
        ("Stashes", "Stashes"),
        ("Search history", "Search commits"),
        ("Search file contents", "Find in working files"),
        ("Recover lost work", "Recover Lost Work"),
        ("Clean up branches", "Clean Up Branches"),
        ("Bisect", "Bisect"),
        ("Worktrees", "Worktrees"),
        ("Submodules", "Submodules"),
        ("GitFlow", "GitFlow"),
        ("Git LFS", "Git LFS"),
    ];
    for (item, title) in items {
        let before = harness.state().tools.len();
        let picker = format!("{name} {}", egui_phosphor::regular::CARET_DOWN);
        harness.get_all_by_label(&picker).next().expect("repository picker").click();
        settle(&mut harness);
        harness.get_by_label_contains(item).click();
        wait(&mut harness, item, |h| h.state().tools.len() > before || h.state().tools.iter().any(|t| t.title() == title));
        assert!(
            harness.state().tools.iter().any(|t| t.title() == title),
            "{item} opened {:?}",
            harness.state().tools.iter().map(|t| t.title()).collect::<Vec<_>>()
        );
        // Let the window load and draw a few frames without panicking, then close it.
        for _ in 0..5 {
            harness.step();
            std::thread::sleep(Duration::from_millis(20));
        }
        harness.state_mut().tools.clear();
        settle(&mut harness);
    }
}

#[test]
fn stash_window_saves_selected_changes() {
    let repo = repository();
    std::fs::write(repo.path().join("notes.txt"), "changed\n").unwrap();
    let mut harness = open(repo.path());
    loaded(&mut harness);

    harness.get_by_label_contains("Manage stashes").click();
    wait(&mut harness, "the stash window", |h| h.state().tools.iter().any(|t| t.id() == "stashes"));
    harness.get_by_label_contains("Stash changes").click();
    idle(&mut harness);
    assert_eq!(git(repo.path(), &["stash", "list"]).lines().count(), 1, "one stash was made");
    assert_eq!(git(repo.path(), &["stash", "show", "--name-only", "stash@{0}"]), "notes.txt", "it holds the changed file");
    assert_eq!(git(repo.path(), &["show", "stash@{0}:notes.txt"]), "changed", "it holds the changed contents");
    assert_eq!(std::fs::read_to_string(repo.path().join("notes.txt")).unwrap(), "first\nsecond\n");
}

#[cfg(unix)]
#[test]
fn terminal_panel_opens_and_hides() {
    let repo = repository();
    let mut harness = open(repo.path());
    loaded(&mut harness);

    toolbar_button(&mut harness, "Terminal");
    assert!(harness.state().terminal.is_some());
    harness.key_press_modifiers(Modifiers::CTRL, Key::Backtick);
    harness.step();
    assert!(harness.state().terminal.is_none());
}

#[test]
fn revert_and_edit_message_from_the_graph() {
    let repo = repository();
    let mut harness = open(repo.path());
    loaded(&mut harness);

    harness.get_by_label("Add a second line").click_secondary();
    settle(&mut harness);
    harness.get_by_label_contains("Revert").click();
    wait(&mut harness, "the revert confirmation", |h| h.state().dialog.is_some());
    harness.get_by_label("Revert").click();
    idle(&mut harness);
    assert!(git(repo.path(), &["log", "-1", "--format=%s"]).starts_with("Revert"));
    assert_eq!(std::fs::read_to_string(repo.path().join("notes.txt")).unwrap(), "first\n");

    // The new HEAD's message can be edited from its menu.
    let subject = git(repo.path(), &["log", "-1", "--format=%s"]);
    harness.get_by_label(&subject).click_secondary();
    settle(&mut harness);
    harness.get_by_label_contains("Edit message").click();
    wait(&mut harness, "the message editor", |h| matches!(h.state().dialog, Some(crate::ui::dialogs::Dialog::EditMessage { .. })));
    if let Some(crate::ui::dialogs::Dialog::EditMessage { message, .. }) = harness.state_mut().dialog.as_mut() {
        *message = "Take the second line back out".into();
    }
    harness.step();
    harness.get_by_label("Save message").click();
    idle(&mut harness);
    assert_eq!(git(repo.path(), &["log", "-1", "--format=%s"]), "Take the second line back out");
}

#[test]
fn create_a_branch_from_the_toolbar() {
    let repo = repository();
    let mut harness = open(repo.path());
    loaded(&mut harness);
    settle(&mut harness);

    toolbar_button(&mut harness, "Branch");
    wait(&mut harness, "the new branch dialog", |h| h.state().dialog.is_some());
    harness.event(egui::Event::Text("topic/ui-test".into()));
    harness.step();
    harness.get_by_label("OK").click();
    idle(&mut harness);
    assert_eq!(git(repo.path(), &["branch", "--show-current"]), "topic/ui-test");
}

#[test]
fn resolve_a_merge_conflict_in_the_editor() {
    let repo = repository();
    let path = repo.path();
    git(path, &["switch", "-q", "feature"]);
    std::fs::write(path.join("notes.txt"), "first\nfrom feature\n").unwrap();
    git(path, &["commit", "-qam", "Change notes on feature"]);
    git(path, &["switch", "-q", "main"]);
    let mut harness = open(path);
    loaded(&mut harness);

    // Merge from the branch menu; Git stops with a conflict.
    harness.get_by_label("feature").click_secondary();
    settle(&mut harness);
    harness.get_by_label_contains("Merge into main").click();
    wait(&mut harness, "the merge confirmation", |h| h.state().dialog.is_some());
    harness.get_by_label("Merge").click();
    idle(&mut harness);
    assert_eq!(harness.state().snapshot().and_then(|s| s.operation), Some(nicegit_core::Operation::Merge));

    // Open the conflict editor from the banner and keep the current version.
    harness.get_by_label_contains("  notes.txt").click();
    wait(&mut harness, "the conflict editor", |h| h.state().tools.iter().any(|t| t.id().starts_with("conflict:")));
    wait(&mut harness, "the conflict to load", |h| h.query_by_label_contains("Take current").is_some());
    // Wait until the window has finished sizing itself, so the button stays where it was found.
    let mut last = harness.get_by_label_contains("Take current").rect();
    for _ in 0..30 {
        harness.step();
        let now = harness.get_by_label_contains("Take current").rect();
        if now == last {
            break;
        }
        last = now;
    }
    let take = harness.get_by_label_contains("Take current").rect();
    assert!(take.max.y <= 900.0, "the conflict window's buttons fit on screen: {take:?}");
    harness.get_by_label_contains("Take current").click();
    settle(&mut harness);

    harness.get_by_label("Take version").click();
    idle(&mut harness);
    assert_eq!(std::fs::read_to_string(path.join("notes.txt")).unwrap(), "first\nsecond\n");
    assert!(git(path, &["diff", "--name-only", "--diff-filter=U"]).is_empty());

    // Continue the merge from the banner.
    harness.state_mut().tools.clear();
    settle(&mut harness);
    harness.get_by_label("Continue").click();
    idle(&mut harness);
    assert!(harness.state().snapshot().is_some_and(|s| s.operation.is_none()));
    assert_eq!(git(path, &["rev-list", "--parents", "-n", "1", "HEAD"]).split_whitespace().count(), 3, "a merge commit");
}

#[test]
fn restore_a_file_from_the_commit_inspector() {
    let repo = repository();
    let mut harness = open(repo.path());
    loaded(&mut harness);

    harness.get_by_label("Add a second line").click();
    wait(&mut harness, "the commit's files", |h| h.state().repo().is_some_and(|r| !r.commit_files.is_empty()));
    settle(&mut harness);
    harness.get_by_label("notes.txt").click_secondary();
    settle(&mut harness);
    harness.get_by_label_contains("Restore version before this commit").click();
    wait(&mut harness, "the restore confirmation", |h| h.state().dialog.is_some());
    harness.get_by_label("Restore").click();
    idle(&mut harness);
    assert_eq!(std::fs::read_to_string(repo.path().join("notes.txt")).unwrap(), "first\n");
    assert_eq!(git(repo.path(), &["diff", "--cached", "--name-only"]), "notes.txt");
}

#[test]
fn arrow_keys_move_through_the_graph_and_escape_clears() {
    let repo = repository();
    let mut harness = open(repo.path());
    loaded(&mut harness);
    let head = git(repo.path(), &["rev-parse", "HEAD"]);
    let parent = git(repo.path(), &["rev-parse", "HEAD~1"]);
    let selected = |h: &Harness<'static, NiceGitApp>| match h.state().repo().map(|r| r.selection.clone()) {
        Some(crate::app::Selection::Commit { hash, .. }) => Some(hash),
        _ => None,
    };

    harness.key_press(Key::ArrowDown);
    harness.step();
    assert_eq!(selected(&harness), Some(head));
    harness.key_press(Key::ArrowDown);
    harness.step();
    assert_eq!(selected(&harness), Some(parent));
    harness.key_press(Key::Escape);
    harness.step();
    assert_eq!(selected(&harness), None);
}

/// The smallest window the app allows still lays out every panel; debug builds check widths.
#[test]
fn smallest_window_lays_out_every_panel() {
    let repo = repository();
    std::fs::write(repo.path().join("notes.txt"), "changed\n").unwrap();
    let path = repo.path().to_path_buf();
    let mut harness = Harness::builder().with_size(egui::vec2(900.0, 560.0)).build_eframe(move |cc| NiceGitApp::new(cc, Some(path)));
    loaded(&mut harness);
    harness.state_mut().select_working_tree();
    settle(&mut harness);
    let head = git(repo.path(), &["rev-parse", "HEAD"]);
    harness.state_mut().select_commit(head);
    settle(&mut harness);
    harness.key_press_modifiers(Modifiers::COMMAND | Modifiers::SHIFT, Key::P);
    settle(&mut harness);
    harness.key_press(Key::Escape);
    harness.state_mut().show_settings = true;
    settle(&mut harness);
    let ctx = harness.ctx.clone();
    harness.state_mut().toggle_terminal(&ctx);
    settle(&mut harness);
}

/// Types into the text field with this accessible label.
pub(crate) fn type_into(harness: &mut Harness<'static, NiceGitApp>, label: &str, text: &str) {
    note_progress(|| format!("typing into {label}"));
    let role = egui::accesskit::Role::TextInput;
    harness.get_by_role_and_label(role, label).focus();
    harness.step();
    harness.get_by_role_and_label(role, label).type_text(text);
    harness.step();
}

/// Opens a Repository menu item from the toolbar's repository picker.
pub(crate) fn repository_menu(harness: &mut Harness<'static, NiceGitApp>, name: &str, item: &str) {
    let picker = format!("{name} {}", egui_phosphor::regular::CARET_DOWN);
    harness.get_all_by_label(&picker).next().expect("repository picker").click();
    settle(harness);
    harness.get_by_label_contains(item).click();
    settle(harness);
}

pub(crate) fn folder_name(path: &Path) -> String {
    path.file_name().unwrap().to_string_lossy().into_owned()
}

#[test]
fn clone_a_repository_through_its_dialog() {
    let source = repository();
    let target_parent = temp_dir();
    let destination = target_parent.path().join("cloned");
    let mut harness = open(source.path());
    loaded(&mut harness);

    repository_menu(&mut harness, &folder_name(source.path()), "Clone repository");
    wait(&mut harness, "the clone dialog", |h| h.state().dialog.is_some());
    type_into(&mut harness, "Repository URL or local path", &source.path().display().to_string());
    type_into(&mut harness, "Destination folder", &destination.display().to_string());
    harness.get_by_label("OK").click();
    wait(&mut harness, "the clone to open", |h| {
        h.state().repo().is_some_and(|r| r.snapshot.as_ref().is_some_and(|s| s.name == "cloned")) && h.state().busy.is_none()
    });
    assert_eq!(git(&destination, &["log", "-1", "--format=%s"]), "Add a second line");
    assert_eq!(harness.state().repos.len(), 2, "the clone opens in a new tab");
}

#[test]
fn create_and_remove_a_worktree_from_the_interface() {
    let repo = repository();
    let parent = temp_dir();
    let mut harness = open(repo.path());
    loaded(&mut harness);

    harness.get_by_label("feature").click_secondary();
    settle(&mut harness);
    harness.get_by_label_contains("Create worktree").click();
    wait(&mut harness, "the worktree window", |h| h.state().tools.iter().any(|t| t.id() == "worktrees"));
    settle(&mut harness);
    type_into(&mut harness, "Parent folder", &parent.path().display().to_string());
    harness.get_by_label_contains("Create worktree").click();
    idle(&mut harness);
    let created = parent.path().join("feature");
    assert!(created.join("notes.txt").exists(), "the worktree folder was created");
    assert!(git(repo.path(), &["worktree", "list"]).contains("[feature]"));

    // Remove it again from its card, after confirming.
    settle(&mut harness);
    // The new worktree's card comes after the main worktree's, whose Remove is disabled.
    let remove = format!("{}  Remove", egui_phosphor::regular::TRASH);
    harness.get_all_by_label(&remove).last().expect("remove button").click();
    settle(&mut harness);
    harness.get_by_label_contains("Remove worktree").click();
    idle(&mut harness);
    assert!(!git(repo.path(), &["worktree", "list"]).contains("[feature]"));
    assert!(!created.exists());
}

#[test]
fn track_and_untrack_an_lfs_pattern() {
    let repo = repository();
    let mut harness = open(repo.path());
    loaded(&mut harness);
    let lfs_installed = Command::new("git").args(["lfs", "version"]).output().is_ok_and(|o| o.status.success());

    repository_menu(&mut harness, &folder_name(repo.path()), "Git LFS");
    wait(&mut harness, "the LFS window", |h| h.query_by_role_and_label(egui::accesskit::Role::TextInput, "LFS pattern").is_some());
    settle(&mut harness);
    type_into(&mut harness, "LFS pattern", "*.psd");
    if !lfs_installed {
        // Without git-lfs, Git would commit matching files in full, so tracking is refused
        // and the window says why.
        assert!(format!("{:?}", harness.get_by_label("Track")).contains("disabled: true"));
        assert!(harness.query_by_label_contains("Git LFS is not installed").is_some());
        return;
    }
    harness.get_by_label("Track").click();
    idle(&mut harness);
    let attributes = std::fs::read_to_string(repo.path().join(".gitattributes")).unwrap_or_default();
    assert!(attributes.contains("*.psd filter=lfs"), "tracked: {attributes}");
    wait(&mut harness, "the tracked pattern", |h| h.query_by_label("Untrack").is_some());
    settle(&mut harness);
    harness.get_by_label("Untrack").click();
    idle(&mut harness);
    let attributes = std::fs::read_to_string(repo.path().join(".gitattributes")).unwrap_or_default();
    assert!(!attributes.contains("*.psd filter=lfs"), "untracked: {attributes}");
}

#[test]
fn recent_repositories_reopen_after_their_tab_closes() {
    let first = repository();
    let second = repository();
    let mut harness = open(first.path());
    loaded(&mut harness);
    harness.state_mut().open(second.path().to_path_buf());
    loaded(&mut harness);
    assert_eq!(harness.state().repos.len(), 2);

    let first_name = folder_name(first.path());
    harness.get_by_label(&format!("Close {first_name}")).click();
    settle(&mut harness);
    assert_eq!(harness.state().repos.len(), 1);

    harness.get_by_label_contains(&format!("  {first_name}")).click();
    loaded(&mut harness);
    assert_eq!(harness.state().repos.len(), 2, "reopened from Recent");
    assert_eq!(harness.state().snapshot().map(|s| s.name.clone()), Some(first_name));
}

#[test]
fn save_an_identity_profile_and_apply_it_from_the_palette() {
    let repo = repository();
    let mut harness = open(repo.path());
    loaded(&mut harness);

    repository_menu(&mut harness, &folder_name(repo.path()), "Repository settings");
    wait(&mut harness, "the identity form", |h| h.query_by_role_and_label(egui::accesskit::Role::TextInput, "Name").is_some());
    settle(&mut harness);
    harness.get_by_label_contains("Profiles").click();
    settle(&mut harness);
    // The form reads the identity in the background; saving is enabled once it has.
    wait(&mut harness, "the identity to load", |h| {
        h.query_by_label_contains("Save as profile").is_some_and(|node| !format!("{node:?}").contains("disabled: true"))
    });
    harness.get_by_label_contains("Save as profile").click();
    settle(&mut harness);
    harness.state_mut().tools.clear();

    // Change the identity outside NiceGit, then put the saved profile back.
    git(repo.path(), &["config", "user.name", "Someone Else"]);
    harness.key_press_modifiers(Modifiers::COMMAND | Modifiers::SHIFT, Key::P);
    wait(&mut harness, "the palette", |h| h.state().palette.is_some());
    harness.event(egui::Event::Text("use identity test".into()));
    wait(&mut harness, "the profile command", |h| h.query_by_label("Use identity Test").is_some());
    harness.key_press(Key::Enter);
    idle(&mut harness);
    assert_eq!(git(repo.path(), &["config", "user.name"]), "Test");
    assert_eq!(git(repo.path(), &["config", "user.email"]), "test@example.invalid");
}

/// Loads this project's own pull requests and issues from github.com with the signed-in
/// GitHub CLI. Needs network access and `gh auth login`, so it runs only on request:
/// `cargo test -p nicegit -- --ignored github`.
#[test]
#[ignore]
fn github_pull_requests_and_issues_load_from_github() {
    let repo = repository();
    git(repo.path(), &["remote", "add", "origin", "https://github.com/daniaalnadir/NiceGit.git"]);
    let mut harness = open(repo.path());
    loaded(&mut harness);

    harness.get_by_label_contains("PULL REQUESTS").click();
    settle(&mut harness);
    harness.get_all_by_label_contains("Load from GitHub").next().expect("load button").click();
    settle(&mut harness);
    harness.get_by_label("origin").click();
    // The open pull requests list in the sidebar, read live from github.com, with their count.
    wait(&mut harness, "pull requests in the sidebar", |h| h.query_all_by_label_contains("Port NiceGit to Rust").next().is_some());
    let loaded = harness.state().repo().and_then(|r| r.github[0].as_ref()).map(|l| l.items.len()).unwrap_or(0);
    assert!(loaded >= 1, "the sidebar counts loaded pull requests");
    // The window offers closed and merged pull requests and the Issues tab.
    harness.get_by_label("Open GitHub window").click();
    wait(&mut harness, "pull requests from GitHub", |h| h.query_by_label_contains("Open in browser").is_some());
    assert!(harness.query_all_by_label_contains("Port NiceGit to Rust").next().is_some(), "lists the open pull request");
    // Closed and merged pull requests load when chosen, such as #33.
    settle(&mut harness);
    harness.get_by_label("Merged").click();
    wait(&mut harness, "merged pull requests", |h| h.query_by_label_contains("Rewrite the README").is_some());
    // The Issues tab loads too; this project may have none, which shows an empty state.
    settle(&mut harness);
    harness.get_by_label_contains("Issues").click();
    wait(&mut harness, "issues from GitHub", |h| {
        h.query_by_label_contains("Open in browser").is_some()
            || h.query_by_label_contains("No open issues").is_some()
            || h.query_by_label_contains("No issues").is_some()
    });
}

#[test]
fn double_clicking_a_panel_edge_restores_its_width() {
    use egui::containers::panel::PanelState;
    let repo = repository();
    let mut harness = open(repo.path());
    loaded(&mut harness);
    settle(&mut harness);
    let id = egui::Id::new("sidebar");
    let width = |h: &Harness<'static, NiceGitApp>| PanelState::load(&h.ctx, id).map(|s| s.size().x).unwrap_or_default();
    let default = width(&harness);

    // Drag the sidebar's edge wider.
    let edge = PanelState::load(&harness.ctx, id).expect("sidebar shown").outer_rect.right_center();
    harness.hover_at(edge);
    harness.step();
    let pressed = |down: bool, pos: egui::Pos2| egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed: down,
        modifiers: Modifiers::NONE,
    };
    harness.event(pressed(true, edge));
    harness.step();
    for step in 1..=10 {
        harness.event(egui::Event::PointerMoved(edge + egui::vec2(8.0 * step as f32, 0.0)));
        harness.step();
    }
    harness.event(pressed(false, edge + egui::vec2(80.0, 0.0)));
    settle(&mut harness);
    let dragged = width(&harness);
    assert!(dragged > default + 40.0, "the drag widened the sidebar: {default} -> {dragged}");

    // Double-click the edge: the default width comes back.
    let edge = PanelState::load(&harness.ctx, id).unwrap().outer_rect.right_center();
    harness.hover_at(edge);
    harness.step();
    for _ in 0..2 {
        harness.event(pressed(true, edge));
        harness.event(pressed(false, edge));
        harness.step();
    }
    settle(&mut harness);
    let restored = width(&harness);
    assert!((restored - default).abs() < 2.0, "restored {restored} to the default {default}");
}
