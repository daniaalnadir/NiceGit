//! Short tips about features NiceGit has, shown on the welcome screen and by the command
//! palette's Show a tip. The text is written with `Cmd` for Command, which Windows and Linux
//! show as `Ctrl`, as the shortcuts window does.

use std::time::{SystemTime, UNIX_EPOCH};

use crate::app::NiceGitApp;

pub const TIPS: &[&str] = &[
    "Stage one line at a time: press + beside a line in the diff, or drag over several lines first.",
    "Press Shift-Cmd-P to open the command palette, which lists most actions and their shortcuts.",
    "Made a mistake in a commit, merge, or rebase? Undo in the toolbar steps back through recent actions.",
    "Lost commits after a reset or rebase? Repository > Recover lost work lists every earlier position of HEAD.",
    "Drag a branch onto the current branch in the sidebar to merge it in or rebase onto it.",
    "Fix up with staged changes, in a commit's menu, folds what you have staged into that older commit.",
    "Blame a file to see who changed each line. Right-click a line to blame the file before that change.",
    "Cmd-P goes to any tracked file by name, and Cmd-Enter in that list opens the file's blame.",
    "Press Ctrl-` to show or hide the terminal. Its shells keep running while it is hidden.",
    "Settings (Cmd-,) offers a colour-blind safe graph palette.",
    "Press Cmd-/ to see every keyboard shortcut.",
    "Middle-click a repository tab to close it, as in a browser.",
    "Cmd-click commits in the graph to select several, then cherry-pick them together.",
];

/// The tip at `index`, with the modifier names this platform uses.
pub fn text(index: usize) -> String {
    let command = if cfg!(target_os = "macos") { "Cmd" } else { "Ctrl" };
    TIPS[index % TIPS.len()].replace("Cmd", command)
}

/// The tip a run starts on: it changes each day, and stays the same for the whole run.
pub fn starting_index() -> usize {
    let day = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |elapsed| elapsed.as_secs() / 86_400);
    (day % TIPS.len() as u64) as usize
}

impl NiceGitApp {
    /// Moves to the next tip, wrapping after the last one.
    pub fn next_tip(&mut self) {
        self.welcome_tip = (self.welcome_tip + 1) % TIPS.len();
    }

    /// The command palette's Show a tip: the next tip, as a notice.
    pub fn show_tip(&mut self) {
        self.next_tip();
        self.notify(text(self.welcome_tip), false);
    }
}

#[cfg(test)]
mod tests {
    use super::{text, TIPS};

    #[test]
    fn every_tip_is_a_short_full_sentence() {
        assert!(!TIPS.is_empty());
        for tip in TIPS {
            assert!(!tip.is_empty(), "a tip is empty");
            assert!(tip.ends_with('.'), "{tip} does not end with a full stop");
            assert!(tip.chars().count() < 140, "{tip} is too long");
        }
    }

    #[test]
    fn shown_tips_are_short_full_sentences_too() {
        for index in 0..TIPS.len() {
            let shown = text(index);
            assert!(shown.ends_with('.') && shown.chars().count() < 140, "{shown}");
            assert!(cfg!(target_os = "macos") || !shown.contains("Cmd"), "{shown} keeps Cmd off this platform");
        }
    }
}
