#![allow(dead_code)]
//! Content search: finds lines containing some text across tracked files, at a commit or in the
//! working files. Each match can open blame at its line.

use crate::i18n::{tr, tr_count, tr_fmt};
use std::collections::HashSet;

use egui::{Key, Margin, RichText, Ui};
use egui_phosphor::regular as icon;
use nicegit_core::search::ContentMatch;

use crate::theme;
use crate::tools::blame::{short_revision, BlameWindow};
use crate::tools::widgets;
use crate::tools::{query, Ctx, Task, ToolWindow};

/// How many matching lines are shown.
const LIMIT: usize = 1000;

pub struct ContentSearchWindow {
    /// The commit searched, or `None` for the working files.
    revision: Option<String>,
    /// What is searched, as shown above the results.
    label: String,
    query: String,
    match_case: bool,
    results: Option<Task<nicegit_core::Result<Vec<ContentMatch>>>>,
    focus_requested: bool,
}

impl ContentSearchWindow {
    /// Searches the files at `revision`, or the working files when it is `None`.
    pub fn new(revision: Option<String>) -> Self {
        let label = match &revision {
            Some(revision) => tr_fmt!("commit {revision}", revision = short_revision(revision)),
            None => tr("working files").to_string(),
        };
        Self { revision, label, query: String::new(), match_case: false, results: None, focus_requested: true }
    }

    fn search(&mut self, ctx: &egui::Context, repo: &std::path::Path) {
        if self.query.is_empty() {
            return;
        }
        let text = self.query.clone();
        let revision = self.revision.clone();
        let ignore_case = !self.match_case;
        self.results =
            Some(query(ctx, repo, move |git, directory| git.search_contents(&text, revision.as_deref(), ignore_case, LIMIT, directory)));
    }
}

/// Consecutive matches in the same file, in order. Git lists each file's matches together.
fn group(matches: &[ContentMatch]) -> Vec<(&str, Vec<&ContentMatch>)> {
    let mut groups: Vec<(&str, Vec<&ContentMatch>)> = Vec::new();
    for found in matches {
        match groups.last_mut() {
            Some((path, items)) if *path == found.path.as_str() => items.push(found),
            _ => groups.push((found.path.as_str(), vec![found])),
        }
    }
    groups
}

impl ToolWindow for ContentSearchWindow {
    fn id(&self) -> String {
        format!("content-search\0{}", self.revision.as_deref().unwrap_or(""))
    }

    fn title(&self) -> String {
        tr_fmt!("Find in {label}", label = self.label)
    }

    fn default_size(&self) -> egui::Vec2 {
        egui::vec2(760.0, 560.0)
    }

    fn ui(&mut self, ui: &mut Ui, cx: &mut Ctx) {
        let mut run = false;
        let mut match_case_changed = false;
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new(icon::FILE_TEXT).size(16.0).color(theme::of(ui).muted));
            let response = ui.add(
                egui::TextEdit::singleline(&mut self.query)
                    .hint_text(tr("Text in files"))
                    .desired_width((ui.available_width() - 150.0).max(160.0)),
            );
            response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::TextInput, true, tr("Text in files")));
            if self.focus_requested {
                response.request_focus();
                self.focus_requested = false;
            }
            if response.lost_focus() && ui.input(|input| input.key_pressed(Key::Enter)) {
                run = true;
            }
            if crate::tools::widgets::checkbox(ui, true, &mut self.match_case, tr("Match case")).changed() {
                match_case_changed = true;
            }
        });
        ui.add_space(4.0);

        // A match-case change searches again once a search has been shown.
        if match_case_changed && self.results.is_some() {
            run = true;
        }
        if run {
            self.search(ui.ctx(), cx.repo);
        }

        let muted = theme::of(ui).muted;
        let Self { revision, label, results, .. } = self;
        ui.horizontal(|ui| {
            ui.label(RichText::new(tr_fmt!("Searching {label}", label = label)).small().color(muted));
        });
        ui.separator();

        match results.as_mut() {
            None => widgets::empty_state(ui, icon::FILE_TEXT, tr("Press Return to search every tracked text file.")),
            Some(task) => match task.get() {
                None => widgets::loading(ui, tr("Searching")),
                Some(Err(error)) => widgets::error(ui, &error.to_string()),
                Some(Ok(matches)) if matches.is_empty() => widgets::empty_state(ui, icon::FILE_TEXT, tr("No matches")),
                Some(Ok(matches)) => {
                    let files = matches.iter().map(|found| found.path.as_str()).collect::<HashSet<_>>().len();
                    let summary = if matches.len() == LIMIT {
                        tr_fmt!("First {limit} matches", limit = LIMIT)
                    } else {
                        tr_fmt!(
                            "{matches} in {files}",
                            matches = tr_count("{count} match", "{count} matches", matches.len()),
                            files = tr_count("{count} file", "{count} files", files),
                        )
                    };
                    ui.label(RichText::new(summary).small().monospace().color(muted));
                    ui.add_space(4.0);
                    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                        for (path, items) in group(matches) {
                            egui::Frame::new().fill(theme::of(ui).subtle_bg).inner_margin(Margin::symmetric(12, 6)).show(ui, |ui| {
                                ui.set_width(ui.available_width());
                                ui.label(RichText::new(path).monospace().strong());
                            });
                            for found in items {
                                match_row(ui, cx, revision.as_deref(), found);
                            }
                            ui.add_space(6.0);
                        }
                    });
                }
            },
        }
    }
}

/// One matching line, with a button that opens blame at it.
fn match_row(ui: &mut Ui, cx: &mut Ctx, revision: Option<&str>, found: &ContentMatch) {
    let c = theme::of(ui);
    ui.horizontal(|ui| {
        ui.add_space(12.0);
        ui.add_sized([48.0, 18.0], egui::Label::new(RichText::new(found.line.to_string()).monospace().color(c.muted)));
        if ui.small_button(format!("{}  {}", icon::USER, tr("Blame"))).on_hover_text(tr("Show who last changed this line")).clicked() {
            cx.open(Box::new(BlameWindow::new(found.path.clone(), revision.map(str::to_string)).focused_on(found.line)));
        }
        ui.add_space(8.0);
        ui.add(egui::Label::new(RichText::new(found.text.trim()).monospace()).truncate());
    });
}
