//! Accessibility audit: every control in the open windows has a label that a screen reader can
//! speak. The tests read the accessibility tree egui builds (AccessKit) and report each control
//! that has no words, only an icon glyph or nothing at all.

use egui::accesskit::{Action, Role};
use egui::{Key, Modifiers};
use egui_kittest::kittest::{NodeT, Queryable};
use egui_kittest::Harness;

use crate::app::NiceGitApp;
use crate::ui_tests::{git, loaded, open, repository, settle, wait};

/// The roles a person activates or changes, which need a name for a screen reader.
const CONTROL_ROLES: &[Role] = &[
    Role::Button,
    Role::CheckBox,
    Role::RadioButton,
    Role::ComboBox,
    Role::TextInput,
    Role::MultilineTextInput,
    Role::Slider,
    Role::Link,
    Role::Tab,
    Role::MenuItem,
];

/// Whether `text` has no words: it is empty, blank, or only icon glyphs from the private-use area
/// that egui-phosphor draws its icons in.
fn is_glyph_only(text: &str) -> bool {
    text.chars().all(|c| c.is_whitespace() || ('\u{E000}'..='\u{F8FF}').contains(&c))
}

/// Describes every control in the accessibility tree that has no label and no value a screen
/// reader could speak, by role and bounds. An empty list means every control is named.
pub(crate) fn unlabelled_controls(harness: &Harness<'static, NiceGitApp>) -> Vec<String> {
    harness
        .root()
        .children_recursive()
        .filter_map(|node| {
            let access = node.accesskit_node();
            let role = access.role();
            // A custom-painted widget that takes clicks has no role of its own: egui gives it
            // Unknown, which a screen reader cannot name.
            let clickable_unnamed = role == Role::Unknown && access.data().supports_action(Action::Click);
            if !CONTROL_ROLES.contains(&role) && !clickable_unnamed {
                return None;
            }
            let label = access.label().unwrap_or_default();
            let value = access.value().unwrap_or_default();
            if !is_glyph_only(&label) || !is_glyph_only(&value) {
                return None;
            }
            let bounds = access.bounding_box().map(|rect| format!("{rect:?}")).unwrap_or_else(|| "no bounds".into());
            let mut chain = Vec::new();
            let mut up = access.parent();
            while let Some(ancestor) = up {
                let name = ancestor.label().unwrap_or_default();
                if ancestor.role() != Role::GenericContainer || !name.is_empty() {
                    chain.push(format!("{:?} {name:?}", ancestor.role()));
                }
                up = ancestor.parent();
            }
            chain.truncate(3);
            Some(format!("{role:?} at {bounds} (label {label:?}), inside {}", chain.join(" < ")))
        })
        .collect()
}

fn assert_all_labelled(harness: &Harness<'static, NiceGitApp>, what: &str) {
    let missing = unlabelled_controls(harness);
    assert!(missing.is_empty(), "{what} has {} unlabelled controls:\n{}", missing.len(), missing.join("\n"));
}

/// Waits until the selected commit's details have arrived, so its inspector is complete.
fn wait_for_details(harness: &mut Harness<'static, NiceGitApp>) {
    wait(harness, "the commit's details", |h| {
        h.state().repo().is_some_and(|r| !r.commit_files.is_empty() && r.details.as_ref().is_some_and(|task| task.peek().is_some()))
    });
}

/// Opens the command palette, runs the first command matching `query`, and lets the window appear.
fn run_from_palette(harness: &mut Harness<'static, NiceGitApp>, query: &str) {
    harness.key_press_modifiers(Modifiers::COMMAND | Modifiers::SHIFT, Key::P);
    wait(harness, "the palette", |h| h.state().palette.is_some());
    harness.event(egui::Event::Text(query.into()));
    harness.step();
    harness.key_press(Key::Enter);
    settle(harness);
}

/// A repository with a changed file, so the Changes list and its diff are on screen.
fn repository_with_a_change() -> tempfile::TempDir {
    let repo = repository();
    std::fs::write(repo.path().join("notes.txt"), "first\nsecond\nthird\n").unwrap();
    repo
}

#[test]
fn main_window_with_a_changed_file_and_its_diff_is_labelled() {
    let repo = repository_with_a_change();
    let mut harness = open(repo.path());
    loaded(&mut harness);
    let change = harness.state().snapshot().unwrap().status.first().cloned().expect("a changed file");
    harness.state_mut().select_change(change, false);
    wait(&mut harness, "the diff", |h| h.state().repo().is_some_and(|r| r.diff.is_some()));
    settle(&mut harness);
    assert_all_labelled(&harness, "the main window with a diff");
}

#[test]
fn main_window_with_history_and_the_working_tree_is_labelled() {
    let repo = repository_with_a_change();
    let mut harness = open(repo.path());
    loaded(&mut harness);
    settle(&mut harness);
    assert_all_labelled(&harness, "the main window");
}

#[test]
fn commit_inspector_is_labelled() {
    let repo = repository();
    let head = git(repo.path(), &["rev-parse", "HEAD"]);
    let mut harness = open(repo.path());
    loaded(&mut harness);
    harness.state_mut().select_commit(head);
    wait_for_details(&mut harness);
    settle(&mut harness);
    assert_all_labelled(&harness, "the commit inspector");
}

#[test]
fn settings_window_is_labelled() {
    let repo = repository();
    let mut harness = open(repo.path());
    loaded(&mut harness);
    harness.state_mut().show_settings = true;
    settle(&mut harness);
    assert_all_labelled(&harness, "Settings");
}

#[test]
fn command_palette_is_labelled() {
    let repo = repository();
    let mut harness = open(repo.path());
    loaded(&mut harness);
    harness.key_press_modifiers(Modifiers::COMMAND | Modifiers::SHIFT, Key::P);
    wait(&mut harness, "the palette", |h| h.state().palette.is_some());
    settle(&mut harness);
    assert_all_labelled(&harness, "the command palette");
}

#[test]
fn stashes_window_is_labelled() {
    let repo = repository_with_a_change();
    let mut harness = open(repo.path());
    loaded(&mut harness);
    run_from_palette(&mut harness, "Stashes");
    assert_all_labelled(&harness, "the Stashes window");
}

#[test]
fn insights_window_is_labelled() {
    let repo = repository();
    let mut harness = open(repo.path());
    loaded(&mut harness);
    run_from_palette(&mut harness, "Insights");
    assert_all_labelled(&harness, "the Insights window");
}

#[test]
fn range_diff_window_is_labelled() {
    let repo = repository();
    let mut harness = open(repo.path());
    loaded(&mut harness);
    run_from_palette(&mut harness, "Compare branch versions");
    assert_all_labelled(&harness, "the Compare branch versions window");
}

#[test]
fn apply_patch_window_is_labelled() {
    let repo = repository();
    let mut harness = open(repo.path());
    loaded(&mut harness);
    run_from_palette(&mut harness, "Apply patch");
    assert_all_labelled(&harness, "the Apply patch window");
}

#[test]
fn absorb_window_is_labelled() {
    let repo = repository();
    std::fs::write(repo.path().join("notes.txt"), "first\nSECOND\n").unwrap();
    git(repo.path(), &["add", "notes.txt"]);
    let mut harness = open(repo.path());
    loaded(&mut harness);
    run_from_palette(&mut harness, "Absorb staged");
    assert_all_labelled(&harness, "the Absorb window");
}

#[test]
fn blame_window_is_labelled() {
    let repo = repository();
    let mut harness = open(repo.path());
    loaded(&mut harness);
    harness.state_mut().open_tool(Box::new(crate::tools::blame::BlameWindow::new("notes.txt".into(), None)));
    settle(&mut harness);
    assert_all_labelled(&harness, "the Blame window");
}

#[test]
fn file_history_window_is_labelled() {
    let repo = repository();
    let mut harness = open(repo.path());
    loaded(&mut harness);
    harness.state_mut().open_tool(Box::new(crate::tools::file_history::FileHistoryWindow::new("notes.txt".into())));
    settle(&mut harness);
    assert_all_labelled(&harness, "the File history window");
}

#[test]
fn branch_compare_window_is_labelled() {
    let repo = repository();
    let tip = git(repo.path(), &["rev-parse", "feature"]);
    let mut harness = open(repo.path());
    loaded(&mut harness);
    harness.state_mut().open_tool(Box::new(crate::tools::branch_compare::BranchCompareWindow::new("feature".into(), tip, "main".into())));
    settle(&mut harness);
    assert_all_labelled(&harness, "the Branch compare window");
}

#[test]
fn split_commit_window_is_labelled() {
    let repo = repository();
    let head = git(repo.path(), &["rev-parse", "HEAD"]);
    let mut harness = open(repo.path());
    loaded(&mut harness);
    let snapshot = harness.state().snapshot().cloned().expect("a snapshot");
    let window = crate::tools::split_commit::SplitWindow::new(&head, "Add a second line", &snapshot);
    harness.state_mut().open_tool(Box::new(window));
    settle(&mut harness);
    assert_all_labelled(&harness, "the Split commit window");
}

/// The selected commit's hash, or None when a file or the working tree is selected.
fn selected_commit(harness: &Harness<'static, NiceGitApp>) -> Option<String> {
    match harness.state().repo().map(|r| r.selection.clone()) {
        Some(crate::app::Selection::Commit { hash, .. }) => Some(hash),
        _ => None,
    }
}

/// Whether a text node says `text`. A static text node, such as a diff line, keeps its words as
/// its value rather than its label.
fn has_text(harness: &Harness<'static, NiceGitApp>, text: &str) -> bool {
    harness.root().children_recursive().any(|node| node.accesskit_node().value().as_deref() == Some(text))
}

#[test]
fn commit_rows_are_named_by_subject_and_described_with_their_details() {
    let repo = repository();
    let head = git(repo.path(), &["rev-parse", "HEAD"]);
    let short = git(repo.path(), &["rev-parse", "--short", "HEAD"]);
    let mut harness = open(repo.path());
    loaded(&mut harness);
    harness.state_mut().select_commit(head);
    settle(&mut harness);

    let row = harness.get_by_role_and_label(Role::Button, "Add a second line").accesskit_node();
    assert!(row.data().is_selected() == Some(true) && row.data().toggled().is_none(), "the selected row says so, as a list item does");
    let description = row.data().description().unwrap_or_default().to_string();
    assert!(description.starts_with(&format!("Commit {short}, by Test")), "the description gives the commit: {description}");
    assert!(description.contains("HEAD"), "HEAD is named: {description}");
    let other = harness.get_by_role_and_label(Role::Button, "Start the notes").accesskit_node();
    assert_eq!(other.data().is_selected(), Some(false), "the other row is not selected");
}

#[test]
fn diff_lines_are_read_with_their_kind_and_number() {
    let repo = repository_with_a_change();
    let mut harness = open(repo.path());
    loaded(&mut harness);
    let change = harness.state().snapshot().unwrap().status.first().cloned().expect("a changed file");
    harness.state_mut().select_change(change, false);
    wait(&mut harness, "the diff", |h| h.state().repo().is_some_and(|r| r.diff.is_some()));
    settle(&mut harness);

    assert!(has_text(&harness, "Added line 3: third"), "the added line is named with its number");
    assert!(has_text(&harness, "Line 2: second"), "the unchanged line is named as a line");
    assert!(has_text(&harness, "Hunk at lines 1 to 3"), "the hunk header gives its range");
}

#[test]
fn arrow_keys_move_the_graph_and_focus_the_selected_row() {
    let repo = repository();
    let head = git(repo.path(), &["rev-parse", "HEAD"]);
    let parent = git(repo.path(), &["rev-parse", "HEAD~1"]);
    let mut harness = open(repo.path());
    loaded(&mut harness);

    harness.key_press(Key::ArrowDown);
    harness.step();
    assert_eq!(selected_commit(&harness), Some(head.clone()));
    harness.run_steps(2);
    assert!(
        harness.get_by_role_and_label(Role::Button, "Add a second line").accesskit_node().is_focused(),
        "the row the arrow key chose has keyboard focus"
    );

    // With the row focused, the arrow keys still move through the graph.
    harness.key_press(Key::ArrowDown);
    harness.step();
    assert_eq!(selected_commit(&harness), Some(parent.clone()));
    harness.run_steps(2);
    assert!(harness.get_by_role_and_label(Role::Button, "Start the notes").accesskit_node().is_focused());
}

#[test]
fn larger_text_and_reduce_motion_are_settings_that_apply_at_once() {
    let repo = repository();
    let mut harness = open(repo.path());
    loaded(&mut harness);
    // The Accessibility section is at the foot of Settings, below the Snapshots section and the Gitea
    // and Forgejo token rows, so the window is made tall enough to show it.
    harness.set_size(egui::vec2(1440.0, 3600.0));
    harness.state_mut().show_settings = true;
    settle(&mut harness);
    let zoom = harness.ctx.zoom_factor();

    harness.get_by_role_and_label(Role::CheckBox, "Larger text").click();
    settle(&mut harness);
    assert!(harness.state().settings.larger_text, "the choice is saved");
    assert!((harness.ctx.zoom_factor() - zoom * 1.25).abs() < 0.001, "the interface is a quarter larger");

    harness.get_by_role_and_label(Role::CheckBox, "Reduce motion").click();
    settle(&mut harness);
    assert!(harness.state().settings.reduce_motion, "the choice is saved");
    assert_eq!(harness.ctx.style_of(harness.ctx.theme()).animation_time, 0.0, "animations are switched off");
}

#[test]
fn a_tool_window_title_is_announced_as_the_window_title() {
    let repo = repository();
    let mut harness = open(repo.path());
    loaded(&mut harness);
    harness.state_mut().open_tool(Box::new(crate::tools::file_history::FileHistoryWindow::new("notes.txt".into())));
    settle(&mut harness);
    assert!(
        harness.query_by_role_and_label(Role::TitleBar, "File history: notes.txt").is_some(),
        "the title bar carries the window's title"
    );
}
