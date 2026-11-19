// Release builds on Windows open no console window alongside the app.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod claude;
mod conventional;
mod custom_actions;
mod diff_view;
mod file_dialog;
mod git_ext;
mod graph_view;
mod install;
mod issue_links;
mod notifications;
mod preload;
mod settings;
mod syntax;
mod theme;
mod timing;
mod tools;
mod ui;
#[cfg(test)]
mod ui_bench;
#[cfg(test)]
mod ui_bench_speed;
#[cfg(test)]
mod ui_screenshots;
#[cfg(test)]
mod ui_tests;
#[cfg(test)]
mod ui_tests_branch_checks;
#[cfg(test)]
mod ui_tests_ai_message;
#[cfg(test)]
mod ui_tests_clone;
#[cfg(test)]
mod ui_tests_conventional;
#[cfg(test)]
mod ui_tests_custom_actions;
#[cfg(test)]
mod ui_tests_drag_cherry_pick;
#[cfg(test)]
mod ui_tests_files;
#[cfg(test)]
mod ui_tests_find_repositories;
#[cfg(test)]
mod ui_tests_identity;
#[cfg(test)]
mod ui_tests_insights;
#[cfg(test)]
mod ui_tests_install;
#[cfg(test)]
mod ui_tests_issue_links;
#[cfg(test)]
mod ui_tests_more;
#[cfg(test)]
mod ui_tests_notifications;
#[cfg(test)]
mod ui_tests_parity_a;
#[cfg(test)]
mod ui_tests_parity_b;
#[cfg(test)]
mod ui_tests_parity_c;
#[cfg(test)]
mod ui_tests_parity_d;
#[cfg(test)]
mod ui_tests_parity_e;
#[cfg(test)]
mod ui_tests_parity_f;
#[cfg(test)]
mod ui_tests_perf;
#[cfg(test)]
mod ui_tests_range_diff;
#[cfg(test)]
mod ui_tests_resolutions;
#[cfg(test)]
mod ui_tests_speed;
#[cfg(test)]
mod ui_tests_split;
#[cfg(test)]
mod ui_tests_stash_lines;
#[cfg(test)]
mod ui_tests_tools;
#[cfg(test)]
mod ui_tests_updates;
#[cfg(test)]
mod ui_tests_welcome;
#[cfg(test)]
mod ui_tests_word_diff;
mod updates;
mod worker;

use std::path::PathBuf;

fn icon() -> Option<egui::IconData> {
    let image = image::load_from_memory(include_bytes!("../../../assets/icon-256.png")).ok()?.into_rgba8();
    let (width, height) = image.dimensions();
    Some(egui::IconData { rgba: image.into_raw(), width, height })
}

fn main() -> eframe::Result {
    timing::start();
    // A folder passed on the command line opens straight away, as `nicegit .` would.
    let initial = std::env::args_os().nth(1).map(PathBuf::from);
    // Read the first repository while the window and renderer are created.
    preload::start(initial.as_deref());
    // Folders dropped onto the window open as repositories.
    let mut viewport = egui::ViewportBuilder::default()
        .with_drag_and_drop(true)
        .with_title("NiceGit")
        .with_app_id("nicegit")
        .with_inner_size([1440.0, 900.0])
        .with_min_inner_size([900.0, 560.0]);
    if let Some(icon) = icon() {
        viewport = viewport.with_icon(icon);
    }
    #[allow(unused_mut)]
    let mut options = eframe::NativeOptions { viewport, ..Default::default() };
    // On Windows and macOS, draw with wgpu (Direct3D 12 or Metal) unless NICEGIT_RENDERER=glow
    // asks for OpenGL.
    #[cfg(any(windows, target_os = "macos"))]
    if std::env::var("NICEGIT_RENDERER").as_deref() != Ok("glow") {
        options.renderer = eframe::Renderer::Wgpu;
    }
    eframe::run_native("NiceGit", options, Box::new(move |creation| Ok(Box::new(app::NiceGitApp::new(creation, initial)))))
}
