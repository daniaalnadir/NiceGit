#![allow(dead_code)]
//! Bisect controls: start a bisect from a known good commit, mark the commit under test, and
//! show the first bad commit once Git has found it.

use std::path::Path;

use egui::{Color32, RichText, Ui};
use egui_phosphor::regular as icon;
use nicegit_core::bisect::{BisectMark, BisectStatus};
use nicegit_core::{GitClient, GitError, Snapshot};

use crate::theme;
use crate::tools::{query, widgets, Ctx, Task, ToolWindow};

/// The bisect as read from Git, with the subjects of the commits it names.
struct Loaded {
    status: Option<BisectStatus>,
    testing_subject: Option<String>,
    first_bad_subject: Option<String>,
}

pub struct BisectWindow {
    /// The good commit typed or passed in, for starting a bisect.
    good: String,
    loaded: Refreshing<(), Loaded>,
    stale: bool,
}

impl BisectWindow {
    /// A window for a bisect; shows how to start one when none is running.
    pub fn new() -> Self {
        Self { good: String::new(), loaded: Refreshing::new(), stale: true }
    }

    /// A window that starts a bisect from `good`, with that commit already filled in.
    pub fn start_from(good: String) -> Self {
        Self { good, ..Self::new() }
    }
}

impl Default for BisectWindow {
    fn default() -> Self {
        Self::new()
    }
}

impl ToolWindow for BisectWindow {
    fn id(&self) -> String {
        "bisect".to_string()
    }

    fn title(&self) -> String {
        "Bisect".to_string()
    }

    fn default_size(&self) -> egui::Vec2 {
        egui::vec2(580.0, 380.0)
    }

    fn repository_changed(&mut self, _snapshot: &Snapshot) {
        self.stale = true;
    }

    fn ui(&mut self, ui: &mut Ui, cx: &mut Ctx) {
        if self.stale {
            self.stale = false;
            let task = query(ui.ctx(), cx.repo, load_bisect);
            self.loaded.start((), task);
        }
        self.loaded.settle();

        let snapshot: &Snapshot = cx.snapshot;
        let idle = cx.idle;
        ui.add_space(4.0);
        match self.loaded.value(&()) {
            None => widgets::loading(ui, "Reading the bisect status"),
            Some(Err(error)) => widgets::callout(ui, &message(error), true),
            Some(Ok(loaded)) => match &loaded.status {
                None => start_view(ui, cx, snapshot, idle, &mut self.good),
                Some(status) => running_view(ui, cx, idle, status, loaded),
            },
        }
    }
}

/// Reads the bisect in progress, if any, and the subjects of the commits it names.
fn load_bisect(client: &GitClient, directory: &Path) -> nicegit_core::Result<Loaded> {
    let status = client.bisect_status(directory)?;
    let subject = |hash: Option<&str>| {
        hash.and_then(|hash| client.commit_message(hash, directory).ok())
            .map(|message| message.lines().next().unwrap_or("").to_string())
            .filter(|line| !line.is_empty())
    };
    let (testing_subject, first_bad_subject) = match &status {
        Some(status) => (subject(status.testing.as_deref()), subject(status.first_bad.as_deref())),
        None => (None, None),
    };
    Ok(Loaded { status, testing_subject, first_bad_subject })
}

/// The view when no bisect is running: explains it and starts one from a good commit, testing
/// from the current commit, which is taken as bad.
fn start_view(ui: &mut Ui, cx: &mut Ctx, snapshot: &Snapshot, idle: bool, good: &mut String) {
    let c = theme::of(ui);
    widgets::section(ui, "Start a bisect");
    ui.label("Bisect finds the commit that introduced a problem. Give a commit that is known to be good; NiceGit then tests the commits between it and the current commit.");
    ui.add_space(10.0);

    ui.horizontal(|ui| {
        ui.label(RichText::new("Bad").strong());
        match &snapshot.head_hash {
            Some(head) => {
                widgets::hash_label(ui, head);
                ui.label(RichText::new("current commit").color(c.muted));
            }
            None => {
                ui.label(RichText::new("This repository has no commits yet.").color(c.muted));
            }
        }
    });
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.label(RichText::new("Good").strong());
        ui.add(egui::TextEdit::singleline(good).hint_text("Commit, branch, or tag known to work").desired_width(ui.available_width()));
    });
    ui.add_space(10.0);

    if snapshot.operation.is_some() {
        widgets::callout(ui, "Finish the current Git operation before starting a bisect.", true);
        ui.add_space(6.0);
    }
    let head = snapshot.head_hash.clone();
    let good_text = good.trim().to_string();
    let can_start = idle && head.is_some() && snapshot.operation.is_none() && !good_text.is_empty();
    ui.horizontal(|ui| {
        if widgets::primary_button(ui, "Start bisect", can_start).clicked() {
            if let Some(bad) = head {
                let branch = snapshot.current_branch.clone();
                cx.act("Start bisect", move |client, directory| {
                    client.start_bisect(&bad, &good_text, &branch, Some(&bad), directory)?;
                    Ok(Some("Bisect started. Mark the checked-out commit as good or bad.".to_string()))
                });
            }
        }
        ui.label(RichText::new("Checkouts happen during a bisect, so commit or stash changes first.").small().color(c.muted));
    });
}

/// The view while a bisect runs: the commit under test and its verdict buttons, or the first
/// bad commit once Git has found it.
fn running_view(ui: &mut Ui, cx: &mut Ctx, idle: bool, status: &BisectStatus, loaded: &Loaded) {
    let c = theme::of(ui);
    match &status.first_bad {
        Some(first_bad) => {
            widgets::section(ui, "First bad commit");
            ui.horizontal(|ui| {
                ui.label(RichText::new(icon::TARGET).color(c.danger).size(18.0));
                widgets::hash_label(ui, first_bad);
            });
            ui.label(RichText::new(loaded.first_bad_subject.as_deref().unwrap_or("Subject unavailable")).strong());
            ui.add_space(4.0);
            ui.label(RichText::new("Git narrowed the search to this commit. Select it in the graph to inspect it.").color(c.muted));
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                if ui.button(format!("{}  Select commit", icon::GIT_COMMIT)).clicked() {
                    cx.select_commit(first_bad.clone());
                }
                if ui.add_enabled(idle, egui::Button::new(format!("{}  End bisect", icon::STOP))).clicked() {
                    cx.act("End bisect", |client, directory| {
                        client.end_bisect(directory)?;
                        Ok(Some("Bisect ended. Returned to the starting checkout.".to_string()))
                    });
                }
            });
        }
        None => {
            widgets::section(ui, "Testing");
            match &status.testing {
                Some(testing) => {
                    ui.horizontal(|ui| {
                        widgets::hash_label(ui, testing);
                        if let Some(subject) = &loaded.testing_subject {
                            ui.label(RichText::new(subject).strong());
                        }
                    });
                }
                None => {
                    ui.label(RichText::new("No commit is checked out for testing.").color(c.muted));
                }
            }
            ui.label(RichText::new(steps_text(status.remaining_steps)).color(c.muted).small());
            ui.add_space(4.0);
            ui.label(
                RichText::new(format!(
                    "{} good, {} bad, {} skipped so far. Returns to {} when ended.",
                    status.good.len(),
                    usize::from(status.bad.is_some()),
                    status.skipped.len(),
                    checkout_name(&status.original_checkout),
                ))
                .small()
                .color(c.muted),
            );
            ui.add_space(12.0);
            let testing = status.testing.clone();
            ui.horizontal(|ui| {
                let can_mark = idle && testing.is_some();
                mark_button(
                    ui,
                    cx,
                    can_mark,
                    testing.as_deref(),
                    BisectMark::Good,
                    icon::CHECK_CIRCLE,
                    "This commit does not have the problem",
                );
                mark_button(ui, cx, can_mark, testing.as_deref(), BisectMark::Bad, icon::X_CIRCLE, "This commit has the problem");
                mark_button(ui, cx, can_mark, testing.as_deref(), BisectMark::Skip, icon::SKIP_FORWARD, "This commit cannot be tested");
                ui.add_space(12.0);
                if ui
                    .add_enabled(idle, egui::Button::new(format!("{}  End bisect", icon::STOP)))
                    .on_hover_text(format!("Return to {}", checkout_name(&status.original_checkout)))
                    .clicked()
                {
                    cx.act("End bisect", |client, directory| {
                        client.end_bisect(directory)?;
                        Ok(Some("Bisect ended. Returned to the starting checkout.".to_string()))
                    });
                }
            });
        }
    }
}

/// A verdict button. It marks the commit under test and reports Git's progress.
fn mark_button(ui: &mut Ui, cx: &mut Ctx, enabled: bool, testing: Option<&str>, mark: BisectMark, glyph: &str, tooltip: &str) {
    let label = match mark {
        BisectMark::Good => "Good",
        BisectMark::Bad => "Bad",
        BisectMark::Skip => "Skip",
    };
    let text = format!("{glyph}  {label}");
    let response = match mark {
        BisectMark::Bad => {
            let c = theme::of(ui);
            ui.add_enabled(enabled, egui::Button::new(RichText::new(text).color(Color32::WHITE)).fill(c.danger))
        }
        _ => ui.add_enabled(enabled, egui::Button::new(text)),
    };
    if response.on_hover_text(tooltip).on_disabled_hover_text("Wait for the current action to finish").clicked() {
        if let Some(testing) = testing {
            let testing = testing.to_string();
            cx.act(format!("Mark {label}"), move |client, directory| {
                let output = client.mark_bisect(mark, &testing, directory)?;
                Ok(output.lines().next().map(str::to_string).filter(|line| !line.is_empty()))
            });
        }
    }
}

fn steps_text(remaining: Option<usize>) -> String {
    match remaining {
        Some(0) => "One more mark should find the first bad commit.".to_string(),
        Some(1) => "About 1 more mark after this one.".to_string(),
        Some(count) => format!("About {count} more marks after this one."),
        None => "Mark this commit good or bad.".to_string(),
    }
}

fn checkout_name(original: &str) -> &str {
    if original.is_empty() {
        "the starting checkout"
    } else {
        original
    }
}

/// The message to show for a Git error, without the command prefix.
fn message(error: &GitError) -> String {
    match error {
        GitError::CommandFailed { message, .. } => message.clone(),
        other => other.to_string(),
    }
}

/// The latest answer to a background query. A refresh keeps the previous answer on screen until
/// the new one arrives, so the window does not flicker while it reloads.
struct Refreshing<K, T> {
    shown: Option<(K, Task<nicegit_core::Result<T>>)>,
    pending: Option<(K, Task<nicegit_core::Result<T>>)>,
}

impl<K: PartialEq, T: Send + 'static> Refreshing<K, T> {
    fn new() -> Self {
        Self { shown: None, pending: None }
    }

    /// Replaces any request still in flight; only the newest answer is kept.
    fn start(&mut self, key: K, task: Task<nicegit_core::Result<T>>) {
        self.pending = Some((key, task));
    }

    /// Moves a finished request into place. Returns true when a new answer arrived.
    fn settle(&mut self) -> bool {
        let finished = self.pending.as_mut().is_some_and(|(_, task)| !task.is_pending());
        if finished {
            self.shown = self.pending.take();
        }
        finished
    }

    /// The answer for `key`, once it has arrived.
    fn value(&mut self, key: &K) -> Option<&nicegit_core::Result<T>> {
        match &mut self.shown {
            Some((shown, task)) if shown == key => task.get(),
            _ => None,
        }
    }
}
