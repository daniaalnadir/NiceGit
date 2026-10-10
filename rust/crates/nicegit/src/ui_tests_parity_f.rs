//! Interface tests for the remaining details of Mac app features (round two), built on the helpers in
//! `ui_tests`.

#![allow(unused_imports)]

use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use egui::accesskit::{Role, Toggled};
use egui::{Key, Modifiers};
use egui_kittest::kittest::{By, NodeT, Queryable};
use egui_kittest::Harness;
use egui_phosphor::regular as icon;

use crate::app::NiceGitApp;
use crate::ui_tests::*;

type App = Harness<'static, NiceGitApp>;

/// The key eframe stores egui's own memory under. Panel sizes live in that memory, so a relaunch
/// must carry it across as well as the app's settings.
const EGUI_MEMORY_KEY: &str = "egui";

/// Saved app state held in memory, standing in for the platform's storage between launches.
#[derive(Clone, Default)]
struct MemoryStorage(std::collections::HashMap<String, String>);

impl eframe::Storage for MemoryStorage {
    fn get_string(&self, key: &str) -> Option<String> {
        self.0.get(key).cloned()
    }

    fn set_string(&mut self, key: &str, value: String) {
        self.0.insert(key.to_string(), value);
    }

    fn remove_string(&mut self, key: &str) {
        self.0.remove(key);
    }

    fn flush(&mut self) {}
}

/// Opens the repository as a relaunched app, given what the previous run saved. The copy is
/// leaked so the harness can own it for the whole test, as `open` does for its folder.
fn relaunch(path: &Path, saved: &MemoryStorage) -> App {
    let storage: &'static MemoryStorage = Box::leak(Box::new(saved.clone()));
    let path = path.to_path_buf();
    Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_eframe(move |cc| {
        cc.storage = Some(storage as &dyn eframe::Storage);
        // eframe restores egui's memory before the app starts; the harness does not, so do it here.
        if let Some(memory) = eframe::get_value::<egui::Memory>(storage, EGUI_MEMORY_KEY) {
            cc.egui_ctx.memory_mut(|m| *m = memory);
        }
        NiceGitApp::new(cc, Some(path))
    })
}

/// Whether any widget's label contains `text`.
fn shows(harness: &App, text: &str) -> bool {
    harness.query_all_by_label_contains(text).next().is_some()
}

/// Replaces the text in the field with this accessible label.
fn replace_text(harness: &mut App, label: &str, text: &str) {
    harness.get_by_role_and_label(Role::TextInput, label).focus();
    harness.step();
    harness.key_press_modifiers(Modifiers::COMMAND, Key::A);
    harness.step();
    harness.get_by_role_and_label(Role::TextInput, label).type_text(text);
    harness.step();
}

/// Writes a file and commits it, with the test repository's identity.
fn commit_file(dir: &Path, file: &str, contents: &str, message: &str) {
    std::fs::write(dir.join(file), contents).unwrap();
    git(dir, &["add", "--", file]);
    git(dir, &["commit", "-q", "-m", message]);
}

/// Commits a change to notes.txt as another author.
fn commit_as(dir: &Path, name: &str, email: &str, contents: &str, message: &str) {
    std::fs::write(dir.join("notes.txt"), contents).unwrap();
    let name = format!("user.name={name}");
    let email = format!("user.email={email}");
    git(dir, &["-c", name.as_str(), "-c", email.as_str(), "commit", "-qam", message]);
}

/// Whether the diff for the selected change has loaded.
fn diff_loaded(harness: &App) -> bool {
    harness.state().repo().is_some_and(|r| !r.diff_loading && r.diff.is_some())
}

/// The current width of a panel, as egui remembers it.
fn panel_width(harness: &App, panel: &str) -> f32 {
    egui::containers::panel::PanelState::load(&harness.ctx, egui::Id::new(panel)).map(|state| state.size().x).unwrap_or_default()
}

/// Drags a panel's edge by `dx` points with the pointer, as a person does. `right_edge` picks the
/// edge a right-hand or left-hand panel is resized from.
fn drag_panel_edge(harness: &mut App, panel: &str, right_edge: bool, dx: f32) {
    let state = egui::containers::panel::PanelState::load(&harness.ctx, egui::Id::new(panel)).expect("panel shown");
    let edge = if right_edge { state.outer_rect.right_center() } else { state.outer_rect.left_center() };
    let press = |down: bool, pos: egui::Pos2| egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed: down,
        modifiers: Modifiers::NONE,
    };
    harness.hover_at(edge);
    harness.step();
    harness.event(press(true, edge));
    harness.step();
    for step in 1..=10 {
        harness.event(egui::Event::PointerMoved(edge + egui::vec2(dx * step as f32 / 10.0, 0.0)));
        harness.step();
    }
    harness.event(press(false, edge + egui::vec2(dx, 0.0)));
    settle(harness);
}

/// The commit subjects of the four commits in the filter test, in graph order.
const FILTER_SUBJECTS: [&str; 4] = ["Start the notes", "Add a second line", "Tidy the margins", "Reword the title"];

/// Which of the filter test's commit subjects the history currently shows.
fn visible_subjects(harness: &App) -> Vec<&'static str> {
    FILTER_SUBJECTS.into_iter().filter(|subject| harness.query_by_label(subject).is_some()).collect()
}

#[test]
fn the_history_filter_matches_author_hash_and_reference_names() {
    let repo = repository();
    let path = repo.path();
    commit_as(path, "Alice Rivers", "alice@example.invalid", "first\nsecond\nmargins\n", "Tidy the margins");
    git(path, &["tag", "v1.0"]);
    git(path, &["branch", "release/2.0"]);
    commit_as(path, "Bo Lindqvist", "bo@example.invalid", "first\nthird\nmargins\n", "Reword the title");
    let reworded = git(path, &["rev-parse", "HEAD"]);
    let mut harness = open(path);
    loaded(&mut harness);
    assert_eq!(visible_subjects(&harness), FILTER_SUBJECTS, "every loaded commit is shown with no filter");

    // The author's name, in another case, names the commits they made and no others.
    replace_text(&mut harness, "Filter loaded commits", "ALICE RIVERS");
    harness.run_steps(3);
    assert_eq!(visible_subjects(&harness), ["Tidy the margins"], "matches the author name, ignoring case");

    replace_text(&mut harness, "Filter loaded commits", "bo lindqvist");
    harness.run_steps(3);
    assert_eq!(visible_subjects(&harness), ["Reword the title"], "matches another author's name");

    // The start of a commit's full ID finds that commit.
    replace_text(&mut harness, "Filter loaded commits", &reworded[..10]);
    harness.run_steps(3);
    assert_eq!(visible_subjects(&harness), ["Reword the title"], "matches a commit ID prefix");

    // Tags and branches are matched by name, though neither appears in the subject.
    replace_text(&mut harness, "Filter loaded commits", "v1.0");
    harness.run_steps(3);
    assert_eq!(visible_subjects(&harness), ["Tidy the margins"], "matches a tag name");

    replace_text(&mut harness, "Filter loaded commits", "release/2.0");
    harness.run_steps(3);
    assert_eq!(visible_subjects(&harness), ["Tidy the margins"], "matches a branch name");

    replace_text(&mut harness, "Filter loaded commits", "nothing has this text");
    harness.run_steps(3);
    assert!(visible_subjects(&harness).is_empty(), "text that matches nothing hides every commit");
}

#[test]
fn panel_widths_and_the_whitespace_choice_survive_a_relaunch() {
    let repo = repository();
    let path = repo.path();
    std::fs::write(path.join("notes.txt"), "first\n  second  \n").unwrap();
    let mut saved = MemoryStorage::default();
    let (sidebar, changes, default_sidebar, default_changes);
    {
        let mut harness = open(path);
        loaded(&mut harness);
        settle(&mut harness);
        default_sidebar = panel_width(&harness, "sidebar");
        default_changes = panel_width(&harness, "changes");

        drag_panel_edge(&mut harness, "sidebar", true, 90.0);
        drag_panel_edge(&mut harness, "changes", false, -90.0);
        sidebar = panel_width(&harness, "sidebar");
        changes = panel_width(&harness, "changes");
        assert!(sidebar > default_sidebar + 40.0, "the sidebar was widened: {default_sidebar} -> {sidebar}");
        assert!(changes > default_changes + 40.0, "the Changes panel was widened: {default_changes} -> {changes}");

        // Hide whitespace is set from the diff's own toolbar.
        harness.get_by_label("notes.txt").click();
        wait(&mut harness, "the diff", diff_loaded);
        settle(&mut harness);
        harness.get_by_role_and_label(Role::CheckBox, "Hide whitespace").click();
        wait(&mut harness, "the whitespace choice", |h| h.state().settings.ignore_whitespace);
        settle(&mut harness);

        // Quitting saves the app state and egui's memory, as eframe does when the window closes.
        eframe::App::save(harness.state_mut(), &mut saved);
        harness.ctx.memory(|memory| eframe::set_value(&mut saved, EGUI_MEMORY_KEY, memory));
    }

    let mut harness = relaunch(path, &saved);
    loaded(&mut harness);
    settle(&mut harness);
    assert!(harness.state().settings.ignore_whitespace, "the whitespace choice is restored");
    assert!((panel_width(&harness, "sidebar") - sidebar).abs() < 2.0, "the sidebar keeps its width after a relaunch");
    assert!((panel_width(&harness, "changes") - changes).abs() < 2.0, "the Changes panel keeps its width after a relaunch");

    // The Settings window shows the restored choice as on.
    harness.key_press_modifiers(Modifiers::COMMAND, Key::Comma);
    wait(&mut harness, "the Settings window", |h| h.query_by_role_and_label(Role::CheckBox, "Hide whitespace-only changes").is_some());
    settle(&mut harness);
    let toggled = harness.get_by_role_and_label(Role::CheckBox, "Hide whitespace-only changes").accesskit_node().toggled();
    assert_eq!(toggled, Some(Toggled::True), "the Settings window shows the whitespace choice as on");
}

#[test]
fn undo_an_amend_restores_the_old_commit_and_keeps_the_change_staged() {
    let repo = repository();
    let path = repo.path();
    let original = git(path, &["rev-parse", "HEAD"]);
    std::fs::write(path.join("forgotten.txt"), "left out\n").unwrap();
    git(path, &["add", "forgotten.txt"]);
    let mut harness = open(path);
    loaded(&mut harness);

    harness.get_by_role_and_label(Role::CheckBox, "Amend last commit").click();
    let amend = format!("{}  Amend last commit", icon::CHECK_CIRCLE);
    wait(&mut harness, "the amend button", |h| h.query_by_role_and_label(Role::Button, &amend).is_some());
    settle(&mut harness);
    harness.get_by_role_and_label(Role::Button, &amend).click();
    idle(&mut harness);
    wait(&mut harness, "the amended commit", |_| git(path, &["ls-tree", "--name-only", "HEAD"]).contains("forgotten.txt"));
    assert_ne!(git(path, &["rev-parse", "HEAD"]), original, "the amend made a new commit");

    toolbar_button(&mut harness, "Undo");
    wait(&mut harness, "the undo confirmation", |h| h.state().dialog.is_some());
    settle(&mut harness);
    harness.get_all_by_label("Undo").last().expect("confirm button").click();
    idle(&mut harness);
    wait(&mut harness, "HEAD back on the original commit", |_| git(path, &["rev-parse", "HEAD"]) == original);
    assert!(!git(path, &["ls-tree", "--name-only", "HEAD"]).contains("forgotten.txt"), "the old commit has no new file");
    assert_eq!(git(path, &["log", "-1", "--format=%s"]), "Add a second line");
    assert_eq!(git(path, &["diff", "--cached", "--name-only"]), "forgotten.txt", "the change is staged again");
}

#[test]
fn apply_an_identity_profile_from_repository_settings() {
    let repo = repository();
    let path = repo.path();
    let mut harness = open(path);
    loaded(&mut harness);

    repository_menu(&mut harness, &folder_name(path), "Repository settings");
    wait(&mut harness, "the identity form", |h| h.query_by_role_and_label(Role::TextInput, "Name").is_some());
    // The form reads the identity in the background; its name field fills in once it has.
    wait(&mut harness, "the identity to load", |h| {
        h.get_by_role_and_label(Role::TextInput, "Name").accesskit_node().value().as_deref() == Some("Test")
    });
    // The profile is saved from the identity on this tab, through the Profiles tab's button.
    replace_text(&mut harness, "Name", "Riley Park");
    replace_text(&mut harness, "Email", "riley@example.invalid");
    settle(&mut harness);
    harness.get_by_label_contains("Profiles").click();
    settle(&mut harness);
    wait(&mut harness, "the Save as profile button", |h| h.query_by_label_contains("Save as profile").is_some());
    harness.get_by_label_contains("Save as profile").click();
    settle(&mut harness);

    // The identity changes outside NiceGit; the saved profile puts the one shown back.
    git(path, &["config", "user.name", "Someone Else"]);
    git(path, &["config", "user.email", "else@example.invalid"]);
    let apply = format!("{}  Apply", icon::CHECK);
    wait(&mut harness, "the saved profile's Apply button", |h| h.query_by_label(&apply).is_some());
    settle(&mut harness);
    harness.get_by_label(&apply).click();
    idle(&mut harness);
    assert_eq!(git(path, &["config", "user.name"]), "Riley Park");
    assert_eq!(git(path, &["config", "user.email"]), "riley@example.invalid");
}

#[test]
fn search_history_finds_a_commit_older_than_the_loaded_page() {
    let dir = temp_dir();
    let path = dir.path();
    git(path, &["init", "-q", "-b", "main"]);
    // More commits than one page loads. The oldest carries a message no other commit has.
    let total = crate::app::PAGE + 20;
    let mut stream = String::new();
    for index in 1..=total {
        let message = if index == 1 { "The very first note\n".to_string() } else { format!("Commit number {index}\n") };
        stream.push_str(&format!(
            "commit refs/heads/main\ncommitter Test <test@example.invalid> {} +0000\ndata {}\n{message}\n",
            1_000_000_000 + index,
            message.len()
        ));
    }
    let mut import = Command::new("git").args(["fast-import", "--quiet"]).current_dir(path).stdin(Stdio::piped()).spawn().unwrap();
    import.stdin.take().unwrap().write_all(stream.as_bytes()).unwrap();
    assert!(import.wait().unwrap().success());
    let oldest = git(path, &["rev-list", "--max-parents=0", "HEAD"]);
    let mut harness = open(path);
    loaded(&mut harness);
    let not_loaded =
        harness.state().snapshot().is_some_and(|s| s.commits.len() == crate::app::PAGE && !s.commits.iter().any(|c| c.hash == oldest));
    assert!(not_loaded, "the oldest commit is outside the loaded page");

    harness.get_by_label_contains("Search all history").click();
    wait(&mut harness, "the commit search window", |h| h.state().tools.iter().any(|t| t.title() == "Search commits"));
    settle(&mut harness);
    // The search field takes focus when the window opens.
    harness.event(egui::Event::Text("very first note".into()));
    harness.step();
    harness.key_press(Key::Enter);
    wait(&mut harness, "the search result", |h| shows(h, "The very first note · Test"));
    harness.get_by_label_contains("The very first note · Test").click();
    wait(
        &mut harness,
        "the oldest commit selected",
        |h| matches!(h.state().repo().map(|r| &r.selection), Some(crate::app::Selection::Commit { hash, .. }) if *hash == oldest),
    );
}

#[test]
fn compare_shows_the_detail_of_the_selected_file() {
    let repo = repository();
    let path = repo.path();
    commit_file(path, "readme.txt", "Readme body\n", "Add a readme");
    let mut harness = open(path);
    loaded(&mut harness);

    // Against the first commit, notes.txt changed and readme.txt was added.
    harness.get_by_label("Start the notes").click_secondary();
    settle(&mut harness);
    harness.get_by_label_contains("Compare with working files").click();
    wait(&mut harness, "the changed files", |h| h.query_by_role_and_label(Role::Button, "readme.txt").is_some());
    settle(&mut harness);

    harness.get_by_role_and_label(Role::Button, "readme.txt").click();
    // The detail pane titles the file it shows, and the list's own entry is not that title.
    wait(&mut harness, "the readme.txt detail", |h| {
        h.query_by_role_and_label(Role::Label, "readme.txt").is_some() && h.query_by_label("Loading the change").is_none()
    });
    assert!(harness.query_by_role_and_label(Role::Label, "notes.txt").is_none(), "the previous file's detail is gone");
    assert!(harness.query_by_label("No changes to show").is_none(), "the file's diff has lines to show");
}

#[test]
fn restore_this_version_brings_back_the_files_content_at_that_commit() {
    let repo = repository();
    let path = repo.path();
    // The file changes again after the commit under test, and has unsaved edits on top.
    commit_file(path, "notes.txt", "first\nthird\n", "Rewrite the second line");
    std::fs::write(path.join("notes.txt"), "unsaved edits\n").unwrap();
    let mut harness = open(path);
    loaded(&mut harness);

    harness.get_by_label("Add a second line").click();
    wait(&mut harness, "the commit's files", |h| h.state().repo().is_some_and(|r| !r.commit_files.is_empty()));
    settle(&mut harness);
    harness.get_by_label("notes.txt").click_secondary();
    settle(&mut harness);
    harness.get_by_label_contains("Restore this version").click();
    wait(&mut harness, "the restore confirmation", |h| h.state().dialog.is_some());
    harness.get_by_label("Restore").click();
    idle(&mut harness);
    assert_eq!(std::fs::read_to_string(path.join("notes.txt")).unwrap(), "first\nsecond\n", "the commit's own version is in the file");
    assert_eq!(git(path, &["show", ":notes.txt"]), "first\nsecond", "the index holds the commit's version too");
    assert_eq!(git(path, &["diff", "--cached", "--name-only"]), "notes.txt", "the restored change is staged");
}

#[test]
fn the_smallest_window_keeps_the_key_controls_on_screen() {
    let repo = repository();
    let path = repo.path();
    std::fs::write(path.join("notes.txt"), "changed\n").unwrap();
    let folder = path.to_path_buf();
    let size = egui::vec2(900.0, 560.0);
    let mut harness = Harness::builder().with_size(size).build_eframe(move |cc| NiceGitApp::new(cc, Some(folder)));
    loaded(&mut harness);
    harness.state_mut().select_working_tree();
    settle(&mut harness);

    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, size);
    let check = |what: &str, rect: egui::Rect| {
        let inside = rect.min.x >= screen.min.x
            && rect.min.y >= screen.min.y
            && rect.max.x <= screen.max.x
            && rect.max.y <= screen.max.y
            && rect.width() > 0.0
            && rect.height() > 0.0;
        assert!(inside, "{what} lies within the {size:?} window: {rect:?}");
    };

    // The toolbar shows its buttons, or folds them into More when they do not fit.
    let toolbar = match harness.query_by_role_and_label(Role::Button, "Undo") {
        Some(undo) => undo.rect(),
        None => harness.get_by_label("More").rect(),
    };
    check("the toolbar", toolbar);
    check("the commit summary field", harness.get_by_role_and_label(Role::TextInput, "Commit summary").rect());
    check("the first history row", harness.get_by_label("Start the notes").rect());
    check("the newest history row", harness.get_by_label("Add a second line").rect());
}

#[test]
fn command_o_opens_a_repository_chosen_in_the_folder_dialog() {
    let first = repository();
    let second = repository();
    let mut harness = open(first.path());
    loaded(&mut harness);

    crate::file_dialog::answer_next(second.path());
    harness.key_press_modifiers(Modifiers::COMMAND, Key::O);
    wait(&mut harness, "the second repository", |h| h.state().repos.len() == 2);
    loaded(&mut harness);
    let active = harness.state().repo().map(|r| r.path.canonicalize().unwrap());
    assert_eq!(active, Some(second.path().canonicalize().unwrap()), "the chosen repository opens in a new, active tab");
}

#[test]
fn option_command_f_opens_file_content_search() {
    let repo = repository();
    let mut harness = open(repo.path());
    loaded(&mut harness);

    harness.key_press_modifiers(Modifiers::COMMAND | Modifiers::ALT, Key::F);
    wait(&mut harness, "the content search window", |h| h.state().tools.iter().any(|t| t.id().starts_with("content-search")));
    assert!(harness.query_by_role_and_label(Role::TextInput, "Text in files").is_some());
}

#[test]
fn shift_command_f_opens_history_search() {
    let repo = repository();
    let mut harness = open(repo.path());
    loaded(&mut harness);

    harness.key_press_modifiers(Modifiers::COMMAND | Modifiers::SHIFT, Key::F);
    wait(&mut harness, "the history search window", |h| h.state().tools.iter().any(|t| t.id() == "commit-search"));
}

#[test]
fn returning_to_the_app_during_an_action_refreshes_once_it_finishes() {
    let repo = repository();
    let path = repo.path();
    let mut harness = open(path);
    loaded(&mut harness);
    harness.state_mut().settings.auto_refresh = false;
    let focus = |harness: &mut Harness<'static, NiceGitApp>, focused: bool| {
        harness.input_mut().viewports.entry(egui::ViewportId::ROOT).or_default().focused = Some(focused);
        harness.run_steps(2);
    };

    focus(&mut harness, false);
    harness.state_mut().act("A slow action", |_, _| {
        std::thread::sleep(Duration::from_millis(1500));
        Ok(None)
    });
    let action = harness.state().repo().unwrap().generation;
    focus(&mut harness, true);
    // The return is remembered while the action runs, and no second load starts beside it.
    let start = Instant::now();
    while harness.state().busy.is_some() {
        assert!(harness.state().activation_refresh, "the refresh for returning to the app waits");
        assert_eq!(harness.state().repo().unwrap().generation, action, "no load starts during the action");
        assert!(start.elapsed() < Duration::from_secs(20), "the action finishes");
        harness.step();
        std::thread::sleep(Duration::from_millis(15));
    }
    // Once the action and its own refresh are done, the postponed refresh runs.
    wait(&mut harness, "the postponed refresh", |h| {
        !h.state().activation_refresh && h.state().repo().is_some_and(|r| r.generation > action)
    });
}

#[test]
fn open_a_repository_from_the_repository_menu_and_the_empty_window() {
    let first = repository();
    let second = repository();
    let third = repository();
    let mut harness = open(first.path());
    loaded(&mut harness);

    crate::file_dialog::answer_next(second.path());
    repository_menu(&mut harness, &folder_name(first.path()), "Open repository");
    wait(&mut harness, "the second repository", |h| h.state().repos.len() == 2);
    loaded(&mut harness);
    assert_eq!(harness.state().repo().map(|r| r.path.canonicalize().unwrap()), Some(second.path().canonicalize().unwrap()));

    // With nothing open, the empty window offers the same choice.
    let mut empty = Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_eframe(|cc| NiceGitApp::new(cc, None));
    settle(&mut empty);
    crate::file_dialog::answer_next(third.path());
    empty.get_by_label(&format!("{}  Open repository…", icon::FOLDER_OPEN)).click();
    wait(&mut empty, "the chosen repository", |h| h.state().snapshot().is_some() && h.state().busy.is_none());
    assert_eq!(empty.state().repo().map(|r| r.path.canonicalize().unwrap()), Some(third.path().canonicalize().unwrap()));
}

#[test]
fn fetch_and_refresh_from_the_toolbar() {
    let repo = repository();
    let path = repo.path();
    let remote = temp_dir();
    git(remote.path(), &["init", "-q", "--bare", "-b", "main"]);
    git(path, &["remote", "add", "origin", &remote.path().to_string_lossy()]);
    git(path, &["push", "-q", "-u", "origin", "main"]);
    let other = temp_dir();
    let clone = other.path().join("clone");
    git(other.path(), &["clone", "-q", &remote.path().to_string_lossy(), "clone"]);
    git(&clone, &["config", "user.name", "Other"]);
    git(&clone, &["config", "user.email", "other@example.invalid"]);
    std::fs::write(clone.join("theirs.txt"), "theirs\n").unwrap();
    git(&clone, &["add", "theirs.txt"]);
    git(&clone, &["commit", "-qm", "Work from elsewhere"]);
    git(&clone, &["push", "-q", "origin", "main"]);
    let theirs = git(&clone, &["rev-parse", "HEAD"]);
    let mut harness = open(path);
    loaded(&mut harness);
    harness.state_mut().settings.auto_refresh = false;

    toolbar_button(&mut harness, "Fetch");
    idle(&mut harness);
    wait(&mut harness, "the fetched commit", |_| git(path, &["rev-parse", "origin/main"]) == theirs);
    wait(&mut harness, "one commit to pull", |h| h.state().snapshot().and_then(|s| s.behind) == Some(1));

    // A change made outside, with automatic refresh off, appears after Refresh.
    std::fs::write(path.join("outside.txt"), "outside\n").unwrap();
    toolbar_button(&mut harness, "Refresh");
    idle(&mut harness);
    wait(&mut harness, "the refreshed changes", |h| {
        h.state().snapshot().is_some_and(|s| s.status.iter().any(|entry| entry.path == "outside.txt"))
    });
}

#[test]
fn outside_changes_wait_while_a_review_window_is_open() {
    // As in the Mac app's sheets, these windows hold refreshes while they are open.
    for (item, id) in [("Stashes", "stashes"), ("Repository settings", "repository-settings")] {
        let repo = repository();
        let path = repo.path();
        let mut harness = open(path);
        loaded(&mut harness);
        let listed =
            |h: &Harness<'static, NiceGitApp>| h.state().snapshot().is_some_and(|s| s.status.iter().any(|e| e.path == "outside.txt"));

        repository_menu(&mut harness, &folder_name(path), item);
        wait(&mut harness, item, |h| h.state().tools.iter().any(|t| t.id() == id));
        std::thread::sleep(Duration::from_millis(600));
        std::fs::write(path.join("outside.txt"), "made in another app\n").unwrap();
        let start = Instant::now();
        while start.elapsed() < Duration::from_millis(1500) {
            harness.step();
            std::thread::sleep(Duration::from_millis(30));
        }
        assert!(!listed(&harness), "nothing refreshes under the {item} window");

        harness.state_mut().tools.clear();
        wait(&mut harness, "the refresh once it closes", listed);
    }
}
