#![allow(dead_code)]
//! Commit search: finds commits in every branch's history by message, author, code change, or
//! commit ID. Choosing a result selects that commit in the graph and inspector.

use std::path::Path;

use egui::{Key, RichText, Ui};
use egui_phosphor::regular as icon;
use nicegit_core::search::CommitSearchField;
use nicegit_core::Commit;

use crate::theme;
use crate::tools::file_history::result_row;
use crate::tools::widgets;
use crate::tools::{query, Ctx, Task, ToolWindow};

/// How many matches are shown. The newest matches are kept.
const LIMIT: usize = 200;

pub struct CommitSearchWindow {
    query: String,
    field: CommitSearchField,
    /// The latest search, once one has been started.
    results: Option<Task<nicegit_core::Result<Vec<Commit>>>>,
    focus_requested: bool,
    close: bool,
}

impl CommitSearchWindow {
    pub fn new() -> Self {
        Self { query: String::new(), field: CommitSearchField::Message, results: None, focus_requested: true, close: false }
    }

    /// Starts a search for the current text and field. Blank text starts nothing.
    fn search(&mut self, ctx: &egui::Context, repo: &Path) {
        let text = self.query.trim().to_string();
        if text.is_empty() {
            return;
        }
        let field = self.field;
        self.results = Some(query(ctx, repo, move |git, directory| git.search_commits(&text, field, LIMIT, directory)));
    }
}

impl Default for CommitSearchWindow {
    fn default() -> Self {
        Self::new()
    }
}

fn hint(field: CommitSearchField) -> &'static str {
    match field {
        CommitSearchField::Message => "Search commit messages",
        CommitSearchField::Author => "Search author names and emails",
        CommitSearchField::CodeChange => "Text added or removed in a file",
        CommitSearchField::CommitId => "Full or abbreviated commit ID",
    }
}

impl ToolWindow for CommitSearchWindow {
    fn id(&self) -> String {
        "commit-search".to_string()
    }

    fn title(&self) -> String {
        "Search commits".to_string()
    }

    fn default_size(&self) -> egui::Vec2 {
        egui::vec2(640.0, 500.0)
    }

    fn ui(&mut self, ui: &mut Ui, cx: &mut Ctx) {
        let field_before = self.field;
        let mut run = false;
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new(icon::MAGNIFYING_GLASS).size(16.0).color(theme::of(ui).muted));
            egui::ComboBox::from_id_salt("commit-search-field").width(140.0).selected_text(self.field.title()).show_ui(ui, |ui| {
                for field in CommitSearchField::ALL {
                    ui.selectable_value(&mut self.field, field, field.title());
                }
            });
            let response = ui.add(egui::TextEdit::singleline(&mut self.query).hint_text(hint(self.field)).desired_width(f32::INFINITY));
            if self.focus_requested {
                response.request_focus();
                self.focus_requested = false;
            }
            if response.lost_focus() && ui.input(|input| input.key_pressed(Key::Enter)) {
                run = true;
            }
        });
        ui.add_space(6.0);
        ui.separator();

        // Changing the field searches again when a search has already been shown.
        if self.field != field_before && self.results.is_some() {
            run = true;
        }
        if run {
            self.search(ui.ctx(), cx.repo);
        }

        let Self { results, close, .. } = self;
        match results.as_mut() {
            None => widgets::empty_state(
                ui,
                icon::MAGNIFYING_GLASS,
                "Press Return to search every branch, including history not yet loaded in the graph.",
            ),
            Some(task) => match task.get() {
                None => widgets::loading(ui, "Searching"),
                Some(Err(error)) => widgets::error(ui, &error.to_string()),
                Some(Ok(commits)) if commits.is_empty() => widgets::empty_state(ui, icon::MAGNIFYING_GLASS, "No commits found"),
                Some(Ok(commits)) => {
                    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                        if commits.len() == LIMIT {
                            let muted = theme::of(ui).muted;
                            ui.label(RichText::new(format!("Showing the newest {LIMIT} matches")).small().color(muted));
                        }
                        for commit in commits {
                            let detail = format!("{} · {} · {}", commit.author_name, commit.relative_date, commit.short_hash);
                            let response = result_row(ui, &commit.subject, &detail, None, None, false)
                                .on_hover_text(format!("Open {} in the inspector", commit.short_hash));
                            if response.clicked() {
                                cx.select_commit(commit.hash.clone());
                                *close = true;
                            }
                        }
                    });
                }
            },
        }
    }

    fn wants_close(&self) -> bool {
        self.close
    }
}
