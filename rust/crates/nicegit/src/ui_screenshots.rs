//! Renders the app offscreen to PNG files for design review, without needing a display. Run with
//! `NICEGIT_SCREENSHOTS=<folder> cargo test -p nicegit screenshots -- --ignored`, optionally with
//! `NICEGIT_SCREENSHOT_REPO=<repository>`; otherwise a small repository is made for the run.

use std::path::{Path, PathBuf};

use egui::accesskit::Role;
use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;
use egui_phosphor::regular as icon;

use crate::app::NiceGitApp;
use crate::theme::Appearance;
use crate::ui_tests::*;

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

        // A changed file's diff, unified and then side by side.
        harness.get_all_by_label(&file).next().expect("a changed file").click();
        wait(&mut harness, "the diff", |h| h.state().repo().is_some_and(|r| r.diff.is_some() && !r.diff_loading));
        save(&mut harness, &folder, &format!("diff-unified-{theme}"));
        harness.state_mut().settings.split_diff = true;
        save(&mut harness, &folder, &format!("diff-split-{theme}"));

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

        // Side by side needs a diff panel at least 560 points wide, which a 1440-point window
        // with both sidebars open does not leave; a wider window shows it in effect.
        let mut wide = open_rendering_at(&path, appearance, egui::vec2(1920.0, 1080.0));
        wide.state_mut().settings.split_diff = true;
        wide.get_all_by_label(&file).next().expect("a changed file").click();
        wait(&mut wide, "the diff", |h| h.state().repo().is_some_and(|r| r.diff.is_some() && !r.diff_loading));
        save(&mut wide, &folder, &format!("diff-split-wide-{theme}"));
    }
}
