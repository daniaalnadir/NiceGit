//! Go to file (Command-P): a fuzzy finder over the repository's tracked files. Enter opens the
//! file's history, Command-Enter its blame, and Option-Enter the built-in editor.

use std::path::PathBuf;

use egui::{Key, KeyboardShortcut, Modifiers, RichText};
use egui_phosphor::regular as icon;

use crate::app::NiceGitApp;
use crate::theme;
use crate::tools::{self, Task};
use crate::ui::changes;
use crate::ui::palette::score;

/// The most matches the finder lists at once.
const LIMIT: usize = 50;

/// Added when the whole query matches inside the file name, so `app.rs` ranks `src/app.rs`
/// above `src/apps/x.rs`.
const NAME_BONUS: i32 = 30;

/// What the file listing holds: still loading, read, or failed.
enum Listing {
    Loading,
    Ready(Vec<String>),
    Failed(String),
}

pub struct GoToFile {
    /// The repository the files were listed from. The finder closes when another becomes active.
    root: PathBuf,
    query: String,
    selection: usize,
    focused: bool,
    task: Task<nicegit_core::Result<Vec<String>>>,
    listing: Listing,
    /// Indexes into the listed files, best match first, for `matched_for`.
    matches: Vec<usize>,
    matched_for: Option<String>,
}

/// What a row's menu or Enter asked for.
#[derive(Clone, Copy)]
enum Open {
    History,
    Blame,
    Edit,
    Reveal,
    CopyPath,
}

/// Indexes of the files that match `query`, best first, at most `LIMIT`. Files that tie keep
/// Git's order. The query's spaces are ignored, as in the command palette.
pub(crate) fn rank(files: &[String], query: &str) -> Vec<usize> {
    let needle: Vec<char> = query.to_lowercase().chars().filter(|c| !c.is_whitespace()).collect();
    if needle.is_empty() {
        return Vec::new();
    }
    let mut scored: Vec<(usize, i32)> = files
        .iter()
        .enumerate()
        .filter_map(|(index, path)| {
            let lower: Vec<char> = path.to_lowercase().chars().collect();
            let whole = score(&lower, &needle)?;
            let name_start = lower.iter().rposition(|c| *c == '/').map_or(0, |slash| slash + 1);
            let in_name = score(&lower[name_start..], &needle).is_some();
            Some((index, whole + if in_name { NAME_BONUS } else { 0 }))
        })
        .collect();
    // A stable sort keeps tied files in the order Git listed them.
    scored.sort_by_key(|(_, points)| std::cmp::Reverse(*points));
    scored.truncate(LIMIT);
    scored.into_iter().map(|(index, _)| index).collect()
}

impl GoToFile {
    fn new(ctx: &egui::Context, root: PathBuf) -> Self {
        let task = tools::query(ctx, &root, |git, directory| git.tracked_files(directory));
        Self {
            root,
            query: String::new(),
            selection: 0,
            focused: false,
            task,
            listing: Listing::Loading,
            matches: Vec::new(),
            matched_for: None,
        }
    }

    /// Takes the listing once it has arrived.
    fn poll(&mut self) {
        if let Some(result) = self.task.get() {
            self.listing = match result {
                Ok(files) => Listing::Ready(files.clone()),
                Err(error) => Listing::Failed(error.to_string()),
            };
        }
    }

    /// Ranks the files again only when the query or the listing has changed since last time.
    fn rank_if_stale(&mut self) {
        let Listing::Ready(files) = &self.listing else { return };
        if self.matched_for.as_deref() != Some(self.query.as_str()) {
            self.matches = rank(files, &self.query);
            self.matched_for = Some(self.query.clone());
        }
    }
}

impl NiceGitApp {
    /// Command-P: opens the finder, or closes it when it is open. Does nothing without a repository.
    pub fn toggle_go_to_file(&mut self, ctx: &egui::Context) {
        if self.go_to_file.is_some() {
            self.go_to_file = None;
        } else {
            self.open_go_to_file(ctx);
        }
    }

    /// Opens the finder over the active repository's tracked files, listing them afresh.
    pub fn open_go_to_file(&mut self, ctx: &egui::Context) {
        if self.snapshot().is_none() {
            return;
        }
        if let Some(root) = self.repo().map(|repo| repo.path.clone()) {
            self.go_to_file = Some(GoToFile::new(ctx, root));
        }
    }

    pub fn go_to_file_window(&mut self, ctx: &egui::Context) {
        let Some(mut state) = self.go_to_file.take() else { return };
        // The finder lists one repository. When another becomes active, it closes.
        if self.repo().map(|repo| &repo.path) != Some(&state.root) {
            return;
        }
        state.poll();
        state.rank_if_stale();
        let command = if cfg!(target_os = "macos") { "Cmd" } else { "Ctrl" };
        let alt = if cfg!(target_os = "macos") { "Option" } else { "Alt" };
        let mut chosen: Option<(String, Open)> = None;
        let mut close = false;
        let modal = egui::Modal::new(egui::Id::new("go_to_file")).show(ctx, |ui| {
            let c = theme::of(ui);
            ui.set_width(580.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new(icon::FILE_MAGNIFYING_GLASS).size(18.0).color(c.muted));
                let field = ui.add(
                    egui::TextEdit::singleline(&mut state.query)
                        .hint_text("Type a file name")
                        .frame(egui::Frame::NONE)
                        .font(egui::FontId::proportional(17.0))
                        .desired_width(f32::INFINITY),
                );
                if !state.focused {
                    field.request_focus();
                    state.focused = true;
                }
                if field.changed() {
                    state.selection = 0;
                }
            });
            ui.separator();
            let (down, up, escape) = ui.input(|i| (i.key_pressed(Key::ArrowDown), i.key_pressed(Key::ArrowUp), i.key_pressed(Key::Escape)));
            // The modifier combinations are consumed here, before the field could use them.
            let by_command = ui.input_mut(|i| i.consume_shortcut(&KeyboardShortcut::new(Modifiers::COMMAND, Key::Enter)));
            let by_alt = ui.input_mut(|i| i.consume_shortcut(&KeyboardShortcut::new(Modifiers::ALT, Key::Enter)));
            let by_enter = ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter));
            let last = state.matches.len().saturating_sub(1);
            if down {
                state.selection = (state.selection + 1).min(last);
            }
            if up {
                state.selection = state.selection.saturating_sub(1);
            }
            state.selection = state.selection.min(last);
            if escape {
                close = true;
            }
            let files: &[String] = match &state.listing {
                Listing::Ready(files) => files,
                _ => &[],
            };
            let selected_path = state.matches.get(state.selection).and_then(|&index| files.get(index)).cloned();
            if let Some(path) = &selected_path {
                if by_command {
                    chosen = Some((path.clone(), Open::Blame));
                } else if by_alt {
                    chosen = Some((path.clone(), Open::Edit));
                } else if by_enter {
                    chosen = Some((path.clone(), Open::History));
                }
            }
            egui::ScrollArea::vertical().max_height(380.0).auto_shrink([false, true]).show(ui, |ui| match &state.listing {
                Listing::Loading => {
                    ui.label(RichText::new("Loading files…").color(c.muted));
                }
                Listing::Failed(error) => {
                    ui.label(RichText::new(error).color(c.danger));
                }
                Listing::Ready(_) if state.query.trim().is_empty() => {
                    ui.label(RichText::new("Type to find a file").color(c.muted));
                }
                Listing::Ready(_) if state.matches.is_empty() => {
                    ui.label(RichText::new("No matching files").color(c.muted));
                }
                Listing::Ready(_) => {
                    for (row, &index) in state.matches.iter().enumerate() {
                        let path = &files[index];
                        let selected = row == state.selection;
                        let frame = egui::Frame::new()
                            .fill(if selected { ui.visuals().selection.bg_fill } else { egui::Color32::TRANSPARENT })
                            .corner_radius(6.0)
                            .inner_margin(egui::Margin::symmetric(10, 6));
                        let (name, folder) = match path.rfind('/') {
                            Some(slash) => (&path[slash + 1..], &path[..slash]),
                            None => (path.as_str(), ""),
                        };
                        let on_disk = state.root.join(path).is_file();
                        let response = frame
                            .show(ui, |ui| {
                                ui.set_width(ui.available_width());
                                ui.horizontal(|ui| {
                                    ui.label(RichText::new(icon::FILE).color(c.muted));
                                    ui.label(name);
                                    if !folder.is_empty() {
                                        ui.add(egui::Label::new(RichText::new(folder).small().color(c.muted)).truncate());
                                    }
                                });
                            })
                            .response
                            .interact(egui::Sense::click());
                        if selected && (down || up) {
                            response.scroll_to_me(None);
                        }
                        response.context_menu(|ui| {
                            let items = [
                                (format!("{}  File history", icon::CLOCK_COUNTER_CLOCKWISE), Open::History, true),
                                (format!("{}  Blame", icon::USER_LIST), Open::Blame, on_disk),
                                (format!("{}  Edit", icon::PENCIL_SIMPLE), Open::Edit, on_disk),
                                (format!("{}  Reveal in file manager", icon::FOLDER_OPEN), Open::Reveal, true),
                                (format!("{}  Copy path", icon::COPY), Open::CopyPath, true),
                            ];
                            for (text, action, enabled) in items {
                                if ui.add_enabled(enabled, egui::Button::new(text)).clicked() {
                                    ui.close();
                                    chosen = Some((path.clone(), action));
                                }
                            }
                        });
                        if response.clicked() {
                            chosen = Some((path.clone(), Open::History));
                        }
                    }
                }
            });
            ui.separator();
            ui.label(RichText::new(format!("Enter history · {command}-Enter blame · {alt}-Enter edit")).small().color(c.muted));
        });
        if modal.should_close() {
            close = true;
        }
        if let Some((path, action)) = chosen {
            close = true;
            self.go_to_file_action(ctx, path, action);
        }
        if !close {
            self.go_to_file = Some(state);
        }
    }

    /// Runs one of the finder's actions on a listed file. The finder has closed by then.
    fn go_to_file_action(&mut self, ctx: &egui::Context, path: String, action: Open) {
        let Some(root) = self.repo().map(|repo| repo.path.clone()) else { return };
        let file = root.join(&path);
        match action {
            Open::History => self.open_tool(Box::new(tools::file_history::FileHistoryWindow::new(path))),
            Open::Blame | Open::Edit if !file.is_file() => {
                self.notify(format!("{path} is not in the working folder, so it has no file to open."), true);
            }
            Open::Blame => self.open_tool(Box::new(tools::blame::BlameWindow::new(path, None))),
            Open::Edit => {
                // A file with unsaved edits in its editor is not replaced by a fresh copy.
                let id = format!("editor:{path}");
                if self.tools.iter().any(|tool| tool.id() == id && tool.has_unsaved_changes()) {
                    self.notify("Save or close the file you are editing first.", true);
                } else {
                    self.open_tool(Box::new(tools::editor::EditorWindow::new(root, path)));
                }
            }
            Open::Reveal => changes::reveal(&file),
            Open::CopyPath => ctx.copy_text(path),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::rank;

    fn files(paths: &[&str]) -> Vec<String> {
        paths.iter().map(|path| path.to_string()).collect()
    }

    #[test]
    fn a_file_name_match_ranks_above_a_folder_that_contains_the_letters() {
        let list = files(&["src/apps/x.rs", "src/app.rs"]);
        assert_eq!(rank(&list, "app.rs"), vec![1, 0]);
    }

    #[test]
    fn a_query_matching_no_file_gives_nothing() {
        let list = files(&["src/app.rs", "README.md"]);
        assert!(rank(&list, "zzzq").is_empty());
        assert!(rank(&list, "   ").is_empty(), "an empty query shows nothing");
    }

    #[test]
    fn tied_files_keep_git_order() {
        let list = files(&["x/a.rs", "y/a.rs", "z/a.rs"]);
        assert_eq!(rank(&list, "a.rs"), vec![0, 1, 2]);
        let reversed = files(&["z/a.rs", "y/a.rs", "x/a.rs"]);
        assert_eq!(rank(&reversed, "a.rs"), vec![0, 1, 2]);
    }

    #[test]
    fn at_most_fifty_matches_are_listed() {
        let list: Vec<String> = (0..80).map(|n| format!("dir/file{n}.txt")).collect();
        assert_eq!(rank(&list, "file").len(), 50);
    }
}
