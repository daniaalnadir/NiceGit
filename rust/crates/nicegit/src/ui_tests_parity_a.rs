//! Interface tests for the remaining details of Mac app features, built on the helpers in
//! `ui_tests`.

#![allow(unused_imports)]

use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

use egui::accesskit::Role;
use egui::{Key, Modifiers};
use egui_kittest::kittest::{By, NodeT, Queryable};
use egui_kittest::Harness;
use egui_phosphor::regular as icon;

use crate::app::NiceGitApp;
use crate::ui_tests::*;

type App = Harness<'static, NiceGitApp>;

/// Clicks at a point in the window, as a pointer press and release.
fn click_at(harness: &mut App, pos: egui::Pos2) {
    harness.hover_at(pos);
    harness.step();
    for pressed in [true, false] {
        harness.event(egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
    }
    harness.step();
}

/// Opens a collapsed sidebar header by its disclosure triangle, which sits just left of its label.
fn open_header(harness: &mut App, label: &str) {
    let rect = harness.get_by_label_contains(label).rect();
    click_at(harness, egui::pos2(rect.min.x - 8.0, rect.center().y));
    settle(harness);
}

/// The commit the selection points at, if a commit is selected.
fn selected_commit(harness: &App) -> Option<String> {
    match harness.state().repo().map(|r| r.selection.clone()) {
        Some(crate::app::Selection::Commit { hash, .. }) => Some(hash),
        _ => None,
    }
}

/// Replaces the text in the sidebar's filter field.
fn set_filter(harness: &mut App, text: &str) {
    harness.get_by_role_and_label(Role::TextInput, "Filter references").focus();
    harness.step();
    harness.key_press_modifiers(Modifiers::COMMAND, Key::A);
    harness.step();
    harness.get_by_role_and_label(Role::TextInput, "Filter references").type_text(text);
    harness.run_steps(3);
}

#[test]
fn sidebar_filter_narrows_tags_and_remote_branches() {
    let repo = repository();
    let path = repo.path();
    let remote = tempfile::tempdir().unwrap();
    git(remote.path(), &["init", "-q", "--bare"]);
    git(path, &["remote", "add", "origin", &remote.path().to_string_lossy()]);
    git(path, &["branch", "release/1.0"]);
    git(path, &["tag", "v1.0", "HEAD~1"]);
    git(path, &["tag", "release-candidate"]);
    // A branch that exists only on the remote: pushed, its local copy deleted, then fetched.
    git(path, &["branch", "review/remote-only"]);
    git(path, &["push", "-q", "origin", "review/remote-only"]);
    git(path, &["branch", "-D", "review/remote-only"]);
    git(path, &["fetch", "-q", "origin"]);
    let mut harness = open(path);
    loaded(&mut harness);

    // Tags and the remote's branches start collapsed, so open both sections.
    harness.get_by_label_contains("TAGS").click();
    settle(&mut harness);
    open_header(&mut harness, "  origin");
    assert!(harness.query_by_label("review/remote-only").is_some(), "the remote branch is listed once its remote is open");
    assert!(harness.query_by_label("v1.0").is_some() && harness.query_by_label("release-candidate").is_some());

    // A filter that matches only the remote branch hides the local branches and tags too.
    set_filter(&mut harness, "remote-only");
    assert!(harness.query_by_label("review/remote-only").is_some(), "the matching remote branch stays");
    assert!(harness.query_by_label("feature").is_none() && harness.query_by_label("release/1.0").is_none());
    assert!(harness.query_by_label("v1.0").is_none() && harness.query_by_label("release-candidate").is_none());

    // A filter that matches a tag and a local branch narrows the tags, and hides the remote branch.
    set_filter(&mut harness, "release");
    assert!(harness.query_by_label("release-candidate").is_some(), "the matching tag stays");
    assert!(harness.query_by_label("v1.0").is_none(), "other tags are hidden");
    assert!(harness.query_by_label("release/1.0").is_some(), "the matching local branch stays");
    assert!(harness.query_by_label("review/remote-only").is_none(), "other remote branches are hidden");
}

/// Typing a filter reveals matching remote branches even while their remote is collapsed.
#[test]
fn filter_reveals_remote_branches_under_a_collapsed_remote() {
    let repo = repository();
    let path = repo.path();
    let remote = tempfile::tempdir().unwrap();
    git(remote.path(), &["init", "-q", "--bare"]);
    git(path, &["remote", "add", "origin", &remote.path().to_string_lossy()]);
    git(path, &["branch", "review/remote-only"]);
    git(path, &["push", "-q", "origin", "review/remote-only"]);
    git(path, &["branch", "-D", "review/remote-only"]);
    git(path, &["fetch", "-q", "origin"]);
    let mut harness = open(path);
    loaded(&mut harness);
    settle(&mut harness);

    set_filter(&mut harness, "remote-only");
    assert!(harness.query_by_label("review/remote-only").is_some(), "the matching remote branch is shown");
}

#[test]
fn settings_choose_a_graph_palette_and_follow_the_system_theme() {
    use crate::theme::{Appearance, GraphPalette};
    let repo = repository();
    let mut harness = open(repo.path());
    loaded(&mut harness);

    harness.key_press_modifiers(Modifiers::COMMAND, Key::Comma);
    wait(&mut harness, "the Settings window", |h| h.query_by_role_and_label(Role::RadioButton, "Muted").is_some());
    settle(&mut harness);
    assert_eq!(harness.state().settings.graph_palette, GraphPalette::Standard, "the standard palette is the default");

    for (label, palette) in
        [("Colour-blind safe", GraphPalette::ColorBlindSafe), ("Muted", GraphPalette::Muted), ("Standard", GraphPalette::Standard)]
    {
        harness.get_by_role_and_label(Role::RadioButton, label).click();
        harness.run_steps(3);
        assert_eq!(harness.state().settings.graph_palette, palette, "choosing {label}");
    }
    harness.get_by_role_and_label(Role::RadioButton, "Muted").click();
    harness.run_steps(3);
    assert_eq!(harness.state().settings.graph_palette, GraphPalette::Muted);

    // System follows the operating system's appearance as it changes.
    let set_os = |harness: &mut App, theme: egui::Theme| {
        harness.input_mut().system_theme = Some(theme);
        harness.run_steps(3);
    };
    assert_eq!(harness.state().settings.appearance, Appearance::System);
    set_os(&mut harness, egui::Theme::Light);
    assert_eq!(harness.ctx.theme(), egui::Theme::Light, "System shows the light system appearance");
    set_os(&mut harness, egui::Theme::Dark);
    assert_eq!(harness.ctx.theme(), egui::Theme::Dark, "System shows the dark system appearance");

    // An explicit choice ignores the system, and System takes over again.
    harness.get_by_label("Light").click();
    harness.run_steps(3);
    assert_eq!(harness.ctx.theme(), egui::Theme::Light, "Light is used while the system is dark");
    harness.get_by_label("System").click();
    harness.run_steps(3);
    assert_eq!(harness.ctx.theme(), egui::Theme::Dark, "System returns to the dark system appearance");
    set_os(&mut harness, egui::Theme::Light);
    assert_eq!(harness.ctx.theme(), egui::Theme::Light, "System follows the system again");
}

#[test]
fn double_clicking_the_repositories_edge_restores_its_width() {
    use egui::containers::panel::PanelState;
    let repo = repository();
    let mut harness = open(repo.path());
    loaded(&mut harness);
    settle(&mut harness);
    let id = egui::Id::new("repositories");
    let width = |h: &App| PanelState::load(&h.ctx, id).map(|s| s.size().x).unwrap_or_default();
    let default = settled_width(&mut harness, id);

    // Drag the repositories column's edge wider.
    let edge = PanelState::load(&harness.ctx, id).expect("repositories shown").outer_rect.right_center();
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
        harness.event(egui::Event::PointerMoved(edge + egui::vec2(6.0 * step as f32, 0.0)));
        harness.step();
    }
    harness.event(pressed(false, edge + egui::vec2(60.0, 0.0)));
    settle(&mut harness);
    let dragged = width(&harness);
    assert!(dragged > default + 30.0, "the drag widened the column: {default} -> {dragged}");

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
    let restored = settled_width(&mut harness, id);
    // The first width can be wider than the default when a long folder path stretches the
    // column's contents (as Windows temporary paths do), so the restored default is at most it.
    assert!(restored < dragged - 30.0, "the double-click undid the drag: {dragged} -> {restored}");
    assert!(restored <= default + 1.0, "restored {restored}, no wider than the starting {default}");
}

/// The width a side panel settles at. Its contents set a minimum width that can change over the
/// first few frames, so this runs frames until the width has held still.
fn settled_width(harness: &mut App, id: egui::Id) -> f32 {
    let width = |h: &App| egui::containers::panel::PanelState::load(&h.ctx, id).map(|s| s.size().x).unwrap_or_default();
    let (mut last, mut still) = (width(harness), 0);
    for _ in 0..200 {
        harness.step();
        let now = width(harness);
        still = if now == last { still + 1 } else { 0 };
        last = now;
        if still >= 5 {
            return now;
        }
    }
    panic!("the panel's width kept changing: {last}");
}

/// The file size, formatted as the compare window shows it.
fn expected_size(bytes: u64) -> String {
    if bytes < 1000 {
        format!("{bytes} bytes")
    } else {
        format!("{:.1} KB", bytes as f64 / 1000.0)
    }
}

/// Commits a small image, then changes it in the working files and compares the two versions.
/// Returns the sizes in bytes of the committed and the working version.
fn changed_icon(path: &Path, committed: &image::RgbaImage, changed: &image::RgbaImage) -> (u64, u64) {
    committed.save(path.join("icon.png")).unwrap();
    git(path, &["add", "icon.png"]);
    git(path, &["commit", "-q", "-m", "Add the icon"]);
    changed.save(path.join("icon.png")).unwrap();
    let before = git(path, &["cat-file", "-s", "HEAD:icon.png"]).parse::<u64>().unwrap();
    let after = std::fs::metadata(path.join("icon.png")).unwrap().len();
    (before, after)
}

/// Pixels from a seeded generator, so the PNG cannot shrink much.
fn noise(width: u32, height: u32) -> image::RgbaImage {
    let mut seed: u32 = 7;
    image::RgbaImage::from_fn(width, height, |_, _| {
        let mut pixel = [0u8; 4];
        for channel in &mut pixel {
            seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12_345);
            *channel = (seed >> 16) as u8;
        }
        image::Rgba(pixel)
    })
}

/// Opens the compare window on the commit that added `icon.png` and selects the image.
fn compare_icon(harness: &mut App) {
    harness.get_by_label("Add the icon").click_secondary();
    settle(harness);
    harness.get_by_label_contains("Compare with working files").click();
    wait(harness, "the changed image", |h| h.query_by_role_and_label(Role::Button, "icon.png").is_some());
    settle(harness);
    harness.get_by_role_and_label(Role::Button, "icon.png").click();
}

#[test]
fn compare_shows_the_file_size_of_each_version_of_an_image() {
    let repo = repository();
    let path = repo.path();
    let (before, after) = changed_icon(path, &image::RgbaImage::from_pixel(4, 3, image::Rgba([40, 120, 200, 255])), &noise(16, 10));
    assert!(before < 1000 && after < 1000, "both sizes are in bytes: {before} and {after}");

    let mut harness = open(path);
    loaded(&mut harness);
    compare_icon(&mut harness);
    let before_label = format!("4 × 3 · {}", expected_size(before));
    let after_label = format!("16 × 10 · {}", expected_size(after));
    wait(&mut harness, "both sizes", |h| h.query_by_label(&before_label).is_some() && h.query_by_label(&after_label).is_some());
}

/// A file of a few kilobytes is shown in kilobytes, not bytes.
#[test]
fn compare_shows_large_image_sizes_in_kilobytes() {
    let repo = repository();
    let path = repo.path();
    let (before, after) = changed_icon(path, &image::RgbaImage::from_pixel(4, 3, image::Rgba([40, 120, 200, 255])), &noise(120, 80));
    assert!(after >= 1000, "the changed image is over a kilobyte: {after}");

    let mut harness = open(path);
    loaded(&mut harness);
    compare_icon(&mut harness);
    let before_label = format!("4 × 3 · {}", expected_size(before));
    let after_label = format!("120 × 80 · {}", expected_size(after));
    wait(&mut harness, "both sizes", |h| h.query_by_label(&before_label).is_some() && h.query_by_label(&after_label).is_some());
}

#[test]
fn palette_switches_to_a_branch_by_its_name() {
    let repo = repository();
    let path = repo.path();
    let mut harness = open(path);
    loaded(&mut harness);

    harness.key_press_modifiers(Modifiers::COMMAND | Modifiers::SHIFT, Key::P);
    wait(&mut harness, "the palette", |h| h.state().palette.is_some());
    harness.event(egui::Event::Text("feature".into()));
    wait(&mut harness, "the switch command", |h| h.query_by_label_contains("Switch to feature").is_some());
    harness.key_press(Key::Enter);
    idle(&mut harness);
    assert_eq!(git(path, &["branch", "--show-current"]), "feature");
    wait(&mut harness, "the new checkout", |h| h.state().snapshot().is_some_and(|s| s.current_branch == "feature"));
}

#[test]
fn palette_reopens_a_recent_repository_after_its_tab_closes() {
    let first = repository();
    let second = repository();
    let mut harness = open(first.path());
    loaded(&mut harness);
    harness.state_mut().open(second.path().to_path_buf());
    loaded(&mut harness);
    let first_name = folder_name(first.path());
    harness.get_by_label(&format!("Close {first_name}")).click();
    settle(&mut harness);
    assert_eq!(harness.state().repos.len(), 1);

    harness.key_press_modifiers(Modifiers::COMMAND | Modifiers::SHIFT, Key::P);
    wait(&mut harness, "the palette", |h| h.state().palette.is_some());
    harness.event(egui::Event::Text(format!("open {first_name}")));
    wait(&mut harness, "the reopen command", |h| h.query_by_label_contains(&format!("Open {first_name}")).is_some());
    harness.key_press(Key::Enter);
    loaded(&mut harness);
    assert_eq!(harness.state().repos.len(), 2, "the repository opens in a new tab");
    assert_eq!(harness.state().snapshot().map(|s| s.name.clone()), Some(first_name));
}

#[test]
fn palette_undoes_the_last_commit() {
    let repo = repository();
    let path = repo.path();
    std::fs::write(path.join("notes.txt"), "first\nsecond\nthird\n").unwrap();
    let mut harness = open(path);
    loaded(&mut harness);

    harness.get_by_label("Stage All Changes").click();
    idle(&mut harness);
    type_into(&mut harness, "Commit summary", "Add a third line");
    wait(&mut harness, "the commit button", |h| has_label(h, "Commit 1 file to main"));
    harness.get_by_label_contains("Commit 1 file to main").click();
    idle(&mut harness);
    assert_eq!(git(path, &["log", "-1", "--format=%s"]), "Add a third line");

    harness.key_press_modifiers(Modifiers::COMMAND | Modifiers::SHIFT, Key::P);
    wait(&mut harness, "the palette", |h| h.state().palette.is_some());
    harness.event(egui::Event::Text("undo".into()));
    wait(&mut harness, "the undo command", |h| h.query_by_label_contains("Undo commit").is_some());
    harness.key_press(Key::Enter);
    // Undo asks first, as it does from the toolbar.
    wait(&mut harness, "the undo confirmation", |h| h.state().dialog.is_some());
    assert_eq!(git(path, &["log", "-1", "--format=%s"]), "Add a third line", "nothing changes before confirming");
    harness.get_all_by_label("Undo").last().expect("the confirm button").click();
    idle(&mut harness);
    wait(&mut harness, "the commit to be undone", |_| git(path, &["log", "-1", "--format=%s"]) == "Add a second line");
    assert_eq!(git(path, &["diff", "--cached", "--name-only"]), "notes.txt", "the undone change stays staged");
}

#[test]
fn clicking_a_tag_selects_the_commit_it_points_to() {
    let repo = repository();
    let path = repo.path();
    git(path, &["tag", "-a", "v1.0", "-m", "First release", "HEAD~1"]);
    git(path, &["tag", "beta"]);
    let first = git(path, &["rev-parse", "HEAD~1"]);
    let head = git(path, &["rev-parse", "HEAD"]);
    let mut harness = open(path);
    loaded(&mut harness);

    harness.get_by_label_contains("TAGS").click();
    settle(&mut harness);
    harness.get_by_label("v1.0").click();
    wait(&mut harness, "the annotated tag's commit to be selected", |h| selected_commit(h) == Some(first.clone()));
    wait(&mut harness, "the inspector to show the tagged commit's subject", |h| inspector_shows(h, "Start the notes"));
    assert_eq!(selected_commit(&harness), Some(first.clone()), "the annotated tag selects the commit it peels to");

    harness.get_by_label("beta").click();
    wait(&mut harness, "the head tag's commit to be selected", |h| selected_commit(h) == Some(head.clone()));
    wait(&mut harness, "the inspector to show the head commit's subject", |h| inspector_shows(h, "Add a second line"));
    assert!(!inspector_shows(&harness, "Start the notes"), "the earlier commit's subject is no longer in the inspector");
}

/// Whether the commit inspector shows `subject`. The graph row for the selected commit always
/// shows its subject once; the inspector's heading is the second copy.
fn inspector_shows(harness: &App, subject: &str) -> bool {
    harness.query_all(By::new().label(subject)).count() >= 2
}
