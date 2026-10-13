//! Renders the app offscreen to PNG files for design review, without needing a display. Run with
//! `NICEGIT_SCREENSHOTS=<folder> cargo test -p nicegit screenshots -- --ignored`, optionally with
//! `NICEGIT_SCREENSHOT_REPO=<repository>`; otherwise a small repository is made for the run.

use std::path::{Path, PathBuf};

use egui::accesskit::Role;
use egui_kittest::kittest::{By, Queryable};
use egui_kittest::Harness;
use egui_phosphor::regular as icon;

use crate::app::NiceGitApp;
use crate::theme::Appearance;
use crate::ui_tests::*;
use crate::ui_tests_more::{publish_to_bare, wait_for_stageable_lines};

type App = Harness<'static, NiceGitApp>;

fn open_rendering(path: &Path, appearance: Appearance) -> App {
    open_rendering_at(path, appearance, egui::vec2(1440.0, 900.0))
}

fn open_rendering_at(path: &Path, appearance: Appearance, size: egui::Vec2) -> App {
    let path: PathBuf = path.to_path_buf();
    let mut harness = Harness::builder().with_size(size).wgpu().build_eframe(move |cc| NiceGitApp::new(cc, Some(path)));
    harness.state_mut().settings.appearance = appearance;
    loaded(&mut harness);
    harness
}

fn save(harness: &mut App, folder: &Path, name: &str) {
    harness.run_steps(4);
    let image = harness.render().expect("render the frame");
    image.save(folder.join(format!("{name}.png"))).expect("save the image");
}

#[test]
#[ignore = "renders design-review screenshots; run on request"]
fn screenshots() {
    let Some(folder) = std::env::var_os("NICEGIT_SCREENSHOTS").map(PathBuf::from) else {
        eprintln!("Set NICEGIT_SCREENSHOTS to a folder to render screenshots.");
        return;
    };
    std::fs::create_dir_all(&folder).unwrap();
    let made = repository();
    let path = std::env::var_os("NICEGIT_SCREENSHOT_REPO").map(PathBuf::from).unwrap_or_else(|| {
        let long = format!("first\nsecond, now with a much longer line that {}\n", "keeps going ".repeat(16));
        std::fs::write(made.path().join("notes.txt"), long).unwrap();
        made.path().to_path_buf()
    });
    let changed = git(&path, &["diff", "--name-only"]);
    let file = changed.lines().next().unwrap_or("notes.txt").to_string();

    for (theme, appearance) in [("dark", Appearance::Dark), ("light", Appearance::Light)] {
        let mut harness = open_rendering(&path, appearance);
        save(&mut harness, &folder, &format!("main-{theme}"));

        // Commits waiting to be pulled, counted on the Fetch button.
        let active = harness.state().active;
        let behind = harness.state().repos[active].snapshot.as_ref().and_then(|s| s.behind);
        harness.state_mut().repos[active].snapshot.as_mut().unwrap().behind = Some(3);
        save(&mut harness, &folder, &format!("toolbar-badge-{theme}"));
        harness.state_mut().repos[active].snapshot.as_mut().unwrap().behind = behind;

        // The menu that chooses what the Fetch and Pull button does.
        harness.get_by_label("Fetch and Pull options").click();
        wait(&mut harness, "the Fetch and Pull menu", |h| h.query_by_role_and_label(Role::RadioButton, "Pull").is_some());
        save(&mut harness, &folder, &format!("sync-menu-{theme}"));
        harness.key_press(egui::Key::Escape);
        harness.run_steps(2);

        // A changed file's diff. Side by side is rendered below from a wider window, since this
        // diff panel is narrower than it needs and would show the same single column.
        harness.get_all_by_label(&file).next().expect("a changed file").click();
        wait(&mut harness, "the diff", |h| h.state().repo().is_some_and(|r| r.diff.is_some() && !r.diff_loading));
        save(&mut harness, &folder, &format!("diff-unified-{theme}"));

        // A change's own buttons, shown while the pointer is over it.
        wait(&mut harness, "stageable lines", |h| h.state().repo().is_some_and(|r| r.diff_options.selectable));
        let field = harness.get_by_label("Find in diff").rect();
        for step in 0..120 {
            harness.hover_at(egui::pos2(field.center().x + 120.0, field.bottom() + 8.0 + step as f32 * 5.0));
            harness.run_steps(2);
            if harness.query_by_label("Stage this change").is_some() {
                break;
            }
        }
        save(&mut harness, &folder, &format!("diff-change-buttons-{theme}"));

        // The built-in editor on the same file.
        let edit = format!("{}  Edit", icon::PENCIL_SIMPLE);
        if harness.query_by_label(&edit).is_some() {
            harness.get_by_label(&edit).click_accesskit();
            wait(&mut harness, "the editor", |h| h.query_by_role_and_label(Role::TextInput, "File contents").is_some());
            save(&mut harness, &folder, &format!("editor-{theme}"));
            harness.state_mut().tools.clear();
        }

        // Settings over the main window.
        harness.state_mut().show_settings = true;
        save(&mut harness, &folder, &format!("settings-{theme}"));
        harness.state_mut().show_settings = false;

        // Something dragged over the window.
        harness.input_mut().hovered_files.push(egui::HoveredFile::default());
        save(&mut harness, &folder, &format!("drop-target-{theme}"));
        harness.input_mut().hovered_files.clear();

        // The interface at 130%, as someone who has made everything larger sees it.
        harness.ctx.set_zoom_factor(1.3);
        harness.run_steps(3);
        save(&mut harness, &folder, &format!("large-{theme}"));
        harness.ctx.set_zoom_factor(1.0);
        harness.run_steps(3);

        // The keyboard shortcuts.
        harness.state_mut().show_shortcuts = true;
        save(&mut harness, &folder, &format!("shortcuts-{theme}"));
        harness.state_mut().show_shortcuts = false;

        // Side by side needs a diff panel at least 560 points wide, which a 1440-point window
        // with both sidebars open does not leave; a wider window shows it in effect.
        let mut wide = open_rendering_at(&path, appearance, egui::vec2(1920.0, 1080.0));
        wide.state_mut().settings.split_diff = true;
        wide.get_all_by_label(&file).next().expect("a changed file").click();
        wait(&mut wide, "the diff", |h| h.state().repo().is_some_and(|r| r.diff.is_some() && !r.diff_loading));
        save(&mut wide, &folder, &format!("diff-split-wide-{theme}"));

        // The history at full width, with every column, and its header's menu of columns.
        wide.state_mut().close_diff();
        wait(&mut wide, "the history", |h| h.query_by_label("History columns").is_some());
        save(&mut wide, &folder, &format!("history-wide-{theme}"));
        // A commit's inspector, with each file's line counts.
        let head = wide.state().snapshot().and_then(|s| s.head_hash.clone()).expect("a commit");
        wide.state_mut().select_commit(head);
        wait(&mut wide, "the commit's files", |h| h.state().repo().is_some_and(|r| !r.commit_files.is_empty()));
        save(&mut wide, &folder, &format!("inspector-{theme}"));
        wide.state_mut().clear_selection();
        let active = wide.state().active;
        let behind = wide.state().repos[active].snapshot.as_ref().and_then(|s| s.behind);
        wide.state_mut().repos[active].snapshot.as_mut().unwrap().behind = Some(3);
        save(&mut wide, &folder, &format!("toolbar-badge-wide-{theme}"));
        wide.state_mut().repos[active].snapshot.as_mut().unwrap().behind = behind;
        wide.get_by_label("History columns").click_secondary();
        wait(&mut wide, "the columns menu", |h| h.query_by_label("Restore column widths").is_some());
        save(&mut wide, &folder, &format!("history-columns-menu-{theme}"));
    }
}

// MARK: Design review of new interface pieces

fn review_folder() -> Option<PathBuf> {
    let folder = std::env::var_os("NICEGIT_SCREENSHOTS").map(PathBuf::from)?;
    std::fs::create_dir_all(&folder).unwrap();
    Some(folder)
}

fn commit_file(dir: &Path, file: &str, contents: &str, message: &str) {
    std::fs::write(dir.join(file), contents).unwrap();
    git(dir, &["add", "--", file]);
    git(dir, &["commit", "-q", "-m", message]);
}

fn commit_as(dir: &Path, name: &str, email: &str, message: &str) {
    let (name, email) = (format!("user.name={name}"), format!("user.email={email}"));
    git(dir, &["-c", &name, "-c", &email, "commit", "-q", "--allow-empty", "-m", message]);
}

fn shows_text(harness: &App, text: &str) -> bool {
    harness.query_all_by_label_contains(text).next().is_some()
}

/// Replaces the text in the sidebar's filter field.
fn filter_references(harness: &mut App, text: &str) {
    harness.get_by_role_and_label(Role::TextInput, "Filter references").focus();
    harness.step();
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    harness.step();
    harness.get_by_role_and_label(Role::TextInput, "Filter references").type_text(text);
    harness.run_steps(3);
}

#[test]
#[ignore = "renders design-review screenshots; run on request"]
fn review_screenshots() {
    let Some(folder) = review_folder() else {
        eprintln!("Set NICEGIT_SCREENSHOTS to a folder to render screenshots.");
        return;
    };
    diff_context_shots(&folder);
    history_scope_shots(&folder);
    changes_panel_shots(&folder);
    trailers_shots(&folder);
    commit_template_shots(&folder);
    stash_inspector_shots(&folder);
    rebase_and_reword_shots(&folder);
    commit_menu_shots(&folder);
    remote_branch_menu_shots(&folder);
    new_tag_shots(&folder);
    force_push_shots(&folder);
    settings_shots(&folder);
}

fn diff_context_shots(folder: &Path) {
    let repo = repository();
    let path = repo.path();
    let body: String = (1..=40).map(|n| format!("line {n}\n")).collect();
    std::fs::write(path.join("long.txt"), &body).unwrap();
    git(path, &["add", "long.txt"]);
    git(path, &["commit", "-q", "-m", "Add a long file"]);
    std::fs::write(path.join("long.txt"), body.replace("line 20\n", "twenty\n")).unwrap();
    for width in [1440.0, 1100.0] {
        let mut harness = open_rendering_at(path, Appearance::Dark, egui::vec2(width, 900.0));
        harness.get_all_by_label("long.txt").next().expect("the changed file").click();
        wait_for_stageable_lines(&mut harness);
        save(&mut harness, folder, &format!("diff-header-{width}-dark"));
        harness.get_by_role_and_label(Role::ComboBox, "Context lines").click();
        settle(&mut harness);
        save(&mut harness, folder, &format!("diff-context-menu-{width}-dark"));
        harness.key_press(egui::Key::Escape);
        harness.run_steps(2);
    }
}

fn history_scope_shots(folder: &Path) {
    let repo = repository();
    let path = repo.path();
    let _remote = publish_to_bare(path);
    git(path, &["checkout", "-q", "feature"]);
    commit_file(path, "feature.txt", "feature\n", "Feature only work");
    git(path, &["checkout", "-q", "main"]);
    for width in [1440.0, 1100.0] {
        let mut harness = open_rendering_at(path, Appearance::Dark, egui::vec2(width, 900.0));
        wait(&mut harness, "the feature commit in the graph", |h| h.query_by_label("Feature only work").is_some());
        harness.get_by_role_and_label(Role::ComboBox, "Branches shown").click();
        settle(&mut harness);
        save(&mut harness, folder, &format!("history-branches-menu-{width}-dark"));
        harness.get_by_label("Local branches").click();
        wait(&mut harness, "the local branches note", |h| shows_text(h, "Showing local branches."));
        save(&mut harness, folder, &format!("history-branches-local-{width}-dark"));
    }
}

fn changes_panel_shots(folder: &Path) {
    let repo = repository();
    let path = repo.path();
    for n in 0..8 {
        std::fs::write(path.join(format!("file{n:02}.txt")), "x\n").unwrap();
    }
    std::fs::write(path.join("notes.txt"), "changed\n").unwrap();
    git(path, &["add", "--", "file00.txt", "file01.txt", "file02.txt"]);
    for (appearance, theme) in [(Appearance::Dark, "dark"), (Appearance::Light, "light")] {
        let mut harness = open_rendering_at(path, appearance, egui::vec2(1440.0, 900.0));
        wait(&mut harness, "the staged files", |h| shows_text(h, "Staged files (3)"));
        save(&mut harness, folder, &format!("changes-panel-{theme}"));
        if theme == "dark" {
            harness.get_by_label("Filter changed files").click();
            harness.step();
            harness.event(egui::Event::Text("file0".into()));
            wait(&mut harness, "the filtered headers", |h| shows_text(h, "Unstaged files (5 of 6)"));
            save(&mut harness, folder, "changes-panel-filtered-dark");
        }
    }
}

fn trailers_shots(folder: &Path) {
    let repo = repository();
    let path = repo.path();
    commit_as(path, "Ana", "ana@example.com", "Review the graph");
    commit_as(path, "Ben", "ben@example.com", "Tidy the sidebar");
    std::fs::write(path.join("new.txt"), "new\n").unwrap();
    git(path, &["add", "--", "new.txt"]);
    let mut harness = open_rendering_at(path, Appearance::Dark, egui::vec2(1440.0, 900.0));
    loaded(&mut harness);
    type_into(&mut harness, "Commit summary", "Add a new file");
    harness.get_by_label("Add trailers").click();
    wait(&mut harness, "the co-author rows", |h| shows_text(h, "Co-authored-by: Ana"));
    settle(&mut harness);
    save(&mut harness, folder, "add-trailers-menu-dark");
    harness.key_press(egui::Key::Escape);
    settle(&mut harness);
}

fn commit_template_shots(folder: &Path) {
    let repo = repository();
    let path = repo.path();
    let template = temp_dir();
    let file = template.path().join("message.txt");
    std::fs::write(&file, "Fix:\n\nDetails\n").unwrap();
    git(path, &["config", "commit.template", &file.to_string_lossy()]);
    let mut harness = open_rendering_at(path, Appearance::Dark, egui::vec2(1440.0, 900.0));
    wait(&mut harness, "the commit template", |h| h.state().repo().is_some_and(|r| r.draft.summary == "Fix:"));
    wait(&mut harness, "the template label", |h| shows_text(h, "From commit.template"));
    save(&mut harness, folder, "commit-template-dark");
}

fn stash_inspector_shots(folder: &Path) {
    let repo = repository();
    let path = repo.path();
    std::fs::write(path.join("notes.txt"), "changed\n").unwrap();
    std::fs::write(path.join("fresh.txt"), "fresh\n").unwrap();
    git(path, &["stash", "push", "-q", "-u", "-m", "Draft work"]);
    let mut harness = open_rendering_at(path, Appearance::Dark, egui::vec2(1440.0, 900.0));
    loaded(&mut harness);
    harness.get_by_label_contains("Draft work").click();
    wait(&mut harness, "the stash's files", |h| h.query_all_by_label("notes.txt").next().is_some());
    settle(&mut harness);
    harness.get_all_by_label("notes.txt").next().expect("the stashed file").click_secondary();
    wait(&mut harness, "the stash file's menu", |h| shows_text(h, "Restore this file from the stash"));
    settle(&mut harness);
    save(&mut harness, folder, "stash-file-menu-dark");
}

fn rebase_and_reword_shots(folder: &Path) {
    let repo = repository();
    let path = repo.path();
    commit_file(path, "keep.txt", "keep\n", "Keep this commit");
    commit_file(path, "change.txt", "change\n", "Change this commit");
    commit_file(path, "also.txt", "also\n", "Keep this one too");
    let mut harness = open_rendering_at(path, Appearance::Dark, egui::vec2(1440.0, 900.0));
    loaded(&mut harness);

    harness.get_by_label("Add a second line").click_secondary();
    settle(&mut harness);
    harness.get_by_label_contains("Interactive rebase from here").click();
    let action = "Action for Change this commit";
    wait(&mut harness, "the rebase plan", |h| h.query_by_role_and_label(Role::ComboBox, action).is_some());
    settle(&mut harness);
    harness.get_by_role_and_label(Role::ComboBox, action).click();
    settle(&mut harness);
    // Each choice explains itself on hover, so the Edit choice's help is shown before picking it.
    let edit_choice = harness.get_all_by_label("Edit").last().expect("the Edit choice").rect().center();
    harness.hover_at(edit_choice);
    wait(&mut harness, "the Edit choice's help", |h| shows_text(h, "Stop after this commit"));
    save(&mut harness, folder, "rebase-edit-choice-help-dark");
    harness.get_all_by_label("Edit").last().expect("the Edit choice").click();
    settle(&mut harness);
    save(&mut harness, folder, "rebase-window-edit-dark");
    harness.get_by_label("Cancel").click();
    wait(&mut harness, "the rebase window to close", |h| h.state().tools.iter().all(|t| t.title() != "Interactive rebase"));

    // The HEAD commit has its own menu; an older commit's Edit message opens the reword window.
    harness.get_by_label("Keep this commit").click_secondary();
    settle(&mut harness);
    harness.get_by_label_contains("Edit message").click();
    wait(&mut harness, "the message editor", |h| h.state().tools.iter().any(|t| t.title() == "Edit commit message"));
    wait(&mut harness, "the reworded commit to load", |h| shows_text(h, "Message for Keep this commit"));
    settle(&mut harness);
    save(&mut harness, folder, "edit-message-window-dark");
}

fn commit_menu_shots(folder: &Path) {
    let repo = repository();
    let path = repo.path();
    git(path, &["remote", "add", "origin", "https://github.com/daniaalnadir/NiceGit.git"]);
    std::fs::write(path.join("notes.txt"), "first\nsecond\nthird\n").unwrap();
    git(path, &["add", "--", "notes.txt"]);
    let mut harness = open_rendering_at(path, Appearance::Dark, egui::vec2(1440.0, 900.0));
    loaded(&mut harness);
    harness.get_by_label("Start the notes").click_secondary();
    wait(&mut harness, "the commit menu", |h| shows_text(h, "Save as ZIP"));
    settle(&mut harness);
    save(&mut harness, folder, "commit-menu-older-dark");
}

fn remote_branch_menu_shots(folder: &Path) {
    let repo = repository();
    let path = repo.path();
    let _remote = publish_to_bare(path);
    git(path, &["push", "-q", "origin", "feature:review"]);
    git(path, &["fetch", "-q", "origin"]);
    let mut harness = open_rendering_at(path, Appearance::Dark, egui::vec2(1440.0, 900.0));
    loaded(&mut harness);
    filter_references(&mut harness, "review");
    wait(&mut harness, "the remote branch row", |h| h.query_all_by_label("review").next().is_some());
    harness.get_all_by_label("review").next().expect("the remote branch row").click_secondary();
    wait(&mut harness, "the remote branch menu", |h| shows_text(h, "Delete origin/review"));
    settle(&mut harness);
    save(&mut harness, folder, "remote-branch-menu-dark");
}

fn new_tag_shots(folder: &Path) {
    let repo = repository();
    let path = repo.path();
    let _remote = publish_to_bare(path);
    let mut harness = open_rendering_at(path, Appearance::Dark, egui::vec2(1440.0, 900.0));
    loaded(&mut harness);
    harness.get_by_label("Add a second line").click_secondary();
    settle(&mut harness);
    harness.get_by_label_contains("Create tag here").click();
    wait(&mut harness, "the tag dialog", |h| h.query_by_role_and_label(Role::TextInput, "Tag name").is_some());
    type_into(&mut harness, "Tag name", "v1.2.0");
    harness.get_by_label("Push to remote").click();
    settle(&mut harness);
    save(&mut harness, folder, "new-tag-push-to-dark");
}

fn force_push_shots(folder: &Path) {
    let repo = repository();
    let path = repo.path();
    let _remote = publish_to_bare(path);
    commit_file(path, "more.txt", "more\n", "Add a file to push");
    git(path, &["push", "-q", "origin", "main"]);
    git(path, &["commit", "-q", "--amend", "-m", "Add a file, reworded"]);
    let mut harness = open_rendering_at(path, Appearance::Dark, egui::vec2(1440.0, 900.0));
    loaded(&mut harness);
    harness.get_all_by_label("main").next().expect("the main branch row").click_secondary();
    settle(&mut harness);
    save(&mut harness, folder, "branch-menu-force-push-dark");
    harness.get_by_label_contains("Force push").click();
    wait(&mut harness, "the force push confirmation", |h| h.state().dialog.is_some());
    settle(&mut harness);
    save(&mut harness, folder, "force-push-confirmation-dark");
}

fn settings_shots(folder: &Path) {
    let repo = repository();
    let mut harness = open_rendering_at(repo.path(), Appearance::Dark, egui::vec2(1440.0, 900.0));
    loaded(&mut harness);
    harness.state_mut().show_settings = true;
    settle(&mut harness);
    type_into(&mut harness, "GitHub Enterprise hosts", "git.example.com");
    harness.key_press(egui::Key::Enter);
    type_into(&mut harness, "GitHub Enterprise hosts", "not a host");
    settle(&mut harness);
    save(&mut harness, folder, "settings-enterprise-dark");
}

// MARK: Design review of the second batch of new interface pieces

#[test]
#[ignore = "renders design-review screenshots; run on request"]
fn review_screenshots_2() {
    let Some(folder) = review_folder() else {
        eprintln!("Set NICEGIT_SCREENSHOTS to a folder to render screenshots.");
        return;
    };
    pull_request_shots(&folder);
    go_to_file_shots(&folder);
    clone_shots(&folder);
    conflict_shots(&folder);
    save_stash_shots(&folder);
    squash_shots(&folder);
    line_history_shots(&folder);
    undo_history_shots(&folder);
    settings_scroll_shots(&folder);
}

/// Runs frames for up to `seconds` while `done` is false. For windows that may never finish
/// loading here, such as anything waiting on the GitHub CLI, so the render captures what shows.
fn run_until(harness: &mut App, seconds: u64, done: impl Fn(&App) -> bool) {
    let start = std::time::Instant::now();
    while !done(harness) && start.elapsed() < std::time::Duration::from_secs(seconds) {
        harness.step();
        std::thread::sleep(std::time::Duration::from_millis(15));
    }
    harness.run_steps(3);
}

fn pull_request_shots(folder: &Path) {
    for (width, height) in [(1440.0, 900.0), (1100.0, 800.0)] {
        let repo = repository();
        let path = repo.path();
        git(path, &["remote", "add", "origin", "https://github.com/owner/repo.git"]);
        git(path, &["update-ref", "refs/remotes/origin/feature", "HEAD"]);
        git(path, &["config", "branch.feature.remote", "origin"]);
        git(path, &["config", "branch.feature.merge", "refs/heads/feature"]);
        let mut harness = open_rendering_at(path, Appearance::Dark, egui::vec2(width, height));
        loaded(&mut harness);
        harness.get_by_label("feature").click_secondary();
        settle(&mut harness);
        harness.get_by_label_contains("Create pull request on GitHub").click();
        wait(&mut harness, "the pull request dialog", |h| h.state().pull_request.is_some());
        run_until(&mut harness, 5, |h| h.state().pull_request.as_ref().is_some_and(|d| d.settled()));
        save(&mut harness, folder, &format!("pull-request-dialog-{width}-dark"));
    }
}

fn go_to_file_shots(folder: &Path) {
    let repo = repository();
    let path = repo.path();
    std::fs::create_dir_all(path.join("src")).unwrap();
    std::fs::create_dir_all(path.join("docs")).unwrap();
    commit_file(path, "src/guide.md", "# Guide\n", "Add a guide");
    commit_file(path, "src/guide_tests.rs", "// tests\n", "Add guide tests");
    commit_file(path, "docs/guide-notes.txt", "notes\n", "Add guide notes");
    commit_file(path, "README.md", "# Readme\n", "Add a readme");
    let mut harness = open_rendering_at(path, Appearance::Dark, egui::vec2(1440.0, 900.0));
    loaded(&mut harness);
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::P);
    wait(&mut harness, "the file finder", |h| h.state().go_to_file.is_some());
    harness.event(egui::Event::Text("guide".into()));
    wait(&mut harness, "the file listing", |h| shows_text(h, "guide.md"));
    settle(&mut harness);
    save(&mut harness, folder, "go-to-file-dark");
}

fn clone_shots(folder: &Path) {
    let source = repository();
    let parent = temp_dir();
    let mut harness = open_rendering_at(source.path(), Appearance::Dark, egui::vec2(1440.0, 900.0));
    loaded(&mut harness);
    harness.state_mut().settings.clone_folder = Some(parent.path().join("Projects"));
    repository_menu(&mut harness, &folder_name(source.path()), "Clone repository");
    wait(&mut harness, "the clone dialog", |h| h.state().dialog.is_some());
    type_into(&mut harness, "Repository URL or local path", "https://github.com/owner/widgets.git");
    settle(&mut harness);
    save(&mut harness, folder, "clone-dialog-default-folder-dark");
    // The GitHub picker is left out: it lists the repositories of whoever's gh login is on the
    // machine, private ones included, which do not belong in a design screenshot.
}

/// Stops a merge of `feature` into main on a conflict in two files, from the feature's menu.
fn stop_a_merge_on_two_conflicts(harness: &mut App) {
    harness.get_by_label("feature").click_secondary();
    settle(harness);
    harness.get_by_label_contains("Merge into main").click();
    wait(harness, "the merge confirmation", |h| h.state().dialog.is_some());
    harness.get_by_label("Merge").click();
    idle(harness);
    wait(harness, "the operation banner", |h| h.query_by_label_contains("  notes.txt").is_some());
    wait(harness, "both conflicted files in the banner", |h| h.query_by_label_contains("  extra.txt").is_some());
    settle(harness);
}

fn conflict_shots(folder: &Path) {
    let repo = repository();
    let path = repo.path();
    git(path, &["switch", "-q", "feature"]);
    commit_file(path, "notes.txt", "first\nfrom feature\n", "Change notes on feature");
    commit_file(path, "extra.txt", "from feature\n", "Add extra on feature");
    git(path, &["switch", "-q", "main"]);
    commit_file(path, "extra.txt", "from main\n", "Add extra on main");
    let mut harness = open_rendering_at(path, Appearance::Dark, egui::vec2(1440.0, 900.0));
    loaded(&mut harness);
    stop_a_merge_on_two_conflicts(&mut harness);
    save(&mut harness, folder, "conflict-operation-bar-dark");

    harness.get_by_label_contains("Resolve all").click();
    settle(&mut harness);
    save(&mut harness, folder, "conflict-resolve-all-menu-dark");
    harness.key_press(egui::Key::Escape);
    settle(&mut harness);

    harness.get_by_label("notes.txt").click_secondary();
    settle(&mut harness);
    save(&mut harness, folder, "conflict-file-menu-dark");
    harness.key_press(egui::Key::Escape);
    settle(&mut harness);
}

fn save_stash_shots(folder: &Path) {
    let repo = repository();
    let path = repo.path();
    std::fs::write(path.join("notes.txt"), "first\nsecond\nstaged\n").unwrap();
    git(path, &["add", "--", "notes.txt"]);
    std::fs::write(path.join("fresh.txt"), "fresh\n").unwrap();
    let mut harness = open_rendering_at(path, Appearance::Dark, egui::vec2(1440.0, 900.0));
    loaded(&mut harness);
    toolbar_button(&mut harness, "Stash");
    wait(&mut harness, "the stash dialog", |h| h.state().dialog.is_some());
    settle(&mut harness);
    save(&mut harness, folder, "save-stash-dialog-dark");
}

fn squash_shots(folder: &Path) {
    let repo = repository();
    let path = repo.path();
    commit_file(path, "one.txt", "one\n", "Squash one");
    commit_file(path, "two.txt", "two\n", "Squash two");
    commit_file(path, "three.txt", "three\n", "Squash three");
    let mut harness = open_rendering_at(path, Appearance::Dark, egui::vec2(1440.0, 900.0));
    loaded(&mut harness);
    harness.get_by_label("Squash one").click_modifiers(egui::Modifiers::COMMAND);
    harness.step();
    harness.get_by_label("Squash three").click_modifiers(egui::Modifiers::COMMAND);
    harness.step();
    wait(&mut harness, "the marked commits bar", |h| shows_text(h, "2 commits selected"));
    settle(&mut harness);
    save(&mut harness, folder, "marked-commits-bar-dark");
    harness.get_by_label("Squash…").click();
    wait(&mut harness, "the squash window", |h| h.state().tools.iter().any(|t| t.title() == "Squash commits"));
    wait(&mut harness, "the squash plan", |h| shows_text(h, "Rewrite 3 commits on main"));
    settle(&mut harness);
    save(&mut harness, folder, "squash-window-dark");
}

fn line_history_shots(folder: &Path) {
    let repo = repository();
    let path = repo.path();
    commit_file(path, "notes.txt", "first\nsecond\nthird\n", "Add a third line");
    commit_file(path, "notes.txt", "first\nSECOND\nTHIRD\n", "Capitalise the last two lines");
    let head = git(path, &["rev-parse", "HEAD"]);
    let mut harness = open_rendering_at(path, Appearance::Dark, egui::vec2(1440.0, 900.0));
    loaded(&mut harness);
    harness.get_by_label("Capitalise the last two lines").click();
    wait_for_commit_file(&mut harness, "notes.txt");
    harness.get_by_label("notes.txt").click_secondary();
    settle(&mut harness);
    harness.get_by_label_contains("Blame at this commit").click();
    let second_line = format!("{} · Test · SECOND", &head[..7]);
    wait(&mut harness, "the blame lines", |h| shows_text(h, &second_line));
    settle(&mut harness);
    harness.get_by_label(&second_line).click_secondary();
    settle(&mut harness);
    harness.get_by_label_contains("History of lines 2–3").click();
    wait(&mut harness, "the line history window", |h| h.state().tools.iter().any(|t| t.title() == "History of notes.txt lines 2–3"));
    wait(&mut harness, "the commits that changed the lines", |h| shows_text(h, "Add a third line · Test"));
    settle(&mut harness);
    save(&mut harness, folder, "line-history-window-dark");
}

fn undo_history_shots(folder: &Path) {
    let repo = repository();
    let path = repo.path();
    let mut harness = open_rendering_at(path, Appearance::Dark, egui::vec2(1440.0, 900.0));
    loaded(&mut harness);
    for (text, summary) in [("first\nsecond\nthird\n", "Add a third line"), ("first\nsecond\nthird\nfourth\n", "Add a fourth line")] {
        std::fs::write(path.join("notes.txt"), text).unwrap();
        harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::R);
        wait(&mut harness, "the changed file to be listed", |h| h.state().snapshot().is_some_and(|s| !s.status.is_empty()));
        idle(&mut harness);
        harness.get_by_label("Stage All Changes").click();
        idle(&mut harness);
        type_into(&mut harness, "Commit summary", summary);
        wait(&mut harness, "the commit button", |h| shows_text(h, "Commit 1 file to main"));
        harness.get_by_label_contains("Commit 1 file to main").click();
        idle(&mut harness);
    }
    settle(&mut harness);
    harness.get_by_label("Undo history options").click();
    settle(&mut harness);
    save(&mut harness, folder, "undo-history-menu-dark");
}

fn settings_scroll_shots(folder: &Path) {
    let repo = repository();
    let mut harness = open_rendering_at(repo.path(), Appearance::Dark, egui::vec2(1440.0, 900.0));
    loaded(&mut harness);
    harness.state_mut().show_settings = true;
    settle(&mut harness);
    let clone_into = harness.query_all(By::new().role(Role::Label).value("Clone into")).next().expect("Clone into").rect().center();
    harness.hover_at(clone_into);
    harness.event(egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: egui::vec2(0.0, -260.0),
        phase: egui::TouchPhase::Move,
        modifiers: egui::Modifiers::NONE,
    });
    settle(&mut harness);
    save(&mut harness, folder, "settings-clone-into-dark");
}
