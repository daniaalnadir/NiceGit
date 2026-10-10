use egui::RichText;

use crate::app::NiceGitApp;
use crate::theme;
use crate::tools::widgets;

/// The keyboard shortcuts, by where they apply. Keys are written for this platform.
fn groups() -> Vec<(&'static str, Vec<(String, &'static str)>)> {
    let command = if cfg!(target_os = "macos") { "Cmd" } else { "Ctrl" };
    let alt = if cfg!(target_os = "macos") { "Option" } else { "Alt" };
    let with = |keys: &str| keys.replace("Cmd", command).replace("Alt", alt);
    vec![
        (
            "Anywhere",
            vec![
                (with("Shift-Cmd-P"), "Command palette"),
                (with("Cmd-O"), "Open a repository"),
                (with("Cmd-R"), "Refresh the repository"),
                (with("Shift-Cmd-F"), "Search the history of every branch"),
                (with("Alt-Cmd-F"), "Search file contents"),
                ("Ctrl-`".to_string(), "Show or hide the terminal"),
                (with("Cmd-,"), "Settings"),
                (with("Cmd-/"), "These shortcuts"),
            ],
        ),
        (
            "History",
            vec![
                ("Up, Down".to_string(), "Select the previous or next commit"),
                (with("Cmd-click"), "Mark several commits to cherry-pick"),
                ("Escape".to_string(), "Clear the selection"),
            ],
        ),
        (
            "Diff",
            vec![
                (with("Alt-Up, Alt-Down"), "Open the previous or next changed file"),
                (with("Cmd-F"), "Find in the diff"),
                ("Enter, Shift-Enter".to_string(), "Next or previous match"),
                ("Escape".to_string(), "Back to the history"),
            ],
        ),
        ("Committing", vec![(with("Cmd-Enter"), "Commit the staged changes"), (with("Cmd-S"), "Save in the built-in editor")]),
    ]
}

impl NiceGitApp {
    pub fn shortcuts_window(&mut self, ctx: &egui::Context) {
        if !self.show_shortcuts {
            return;
        }
        let mut open = true;
        egui::Window::new("Keyboard shortcuts")
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
