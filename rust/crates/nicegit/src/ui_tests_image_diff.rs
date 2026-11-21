//! Interface tests for comparing a changed image in each of its modes, in the Changes panel and in
//! the compare window. They click through the window, then check the setting it remembers. They
//! use the helpers in `ui_tests`.

use std::path::Path;

use egui::accesskit::Role;
use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;

use crate::app::NiceGitApp;
use crate::tools::image_diff::ImageDiffMode;
use crate::ui_tests::*;

type App = Harness<'static, NiceGitApp>;

/// Whether any widget's label contains `text`.
fn shows(harness: &App, text: &str) -> bool {
    harness.query_all_by_label_contains(text).next().is_some()
}

/// Commits an 8 by 6 picture as `picture.png`, then changes six of its top-row pixels in the
/// working files. Six of the 48 pixels change, so the difference reads 12.5 %.
fn committed_picture(path: &Path) {
    let picture = image::RgbaImage::from_fn(8, 6, |x, y| image::Rgba([(x * 30) as u8, (y * 40) as u8, 100, 255]));
    picture.save(path.join("picture.png")).unwrap();
    git(path, &["add", "picture.png"]);
    git(path, &["commit", "-q", "-m", "Add the picture"]);
    let mut changed = picture;
    for x in 0..6 {
        changed.put_pixel(x, 0, image::Rgba([255, 255, 255, 255]));
    }
    changed.save(path.join("picture.png")).unwrap();
}

/// Clicks the mode button with `label` and lets the window settle.
fn pick_mode(harness: &mut App, label: &str) {
    harness.get_by_role_and_label(Role::Button, label).click();
    settle(harness);
}

#[test]
fn a_changed_image_compares_in_each_mode_in_the_changes_panel() {
    let repo = repository();
    let path = repo.path();
    committed_picture(path);

    let mut harness = open(path);
    loaded(&mut harness);
    // A changes row is named for its file, kind, and place, such as "picture.png, modified, unstaged".
    wait(&mut harness, "the changed picture", |h| shows(h, "picture.png, "));
    settle(&mut harness);
    harness.get_by_label_contains("picture.png, ").click();
    wait(&mut harness, "the mode buttons", |h| h.query_by_role_and_label(Role::Button, "Swipe").is_some());
    settle(&mut harness);

    pick_mode(&mut harness, "Swipe");
    wait(&mut harness, "the swipe divider", |h| h.query_by_role_and_label(Role::Slider, "Swipe divider").is_some());

    pick_mode(&mut harness, "Onion skin");
    wait(&mut harness, "the opacity slider", |h| h.query_by_role_and_label(Role::Slider, "Opacity").is_some());

    pick_mode(&mut harness, "Difference");
    wait(&mut harness, "the share of changed pixels", |h| shows(h, "12.5 % of pixels changed"));
    assert_eq!(harness.state().settings.image_diff_mode, ImageDiffMode::Difference, "the chosen mode is remembered");
}

#[test]
fn the_compare_window_offers_the_same_modes_for_a_changed_image() {
    let repo = repository();
    let path = repo.path();
    committed_picture(path);

    let mut harness = open(path);
    loaded(&mut harness);
    harness.get_by_label("Add the picture").click_secondary();
    settle(&mut harness);
    harness.get_by_label_contains("Compare with working files").click();
    wait(&mut harness, "the changed picture", |h| h.query_by_role_and_label(Role::Button, "picture.png").is_some());
    settle(&mut harness);
    harness.get_by_role_and_label(Role::Button, "picture.png").click();
    wait(&mut harness, "the mode buttons", |h| h.query_by_role_and_label(Role::Button, "Swipe").is_some());

    pick_mode(&mut harness, "Onion skin");
    wait(&mut harness, "the opacity slider", |h| h.query_by_role_and_label(Role::Slider, "Opacity").is_some());

    pick_mode(&mut harness, "Difference");
    wait(&mut harness, "the share of changed pixels", |h| shows(h, "12.5 % of pixels changed"));
}
