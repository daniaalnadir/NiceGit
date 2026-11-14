//! Interface tests for the Conventional Commits help in the commit box and the summary length note.
//! They click and type through the real window, using the helpers in `ui_tests`.

use egui::accesskit::Role;
use egui_kittest::kittest::{NodeT, Queryable};
use egui_kittest::Harness;

use crate::app::NiceGitApp;
use crate::conventional::ConventionalCommits;
use crate::ui_tests::*;

/// The repository from `ui_tests::repository`, with six commits that follow the convention.
fn conventional_repository() -> tempfile::TempDir {
    let repo = repository();
    for subject in [
        "feat(parser): read ranges",
        "fix: keep the cursor",
        "docs: explain the layout",
        "chore: update the lockfile",
        "refactor(core): split the client",
        "test: cover the empty case",
    ] {
        git(repo.path(), &["commit", "-q", "--allow-empty", "-m", subject]);
    }
    repo
}

fn summary_text(harness: &Harness<'static, NiceGitApp>) -> String {
    harness.get_by_role_and_label(Role::TextInput, "Commit summary").accesskit_node().value().unwrap_or_default().to_string()
}

#[test]
fn the_type_menu_writes_the_type_scope_and_breaking_into_the_summary_and_commits_it() {
    let repo = conventional_repository();
    let path = repo.path();
    std::fs::write(path.join("notes.txt"), "first\nsecond\nthird\n").unwrap();
    let mut harness = open(path);
    loaded(&mut harness);

    wait(&mut harness, "the type menu", |h| has_label(h, "Commit type"));
    type_into(&mut harness, "Commit summary", "Keep the panel open");

    // The pointer hovers the menu button before the click, as a person's pointer does.
    harness.get_by_label("Commit type").hover();
    harness.run_steps(3);
    harness.get_by_label("Commit type").click();
    wait(&mut harness, "the type menu to open", |h| h.query_by_label("feat").is_some());
    harness.get_by_label("fix").click();
    settle(&mut harness);
    assert_eq!(summary_text(&harness), "fix: Keep the panel open", "choosing a type adds its prefix");

    type_into(&mut harness, "Commit scope", "ui");
    assert_eq!(summary_text(&harness), "fix(ui): Keep the panel open", "the scope goes into the prefix");

    harness.get_by_label("Breaking").click();
    settle(&mut harness);
    assert_eq!(summary_text(&harness), "fix(ui)!: Keep the panel open", "Breaking adds the bang before the colon");

    harness.get_by_label("Stage All Changes").click();
    idle(&mut harness);
    wait(&mut harness, "the commit button", |h| has_label(h, "Commit 1 file to main"));
    harness.get_by_label_contains("Commit 1 file to main").click();
    idle(&mut harness);
    assert_eq!(git(path, &["log", "-1", "--format=%s"]), "fix(ui)!: Keep the panel open");
}

#[test]
fn plain_commits_show_no_type_menu_until_the_setting_is_always() {
    let repo = repository();
    let path = repo.path();
    let mut harness = open(path);
    loaded(&mut harness);
    assert!(!has_label(&harness, "Commit type"), "Automatic shows no type menu in a repository with plain commits");

    harness.state_mut().show_settings = true;
    settle(&mut harness);
    wait(&mut harness, "the Always choice", |h| h.query_by_label("Always").is_some());
    harness.get_by_label("Always").click();
    settle(&mut harness);
    assert_eq!(harness.state().settings.conventional_commits, ConventionalCommits::Always);

    harness.state_mut().show_settings = false;
    wait(&mut harness, "the type menu", |h| has_label(h, "Commit type"));
}

#[test]
fn a_summary_past_eighty_characters_shows_its_length_and_a_warning() {
    let repo = repository();
    let mut harness = open(repo.path());
    loaded(&mut harness);

    type_into(&mut harness, "Commit summary", &"a".repeat(80));
    wait(&mut harness, "the length note", |h| has_label(h, "80/72"));
    assert!(has_label(&harness, "long summaries are cut off"), "the warning explains the limit");
}

#[test]
fn automatic_mode_hints_at_the_convention_for_a_summary_without_a_type() {
    let repo = conventional_repository();
    let mut harness = open(repo.path());
    loaded(&mut harness);

    type_into(&mut harness, "Commit summary", "Keep the panel open");
    wait(&mut harness, "the convention hint", |h| has_label(h, "This repository uses Conventional Commits"));
}
