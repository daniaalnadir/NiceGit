use egui::RichText;

use crate::app::NiceGitApp;
use crate::i18n::tr;
use crate::theme;
use crate::tools::widgets;

/// The keyboard shortcuts, by where they apply. Keys are written for this platform.
fn groups() -> Vec<(&'static str, Vec<(String, &'static str)>)> {
    let command = if cfg!(target_os = "macos") { "Cmd" } else { "Ctrl" };
    let alt = if cfg!(target_os = "macos") { "Option" } else { "Alt" };
    let with = |keys: &str| keys.replace("Cmd", command).replace("Alt", alt);
    vec![
        (
            tr("Anywhere"),
            vec![
                (with("Shift-Cmd-P"), tr("Command palette")),
                (with("Cmd-P"), tr("Go to a file")),
                (with("Cmd-O"), tr("Open a repository")),
                (with("Cmd-1 … Cmd-9"), tr("Go to a repository tab; Cmd-9 for the last")),
                (with("Cmd-W"), tr("Close the open window, or else the repository tab")),
                (with("Cmd-R"), tr("Refresh the repository")),
                (with("Shift-Cmd-F"), tr("Search the history of every branch")),
                (with("Shift-Cmd-L"), tr("Filter the changed files")),
                (with("Alt-Cmd-F"), tr("Search file contents")),
                (with("Shift-Cmd-H"), tr("Go to the checked-out commit")),
                (with("Alt-Cmd-L"), tr("Show the command log")),
                ("Ctrl-`".to_string(), tr("Show or hide the terminal")),
                (with("Cmd-,"), tr("Settings")),
                (with("Cmd-+, Cmd-−"), tr("Make everything larger or smaller")),
                (with("Cmd-0"), tr("Normal size")),
                (with("Cmd-/"), tr("These shortcuts")),
            ],
        ),
        (
            tr("History"),
            vec![
                ("Up, Down".to_string(), tr("Select the previous or next commit")),
                ("Home, End".to_string(), tr("Jump to the top or bottom of the loaded history")),
                (with("Cmd-click"), tr("Mark several commits to cherry-pick")),
                ("Escape".to_string(), tr("Clear the selection")),
            ],
        ),
        (
            tr("Diff"),
            vec![
                (with("Alt-Up, Alt-Down"), tr("Open the previous or next changed file")),
                (with("Shift-Cmd-S"), tr("Stage or unstage the open file")),
                (with("Cmd-F"), tr("Find in the diff")),
                ("Enter, Shift-Enter".to_string(), tr("Next or previous match")),
                ("Escape".to_string(), tr("Back to the history")),
            ],
        ),
        (
            tr("Committing"),
            vec![
                (with("Cmd-Enter"), tr("Commit the staged changes")),
                (with("Shift-Cmd-Enter"), tr("Commit and push")),
                (with("Cmd-S"), tr("Save in the built-in editor")),
            ],
        ),
    ]
}

impl NiceGitApp {
    pub fn shortcuts_window(&mut self, ctx: &egui::Context) {
        if !self.show_shortcuts {
            return;
        }
        let mut open = true;
        egui::Window::new(tr("Keyboard shortcuts"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(420.0)
            .pivot(egui::Align2::CENTER_CENTER)
            .default_pos(ctx.content_rect().center())
            .show(ctx, |ui| {
                let c = theme::of(ui);
                for (title, shortcuts) in groups() {
                    widgets::section(ui, title);
                    egui::Grid::new(("shortcuts", title)).num_columns(2).spacing([18.0, 6.0]).min_col_width(150.0).show(ui, |ui| {
                        for (keys, what) in shortcuts {
                            ui.label(RichText::new(keys).monospace().color(c.accent));
                            ui.label(what);
                            ui.end_row();
                        }
                    });
                    ui.add_space(8.0);
                }
            });
        self.show_shortcuts = open;
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn every_shortcut_has_keys_and_a_description_and_appears_once() {
        let mut seen = std::collections::BTreeSet::new();
        for (_, shortcuts) in super::groups() {
            for (keys, what) in shortcuts {
                assert!(!keys.is_empty() && !what.is_empty());
                assert!(seen.insert((keys.clone(), what)), "{keys} listed twice");
            }
        }
    }
}
