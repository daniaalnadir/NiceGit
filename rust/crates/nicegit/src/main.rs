// Release builds on Windows open no console window alongside the app.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod diff_view;
mod file_dialog;
mod git_ext;
mod graph_view;
mod settings;
mod theme;
mod tools;
mod ui;
#[cfg(test)]
mod ui_screenshots;
#[cfg(test)]
mod ui_tests_parity_a;
#[cfg(test)]
mod ui_tests_parity_b;
#[cfg(test)]
mod ui_tests_parity_c;
#[cfg(test)]
mod ui_tests;
#[cfg(test)]
mod ui_tests_more;
#[cfg(test)]
mod ui_tests_tools;
mod worker;

use std::path::PathBuf;

fn icon() -> Option<egui::IconData> {
    let image = image::load_from_memory(include_bytes!("../../../assets/icon-256.png")).ok()?.into_rgba8();
    let (width, height) = image.dimensions();
    Some(egui::IconData { rgba: image.into_raw(), width, height })
}

fn main() -> eframe::Result {
    // A folder passed on the command line opens straight away, as `nicegit .` would.
    let initial = std::env::args_os().nth(1).map(PathBuf::from);
    let mut viewport = egui::ViewportBuilder::default()
        .with_title("NiceGit")
        .with_app_id("nicegit")
        .with_inner_size([1440.0, 900.0])
        .with_min_inner_size([900.0, 560.0]);
    if let Some(icon) = icon() {
        viewport = viewport.with_icon(icon);
    }
    #[allow(unused_mut)]
    let mut options = eframe::NativeOptions { viewport, ..Default::default() };
    // On Windows, draw with wgpu (Direct3D 12) unless NICEGIT_RENDERER=glow asks otherwise.
    #[cfg(windows)]
    if std::env::var("NICEGIT_RENDERER").as_deref() != Ok("glow") {
        options.renderer = eframe::Renderer::Wgpu;
    }
    eframe::run_native("NiceGit", options, Box::new(move |creation| Ok(Box::new(app::NiceGitApp::new(creation, initial)))))
}
