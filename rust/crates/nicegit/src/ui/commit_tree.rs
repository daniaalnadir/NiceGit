//! The commit inspector's All files view, and the read-only file view it opens: every file in a
//! commit's tree as a folder tree, and one file's contents as it was at that commit.

use std::collections::BTreeMap;
use std::sync::Arc;

use egui::{RichText, Ui};
use egui_phosphor::regular as icon;
use nicegit_core::commit_tree::{FileView, TreeEntry, TreeListing};
use nicegit_core::models::short;

use crate::app::{NiceGitApp, Selection};
use crate::theme;
use crate::tools::{self, widgets};

/// A folder in the All files tree: the files directly inside it and its subfolders.
#[derive(Default)]
pub struct TreeFolder {
    files: Vec<TreeEntry>,
    folders: BTreeMap<String, TreeFolder>,
}

impl TreeFolder {
    fn insert(&mut self, entry: &TreeEntry) {
        let mut folder = self;
        let mut parts = entry.path.split('/').peekable();
        while let Some(part) = parts.next() {
            if parts.peek().is_none() {
                folder.files.push(entry.clone());
            } else {
                folder = folder.folders.entry(part.to_string()).or_default();
            }
        }
    }

    /// Every file under this folder, in its subfolders too.
    fn count(&self) -> usize {
        self.files.len() + self.folders.values().map(TreeFolder::count).sum::<usize>()
    }
}

/// A listed tree, arranged into folders for the All files view.
pub struct TreeIndex {
    pub root: TreeFolder,
    /// Entries the view shows, and every entry in the tree.
    pub shown: usize,
    pub total: usize,
}

impl TreeIndex {
    pub fn new(listing: &TreeListing) -> Self {
        let mut root = TreeFolder::default();
        for entry in &listing.entries {
            root.insert(entry);
        }
        Self { root, shown: listing.entries.len(), total: listing.total }
    }

    fn truncated(&self) -> bool {
        self.total > self.shown
    }
}

/// What the file view offers beyond reading the file, decided after the view is drawn.
enum Ask {
    OpenAnyway,
    SaveCopy,
}

impl NiceGitApp {
    /// Shows the All files view: the commit's whole tree, listed in the background and then drawn
    /// as folders. `viewing` is the file the file view shows, if any, so its row stays selected.
    pub(crate) fn all_files_list(&mut self, ui: &mut Ui, hash: &str, viewing: Option<&str>) {
        let c = theme::of(ui);
        let Some(repo) = self.repos.get_mut(self.active) else { return };
        let Some(tree) = repo.commit_tree.as_mut().filter(|tree| tree.hash == hash) else {
            widgets::loading(ui, "Listing files…");
            return;
        };
        // The folder index is built once, when the listing has arrived.
        if tree.index.is_none() {
            let built = match tree.listing.get() {
                Some(Ok(listing)) => Some(Arc::new(TreeIndex::new(listing))),
                _ => None,
            };
            tree.index = built;
        }
        let index = tree.index.clone();
        let error = match tree.listing.get() {
            Some(Err(error)) => Some(error.to_string()),
            _ => None,
        };
        if let Some(error) = error {
            widgets::error(ui, &error);
            if ui.button("Try again").clicked() {
                self.relist_commit_tree();
            }
            return;
        }
        let Some(index) = index else {
            widgets::loading(ui, "Listing files…");
            return;
        };
        if index.truncated() {
            let text = format!("Showing the first {} of {} entries in this commit.", index.shown, index.total);
            ui.label(RichText::new(text).small().color(c.muted));
        }
        if index.shown == 0 {
            ui.label(RichText::new("This commit's tree has no files.").color(c.muted));
            return;
        }
        self.tree_rows(ui, &index.root, "", hash, viewing);
    }

    /// Switches the inspector between its changed files and the commit's whole tree.
    pub(crate) fn set_all_files(&mut self, all_files: bool) {
        if let Some(repo) = self.repo_mut() {
            repo.inspector_all_files = all_files;
        }
    }

    /// The folders and files in one folder of the tree. `prefix` is the folder's own path, empty
    /// at the top. Folders start closed, so a large tree opens only where the person looks.
    fn tree_rows(&mut self, ui: &mut Ui, folder: &TreeFolder, prefix: &str, hash: &str, viewing: Option<&str>) {
        let c = theme::of(ui);
        for (name, child) in &folder.folders {
            let path = if prefix.is_empty() { name.clone() } else { format!("{prefix}/{name}") };
            let id = ui.make_persistent_id(("commit-tree-folder", &path));
            let state = egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, false);
            // Clicking the row opens or closes the folder, and right-clicking it opens the menu.
            let clicked = std::cell::Cell::new(false);
            let mut header = state.show_header(ui, |ui| {
                let row = ui.horizontal(|ui| {
                    ui.label(RichText::new(format!("{}  {name}", icon::FOLDER)).color(c.muted));
                    ui.label(RichText::new(child.count().to_string()).small().color(c.muted));
                });
                let response = ui.interact(row.response.rect, id.with("menu"), egui::Sense::click());
                clicked.set(response.clicked());
                response.context_menu(|ui| {
                    ui.set_min_width(230.0);
                    if ui.button(format!("{}  Copy folder path", icon::COPY)).clicked() {
                        ui.close();
                        ui.ctx().copy_text(format!("{path}/"));
                    }
                });
            });
            if clicked.get() {
                header.toggle();
            }
            header.body(|ui| self.tree_rows(ui, child, &path, hash, viewing));
        }
        for entry in &folder.files {
            self.tree_file_row(ui, entry, hash, viewing);
        }
    }

    /// One file in the tree. Choosing it shows the file as it was at the commit.
    fn tree_file_row(&mut self, ui: &mut Ui, entry: &TreeEntry, hash: &str, viewing: Option<&str>) {
        let c = theme::of(ui);
        // A newline in a name is shown as an escape, so the row stays on one line.
        let name = entry.path.rsplit('/').next().unwrap_or(entry.path.as_str()).replace('\n', "\\n");
        let glyph = if entry.submodule { icon::GIT_BRANCH } else { icon::FILE };
        let selected = viewing == Some(entry.path.as_str());
        let response = ui
            .horizontal(|ui| {
                ui.label(RichText::new(glyph).color(c.muted));
                ui.add(egui::Button::selectable(selected, name).truncate())
            })
            .inner
            .on_hover_text(entry.path.as_str());
        if response.clicked() {
            self.select_commit_contents(hash.to_string(), entry.path.clone(), false);
        }
        response.context_menu(|ui| self.tree_file_menu(ui, hash, &entry.path, entry.submodule));
    }

    /// The menu for a file in the All files view or its file view. A submodule has no files of its
    /// own here, so only its path can be copied.
    pub(crate) fn tree_file_menu(&mut self, ui: &mut Ui, hash: &str, path: &str, submodule: bool) {
        ui.set_min_width(240.0);
        if !submodule {
            if ui.button(format!("{}  File history", icon::CLOCK_COUNTER_CLOCKWISE)).clicked() {
                ui.close();
                self.open_tool(Box::new(tools::file_history::FileHistoryWindow::new(path.to_string())));
            }
            if ui.button(format!("{}  Blame at this commit", icon::USER_LIST)).clicked() {
                ui.close();
                self.open_tool(Box::new(tools::blame::BlameWindow::new(path.to_string(), Some(hash.to_string()))));
            }
            if let Some(snapshot) = self.snapshot().cloned() {
                crate::git_ext::web_open_menu(
                    ui,
                    "Open file",
                    crate::git_ext::hosted_file_links(&snapshot, hash, path, self.settings.listed_hosts()),
                    crate::git_ext::PUSHED_COMMIT_NOTE,
                );
            }
            ui.separator();
            if ui.button(format!("{}  Save a copy…", icon::FLOPPY_DISK)).clicked() {
                ui.close();
                self.save_commit_file_copy(hash, path);
            }
        }
        if ui.button(format!("{}  Copy path", icon::COPY)).clicked() {
            ui.close();
            ui.ctx().copy_text(path.to_string());
        }
    }

    /// The file view: one file as it was at its commit, read-only, with its lines numbered. It
    /// takes the history's place, and History or Escape goes back to the commit.
    pub(crate) fn contents_panel(&mut self, ui: &mut Ui) {
        let c = theme::of(ui);
        let Some(Selection::CommitFile { hash, path }) = self.repo().map(|r| r.selection.clone()) else { return };
        let submodule = self.repos[self.active]
            .commit_contents
            .as_mut()
            .and_then(|task| task.get())
            .is_some_and(|result| matches!(result, Ok(FileView::Submodule(_))));
        egui::Frame::new().fill(ui.visuals().panel_fill).inner_margin(egui::Margin::symmetric(14, 7)).show(ui, |ui| {
            ui.horizontal(|ui| {
                if ui.button(format!("{}  History", icon::CARET_LEFT)).on_hover_text("Back to the commit history (Esc)").clicked() {
                    self.close_diff();
                }
                ui.add_space(6.0);
                ui.label(RichText::new(icon::FILE).color(c.muted));
                ui.add(egui::Label::new(RichText::new(&path).strong()).truncate()).on_hover_text(&path);
                ui.label(RichText::new(format!("in {}", short(&hash))).small().color(c.muted));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.menu_button(RichText::new(icon::DOTS_THREE).size(16.0), |ui| self.tree_file_menu(ui, &hash, &path, submodule));
                });
            });
        });
        ui.separator();
        let mut ask = None;
        let repo = &mut self.repos[self.active];
        let shown = repo.commit_contents.as_mut().and_then(|task| task.get());
        match shown {
            None => widgets::loading(ui, "Reading file…"),
            Some(Err(error)) => widgets::error(ui, &error.to_string()),
            Some(Ok(FileView::Submodule(commit))) => {
                let text = format!(
                    "This is a submodule. It records commit {}, but its files are in its own repository, so they are not shown here.",
                    short(commit)
                );
                egui::Frame::new().inner_margin(egui::Margin::symmetric(14, 10)).show(ui, |ui| {
                    ui.label(RichText::new(text).color(c.muted));
                });
            }
            Some(Ok(FileView::Binary(bytes))) => {
                egui::Frame::new().inner_margin(egui::Margin::symmetric(14, 10)).show(ui, |ui| {
                    ui.label(RichText::new(format!("Binary file, {bytes} bytes")).color(c.muted));
                    if ui.button(format!("{}  Save a copy…", icon::FLOPPY_DISK)).clicked() {
                        ask = Some(Ask::SaveCopy);
                    }
                });
            }
            Some(Ok(FileView::TooLarge(bytes))) => {
                egui::Frame::new().inner_margin(egui::Margin::symmetric(14, 10)).show(ui, |ui| {
                    let text = format!("This file is {}. Large files are not shown until you open them.", megabytes(*bytes));
                    ui.label(RichText::new(text).color(c.muted));
                    ui.add_space(6.0);
                    if ui.button("Open anyway").clicked() {
                        ask = Some(Ask::OpenAnyway);
                    }
                });
            }
            Some(Ok(FileView::Text(lines))) => text_lines(ui, lines, &c),
        }
        match ask {
            Some(Ask::OpenAnyway) => self.select_commit_contents(hash, path, true),
            Some(Ask::SaveCopy) => self.save_commit_file_copy(&hash, &path),
            None => {}
        }
    }
}

/// The lines of a text file, numbered, in a monospace view that scrolls both ways. Only the rows on
/// screen are laid out, so a long file stays quick.
fn text_lines(ui: &mut Ui, lines: &[String], c: &theme::Colors) {
    if lines.is_empty() {
        ui.add_space(8.0);
        ui.label(RichText::new("This file is empty.").color(c.muted));
        return;
    }
    let row_height = ui.text_style_height(&egui::TextStyle::Monospace);
    let digits = lines.len().to_string().len();
    egui::ScrollArea::both().id_salt("file-contents").auto_shrink(false).show_rows(ui, row_height, lines.len(), |ui, rows| {
        for index in rows {
            ui.horizontal(|ui| {
                ui.label(RichText::new(format!("{:>digits$}", index + 1)).monospace().color(c.muted));
                let text = lines[index].replace('\t', "    ");
                ui.add(egui::Label::new(RichText::new(text).monospace()).wrap_mode(egui::TextWrapMode::Extend));
            });
        }
    });
}

/// A size in megabytes, with one decimal place, as the file view names a large file.
fn megabytes(bytes: u64) -> String {
    format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(path: &str) -> TreeEntry {
        TreeEntry { path: path.to_string(), object: "a".repeat(40), submodule: false }
    }

    #[test]
    fn folders_hold_their_files_and_subfolders_whatever_the_order() {
        let listing = TreeListing {
            entries: vec![entry("src/lib.rs"), entry("README.md"), entry("src/deep/main.rs"), entry("src/two\nlines.txt")],
            total: 4,
        };
        let index = TreeIndex::new(&listing);
        assert_eq!(index.root.files.len(), 1);
        let src = &index.root.folders["src"];
        assert_eq!(src.files.len(), 2, "the files directly inside src");
        assert_eq!(src.folders["deep"].files.len(), 1);
        assert_eq!(index.root.count(), 4);
        assert!(!index.truncated());
    }

    #[test]
    fn a_truncated_listing_says_how_much_it_leaves_out() {
        let listing = TreeListing { entries: vec![entry("a.txt")], total: 20_001 };
        let index = TreeIndex::new(&listing);
        assert!(index.truncated());
        assert_eq!((index.shown, index.total), (1, 20_001));
    }

    #[test]
    fn sizes_read_in_megabytes() {
        assert_eq!(megabytes(2 * 1024 * 1024 + 512 * 1024), "2.5 MB");
    }
}
