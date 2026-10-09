//! Recover lost work: HEAD's reflog, with commits that no branch points to marked, and a way to
//! create a branch at any entry so the commit is kept.
#![allow(dead_code)]

use std::collections::HashSet;
use std::time::{SystemTime, UNIX_EPOCH};

use egui::{Color32, Margin, RichText, Sense, Ui};
use egui_phosphor::regular as icon;
use nicegit_core::reflog::ReflogEntry;
use nicegit_core::Snapshot;

use crate::theme;
use crate::tools::{query, widgets, Ctx, Task, ToolWindow};

/// How many entries are listed; older ones are rarely what anyone is looking for.
const ENTRY_LIMIT: usize = 300;

type Loaded = nicegit_core::Result<(Vec<ReflogEntry>, HashSet<String>)>;

/// A short description of when something happened, such as "3 days ago".
pub(crate) fn age_text(time: i64) -> String {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).map(|duration| duration.as_secs() as i64).unwrap_or(0);
    let elapsed = (now - time).max(0);
    let (value, unit) = match elapsed {
        0..=59 => return "just now".to_string(),
        60..=3_599 => (elapsed / 60, "minute"),
        3_600..=86_399 => (elapsed / 3_600, "hour"),
        86_400..=2_591_999 => (elapsed / 86_400, "day"),
        2_592_000..=31_535_999 => (elapsed / 2_592_000, "month"),
        _ => (elapsed / 31_536_000, "year"),
    };
    format!("{value} {unit}{} ago", if value == 1 { "" } else { "s" })
}

#[derive(Default)]
pub struct ReflogWindow {
    /// The reflog and the unreachable commits, read in the background.
    loading: Option<Task<Loaded>>,
    /// Set when the repository changed, so the reflog is read again on the next frame.
    needs_load: bool,
    entries: Vec<ReflogEntry>,
    /// Hashes of commits that no branch, tag, or remote branch contains.
    unreachable: HashSet<String>,
    error: Option<String>,
    /// The selected entry, by selector (`HEAD@{n}`), which is unique even when a commit repeats.
    selected: Option<String>,
    /// The entry a new branch is being created at, with its short hash for the dialog title.
    branch_target: Option<(String, String)>,
    branch_name: String,
}

impl ReflogWindow {
    pub fn new() -> Self {
        Self { needs_load: true, ..Self::default() }
    }

    /// The selected entry, if it is still listed.
    fn selected_entry(&self) -> Option<&ReflogEntry> {
        let selector = self.selected.as_deref()?;
        self.entries.iter().find(|entry| entry.selector == selector)
    }
}

impl ToolWindow for ReflogWindow {
    fn id(&self) -> String {
        "reflog".to_string()
    }

    fn title(&self) -> String {
        "Recover Lost Work".to_string()
    }

    fn default_size(&self) -> egui::Vec2 {
        egui::vec2(880.0, 560.0)
    }

    fn repository_changed(&mut self, _snapshot: &Snapshot) {
        self.needs_load = true;
    }

    fn ui(&mut self, ui: &mut Ui, cx: &mut Ctx) {
        if self.needs_load && self.loading.is_none() {
            self.needs_load = false;
            self.loading = Some(query(ui.ctx(), cx.repo, |client, directory| {
                let entries = client.reflog(ENTRY_LIMIT, directory)?;
                let hashes: Vec<String> = entries.iter().map(|entry| entry.hash.clone()).collect();
                let unreachable = client.unreachable_commits(&hashes, directory)?;
                Ok((entries, unreachable))
            }));
        }
        if let Some(result) = self.loading.as_mut().and_then(|task| task.get().cloned()) {
            self.loading = None;
            match result {
                Ok((entries, unreachable)) => {
                    self.entries = entries;
                    self.unreachable = unreachable;
                    self.error = None;
                    // Select the newest entry that would otherwise be lost, unless the selection is still listed.
                    let still_listed = self.selected_entry().is_some();
                    if !still_listed {
                        let first = self
                            .entries
                            .iter()
                            .find(|entry| self.unreachable.contains(&entry.hash))
                            .or(self.entries.first())
                            .map(|entry| (entry.selector.clone(), entry.hash.clone()));
                        if let Some((selector, hash)) = first {
                            self.selected = Some(selector);
                            cx.select_commit(hash);
                        }
                    }
                }
                Err(error) => self.error = Some(error.to_string()),
            }
        }

        let c = theme::of(ui);
        ui.horizontal(|ui| {
            ui.label(
                RichText::new("Every position HEAD has had, including commits that no branch points to any more.").small().color(c.muted),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if widgets::icon_button(ui, icon::ARROW_CLOCKWISE, "Read the reflog again", self.loading.is_none()).clicked() {
                    self.needs_load = true;
                }
            });
        });
        ui.add_space(6.0);

        if self.loading.is_some() {
            widgets::loading(ui, "Reading the reflog…");
        } else if let Some(error) = &self.error {
            widgets::error(ui, error);
        }

        let idle = cx.idle;
        ui.columns(2, |columns| {
            self.list(&mut columns[0], cx);
            self.detail(&mut columns[1], cx, idle);
        });

        self.branch_dialog(ui, cx, idle);
    }
}

impl ReflogWindow {
    fn list(&mut self, ui: &mut Ui, cx: &mut Ctx) {
        let c = theme::of(ui);
        if self.entries.is_empty() && self.loading.is_none() && self.error.is_none() {
            widgets::empty_state(ui, icon::CLOCK_COUNTER_CLOCKWISE, "No history yet.");
            return;
        }
        egui::ScrollArea::vertical().id_salt("reflog_entries").auto_shrink([false, false]).show(ui, |ui| {
            for index in 0..self.entries.len() {
                let entry = &self.entries[index];
                let is_selected = self.selected.as_deref() == Some(entry.selector.as_str());
                let is_unreachable = self.unreachable.contains(&entry.hash);
                let fill = if is_selected { ui.visuals().selection.bg_fill } else { Color32::TRANSPARENT };
                let response = egui::Frame::new()
                    .fill(fill)
                    .corner_radius(6.0)
                    .inner_margin(Margin::symmetric(8, 6))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.horizontal(|ui| {
                            ui.add(egui::Label::new(RichText::new(&entry.subject).strong()).truncate());
                            if is_unreachable {
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    widgets::pill(ui, "Not on any branch", c.conflict)
                                        .on_hover_text("Only the reflog keeps this commit. Git may delete it after the reflog expires.");
                                });
                            }
                        });
                        ui.add(egui::Label::new(RichText::new(&entry.action).small().color(c.muted)).truncate());
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(&entry.selector).monospace().small().color(c.muted));
                            ui.label(RichText::new(short(&entry.hash)).monospace().small().color(c.muted));
                            if let Some(time) = entry.time {
                                ui.label(RichText::new(age_text(time)).small().color(c.muted));
                            }
                        });
                    })
                    .response
                    .interact(Sense::click());
                if response.clicked() {
                    self.selected = Some(entry.selector.clone());
                    cx.select_commit(entry.hash.clone());
                }
                let hash = entry.hash.clone();
                let short_hash = short(&entry.hash).to_string();
                response.context_menu(|ui| {
                    if ui.add_enabled(cx.idle, egui::Button::new(format!("{}  Create branch here…", icon::GIT_BRANCH))).clicked() {
                        self.branch_target = Some((hash.clone(), short_hash.clone()));
                        self.branch_name.clear();
                        ui.close();
                    }
                    if ui.button(format!("{}  Copy commit hash", icon::COPY)).clicked() {
                        ui.ctx().copy_text(hash.clone());
                        ui.close();
                    }
                });
                ui.add_space(2.0);
            }
        });
    }

    fn detail(&mut self, ui: &mut Ui, cx: &mut Ctx, idle: bool) {
        let c = theme::of(ui);
        let Some(entry) = self.selected_entry().cloned() else {
            widgets::empty_state(ui, icon::ARROW_U_UP_LEFT, "Select an entry to see its commit.");
            return;
        };
        egui::ScrollArea::vertical().id_salt("reflog_detail").auto_shrink([false, false]).show(ui, |ui| {
            ui.add(egui::Label::new(RichText::new(&entry.subject).strong().size(15.0)).wrap());
            ui.add_space(4.0);
            ui.label(RichText::new(&entry.hash).monospace().color(c.muted));
            ui.add_space(8.0);
            ui.label(RichText::new(&entry.action).color(c.muted));
            ui.label(
                RichText::new(format!("{}  ·  {}", entry.selector, entry.time.map(age_text).unwrap_or_default())).small().color(c.muted),
            );
            ui.add_space(10.0);
            if self.unreachable.contains(&entry.hash) {
                widgets::callout(
                    ui,
                    "Only the reflog keeps this commit. Git may delete it when the reflog expires. Create a branch to keep it.",
                    true,
                );
                ui.add_space(8.0);
            }
            ui.horizontal_wrapped(|ui| {
                if widgets::primary_button(ui, &format!("{}  Create branch here…", icon::GIT_BRANCH), idle).clicked() {
                    self.branch_target = Some((entry.hash.clone(), short(&entry.hash).to_string()));
                    self.branch_name.clear();
                }
                if widgets::labeled_button(ui, icon::GIT_COMMIT, "Show in graph", true).clicked() {
                    cx.select_commit(entry.hash.clone());
                }
                if widgets::labeled_button(ui, icon::COPY, "Copy hash", true).clicked() {
                    ui.ctx().copy_text(entry.hash.clone());
                }
            });
        });
    }

    fn branch_dialog(&mut self, ui: &mut Ui, cx: &mut Ctx, idle: bool) {
        let Some((hash, short_hash)) = self.branch_target.clone() else { return };
        let mut open = true;
        let mut close = false;
        let title = format!("Create branch at {short_hash}");
        egui::Window::new(title)
            .id(ui.id().with("create_branch"))
            .collapsible(false)
            .resizable(false)
            .default_width(360.0)
            .open(&mut open)
            .show(ui.ctx(), |ui| {
                ui.label(RichText::new("The branch keeps this commit and its history. Your current checkout does not change.").small());
                ui.add_space(6.0);
                widgets::text_field(ui, &mut self.branch_name, "Branch name")
                    .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, "Branch name"));
                ui.add_space(8.0);
                let name = self.branch_name.trim().to_string();
                ui.horizontal(|ui| {
                    if ui.button("Cancel").clicked() {
                        close = true;
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if widgets::primary_button(ui, "Create branch", idle && !name.is_empty()).clicked() {
                            let label = format!("Create branch {name}");
                            let created = name.clone();
                            cx.act(label, move |client, directory| {
                                client.create_branch_at(&created, &hash, directory).map(|()| Some(format!("Created branch {created}.")))
                            });
                            close = true;
                        }
                    });
                });
            });
        if !open || close {
            self.branch_target = None;
        }
    }
}

fn short(hash: &str) -> &str {
    &hash[..hash.len().min(8)]
}
