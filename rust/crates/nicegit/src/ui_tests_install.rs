//! Interface tests for installing an update from the notice. Nothing is downloaded: the install
//! method is injected, and the release files come from stand-ins that record what was asked for.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use egui_kittest::kittest::Queryable;
use nicegit_core::install::{asset_name, release_url, InstallMethod, CHECKSUM_MISMATCH};

use crate::install::{Source, METHOD_OVERRIDE, SOURCE_OVERRIDE};
use crate::ui::dialogs::Dialog;
use crate::ui_tests::*;

type Log = Arc<Mutex<Vec<(String, PathBuf)>>>;

fn inject_method(method: InstallMethod) {
    METHOD_OVERRIDE.with(|slot| *slot.borrow_mut() = Some(method));
}

/// A stand-in for the release: its checksum list names the file with a hash that no download has,
/// and its download writes some other bytes, so the check must refuse it.
fn mismatching_release(log: Log) -> Source {
    Source {
        checksums: Arc::new(|version: &str| -> Result<String, String> {
            // Every platform's file is listed, so the check reaches the mismatch whichever one is installed.
            let names = [("macos", "aarch64"), ("windows", "x86_64"), ("linux", "x86_64"), ("linux", "aarch64")];
            let lines: Vec<String> = names
                .iter()
                .filter_map(|(os, arch)| asset_name(version, os, arch))
                .map(|name| format!("{}  {name}", "0".repeat(64)))
                .collect();
            Ok(lines.join("\n") + "\n")
        }),
        download: Arc::new(move |url: &str, dest: &Path| {
            log.lock().unwrap().push((url.to_string(), dest.to_path_buf()));
            std::fs::write(dest, b"not the release").map_err(|error| error.to_string())
        }),
    }
}

fn install_source(log: Log) {
    SOURCE_OVERRIDE.with(|slot| *slot.borrow_mut() = Some(mismatching_release(log)));
}

fn new_log() -> Log {
    Arc::new(Mutex::new(Vec::new()))
}

fn show_update(harness: &mut egui_kittest::Harness<'static, crate::app::NiceGitApp>) {
    harness.state_mut().receive_latest_release("v2.1.0");
    wait(harness, "the update notice", |h| h.query_by_label("NiceGit 2.1.0 is available").is_some());
}

fn confirmation_open(harness: &egui_kittest::Harness<'static, crate::app::NiceGitApp>) -> bool {
    matches!(&harness.state().dialog, Some(Dialog::Confirm { title, .. }) if title == "Install NiceGit 2.1.0?")
}

#[test]
fn a_manual_copy_keeps_only_the_download_link() {
    let repo = repository();
    let mut harness = open(repo.path());
    loaded(&mut harness);
    show_update(&mut harness);
    assert!(harness.query_by_label("Download").is_some(), "the download link is always there");
    assert!(harness.query_by_label("Download and install").is_none(), "a copy that cannot replace itself offers no install");
}

#[test]
fn a_mac_app_offers_install_and_asks_before_anything_is_downloaded() {
    let repo = repository();
    let dir = tempfile::tempdir().unwrap();
    inject_method(InstallMethod::MacBundle(dir.path().join("NiceGit.app")));
    let log = new_log();
    install_source(log.clone());
    let mut harness = open(repo.path());
    loaded(&mut harness);
    show_update(&mut harness);

    assert!(harness.query_by_label("Download").is_some());
    harness.get_by_label("Download and install").click();
    wait(&mut harness, "the install confirmation", confirmation_open);
    match &harness.state().dialog {
        Some(Dialog::Confirm { message, button, .. }) => {
            assert_eq!(message, "NiceGit will restart.");
            assert_eq!(button, "Install");
        }
        _ => panic!("the confirmation is open"),
    }
    assert!(harness.state().install.is_none(), "nothing starts before the confirmation");
    assert!(log.lock().unwrap().is_empty(), "nothing is downloaded before the confirmation");
}

#[test]
fn an_install_whose_download_fails_its_checksum_is_refused_and_leaves_nothing_behind() {
    let repo = repository();
    let dir = tempfile::tempdir().unwrap();
    let bundle = dir.path().join("NiceGit.app");
    inject_method(InstallMethod::MacBundle(bundle.clone()));
    let log = new_log();
    install_source(log.clone());
    let mut harness = open(repo.path());
    loaded(&mut harness);
    show_update(&mut harness);

    harness.get_by_label("Download and install").click();
    wait(&mut harness, "the install confirmation", confirmation_open);
    harness.get_by_label("Install").click();
    wait(&mut harness, "the checksum refusal", |h| {
        h.state().install.is_none() && h.state().notice.as_ref().is_some_and(|notice| notice.text == CHECKSUM_MISMATCH)
    });

    let name = asset_name("2.1.0", "macos", "aarch64").unwrap();
    let downloads = log.lock().unwrap().clone();
    assert_eq!(downloads.len(), 1, "one release file was asked for");
    assert_eq!(downloads[0].0, release_url("2.1.0", &name));
    assert!(!downloads[0].1.exists(), "the file that failed the check is deleted");
    assert!(!bundle.exists(), "the app was never touched");
    assert!(harness.state().installed_update.is_none());
    assert!(harness.state().notice.as_ref().is_some_and(|notice| notice.is_error));
}

#[test]
fn an_appimage_update_that_fails_its_checksum_leaves_the_running_file_alone() {
    let repo = repository();
    let dir = tempfile::tempdir().unwrap();
    let appimage = dir.path().join("NiceGit.AppImage");
    std::fs::write(&appimage, b"the current build").unwrap();
    inject_method(InstallMethod::AppImage(appimage.clone()));
    let log = new_log();
    install_source(log.clone());
    let mut harness = open(repo.path());
    loaded(&mut harness);
    show_update(&mut harness);

    harness.get_by_label("Download and install").click();
    wait(&mut harness, "the install confirmation", confirmation_open);
    harness.get_by_label("Install").click();
    wait(&mut harness, "the checksum refusal", |h| {
        h.state().install.is_none() && h.state().notice.as_ref().is_some_and(|notice| notice.text == CHECKSUM_MISMATCH)
    });

    assert_eq!(std::fs::read(&appimage).unwrap(), b"the current build");
    let downloads = log.lock().unwrap().clone();
    assert!(downloads[0].1.file_name().unwrap().to_string_lossy().ends_with("AppImage.new"), "the copy is made beside it");
    assert!(!downloads[0].1.exists());
}

#[test]
fn an_install_does_not_start_while_an_action_is_busy() {
    let repo = repository();
    let dir = tempfile::tempdir().unwrap();
    inject_method(InstallMethod::MacBundle(dir.path().join("NiceGit.app")));
    let log = new_log();
    install_source(log.clone());
    let mut harness = open(repo.path());
    loaded(&mut harness);
    show_update(&mut harness);

    harness.state_mut().busy = Some("Fetch".into());
    harness.state_mut().start_install("2.1.0".into());
    settle(&mut harness);
    assert!(harness.state().install.is_none());
    assert!(log.lock().unwrap().is_empty(), "nothing is downloaded while busy");
    assert!(harness.state().notice.as_ref().is_some_and(|notice| notice.text.contains("Wait for the current action")));
}
