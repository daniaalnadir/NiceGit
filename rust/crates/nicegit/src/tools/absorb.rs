//! Absorb staged changes: folds each staged change into the unpushed commit that last changed its
//! lines. The core traces the changes and rebuilds the commits after them; the working tree and the
//! index are not touched. Changes that cannot go anywhere stay staged, with the reason shown.

use std::collections::BTreeSet;
use std::path::Path;

use egui::RichText;
use egui_phosphor::regular as icon;
use nicegit_core::absorb::{apply_absorb, plan_absorb, AbsorbHunk, AbsorbPlan};
use nicegit_core::diff::{parse_diff, DiffLineKind};
use nicegit_core::models::short;
use nicegit_core::undo::UndoMode;
use nicegit_core::{GitClient, Snapshot};

use super::{query, widgets, Ctx, Task, ToolWindow};
use crate::theme;

/// How many lines of a change are shown before the rest are counted.
const SHOWN_LINES: usize = 12;

fn count_text(count: usize, one: &str, many: &str) -> String {
    if count == 1 {
        format!("1 {one}")
    } else {
        format!("{count} {many}")
    }
}

/// The notice after an absorb, such as "Absorbed 3 changes into 2 commits. 1 change stays staged."
fn absorbed_notice(absorbed: usize, commits: usize, remaining: usize) -> String {
    let mut text = format!("Absorbed {} into {}.", count_text(absorbed, "change", "changes"), count_text(commits, "commit", "commits"));
    if remaining > 0 {
        text.push_str(&format!(" {} stays staged.", count_text(remaining, "change", "changes")));
    }
    text
}

/// Absorbs staged changes into earlier commits, from the Changes panel or the command palette.
pub struct AbsorbWindow {
    branch: String,
    head: Option<String>,
    load: Option<Task<nicegit_core::Result<AbsorbPlan>>>,
    plan: Option<AbsorbPlan>,
    error: Option<String>,
    /// The commits whose changes will be absorbed. All start checked.
    checked: BTreeSet<String>,
    /// The commits whose changes are listed.
    expanded: BTreeSet<String>,
    closing: bool,
}

impl AbsorbWindow {
    /// The window for the checked-out branch. The branch and HEAD are captured now, so the absorb
    /// refuses if the checkout changes before it is confirmed.
    pub fn new(snapshot: &Snapshot) -> Self {
        Self {
            branch: snapshot.current_branch.clone(),
            head: snapshot.head_hash.clone(),
            load: None,
            plan: None,
            error: None,
            checked: BTreeSet::new(),
            expanded: BTreeSet::new(),
            closing: false,
        }
    }

    /// The changes that are checked, as commit hashes.
    fn accepted(&self) -> BTreeSet<String> {
        self.checked.clone()
    }
}

/// The lines of one change, coloured as a diff, after its `@@` line.
fn change_lines(ui: &mut egui::Ui, hunk: &AbsorbHunk) {
    let c = theme::of(ui);
    let body: Vec<_> = parse_diff(&hunk.patch).into_iter().skip_while(|line| line.kind != DiffLineKind::Hunk).skip(1).collect();
    for line in body.iter().filter(|line| matches!(line.kind, DiffLineKind::Addition | DiffLineKind::Deletion)).take(SHOWN_LINES) {
        let color = if line.kind == DiffLineKind::Addition { c.added } else { c.removed };
        ui.horizontal(|ui| {
            ui.add_space(22.0);
            ui.add(egui::Label::new(RichText::new(&line.text).monospace().small().color(color)).truncate());
        });
    }
    let hidden = body.iter().filter(|line| matches!(line.kind, DiffLineKind::Addition | DiffLineKind::Deletion)).count();
    if hidden > SHOWN_LINES {
        ui.horizontal(|ui| {
            ui.add_space(22.0);
            ui.label(RichText::new(format!("{} more lines", hidden - SHOWN_LINES)).small().color(c.muted));
        });
    }
}

/// The reason a block of changes was left out of the absorb, under its path.
fn unassigned_row(ui: &mut egui::Ui, hunk: &AbsorbHunk, reason: &str) {
    let c = theme::of(ui);
    ui.horizontal(|ui| {
        ui.add(egui::Label::new(RichText::new(&hunk.path).small()).truncate());
        if !hunk.header.is_empty() {
            ui.label(RichText::new(&hunk.header).small().monospace().color(c.muted));
        }
    });
    ui.horizontal(|ui| {
        ui.add_space(12.0);
        ui.label(RichText::new(reason).small().color(c.muted));
    });
}

impl ToolWindow for AbsorbWindow {
    fn id(&self) -> String {
        "absorb".to_string()
    }

    fn title(&self) -> String {
        "Absorb staged changes".to_string()
    }

    fn default_size(&self) -> egui::Vec2 {
        egui::vec2(640.0, 620.0)
    }

    fn ui(&mut self, ui: &mut egui::Ui, cx: &mut Ctx) {
        let c = theme::of(ui);
        if self.load.is_none() {
            self.load = Some(query(ui.ctx(), cx.repo, |client: &GitClient, repo: &Path| plan_absorb(client, repo)));
        }
        if self.plan.is_none() && self.error.is_none() {
            match self.load.as_mut().and_then(|task| task.get().cloned()) {
                Some(Ok(plan)) => {
                    self.checked = plan.targets.iter().map(|target| target.commit.clone()).collect();
                    self.plan = Some(plan);
                }
                Some(Err(error)) => self.error = Some(error.to_string()),
                None => {}
            }
        }

        ui.add_space(4.0);
        ui.label(
            RichText::new(format!(
                "{}  {}. Each change goes into the unpushed commit that last changed its lines. The commits after it are replayed.",
                icon::GIT_MERGE,
                self.branch
            ))
            .small()
            .color(c.muted),
        );
        ui.add_space(8.0);

        if let Some(error) = &self.error {
            widgets::error(ui, error);
            return;
        }
        let Some(plan) = self.plan.as_ref() else {
            widgets::loading(ui, "Tracing the staged changes");
            return;
        };

        widgets::section(ui, "Absorb into");
        ui.add_space(2.0);
        if plan.targets.is_empty() {
            ui.label(RichText::new("No staged change can go into an earlier commit.").small().color(c.muted));
        }
        egui::ScrollArea::vertical().id_salt("absorb_targets").max_height(320.0).auto_shrink([false, false]).show(ui, |ui| {
            for target in &plan.targets {
                ui.horizontal(|ui| {
                    let mut checked = self.checked.contains(&target.commit);
                    let label = format!("{}  {}", short(&target.commit), target.subject);
                    if widgets::checkbox(ui, cx.idle, &mut checked, label).changed() {
                        if checked {
                            self.checked.insert(target.commit.clone());
                        } else {
                            self.checked.remove(&target.commit);
                        }
                    }
                    let open = self.expanded.contains(&target.commit);
                    let glyph = if open { icon::CARET_DOWN } else { icon::CARET_RIGHT };
                    let text = format!("{glyph}  {}", count_text(target.hunks.len(), "change", "changes"));
                    if ui.small_button(text).on_hover_text(if open { "Hide the changes" } else { "Show the changes" }).clicked() {
                        if open {
                            self.expanded.remove(&target.commit);
                        } else {
                            self.expanded.insert(target.commit.clone());
                        }
                    }
                });
                if self.expanded.contains(&target.commit) {
                    for hunk in &target.hunks {
                        ui.horizontal(|ui| {
                            ui.add_space(22.0);
                            ui.label(RichText::new(format!("{}  {}", hunk.path, hunk.header)).small().monospace().color(c.muted));
                        });
                        change_lines(ui, hunk);
                    }
                }
            }
        });

        if !plan.unassigned.is_empty() {
            ui.add_space(8.0);
            widgets::section(ui, "Can't absorb");
            ui.add_space(2.0);
            egui::ScrollArea::vertical().id_salt("absorb_unassigned").max_height(160.0).auto_shrink([false, false]).show(ui, |ui| {
                for (hunk, reason) in &plan.unassigned {
                    unassigned_row(ui, hunk, reason);
                }
            });
        }

        // How many changes go where, for the person to check before confirming.
        let absorbed: usize = plan.targets.iter().filter(|t| self.checked.contains(&t.commit)).map(|t| t.hunks.len()).sum();
        let commits = plan.targets.iter().filter(|t| self.checked.contains(&t.commit)).count();
        let remaining =
            plan.targets.iter().filter(|t| !self.checked.contains(&t.commit)).map(|t| t.hunks.len()).sum::<usize>() + plan.unassigned.len();
        ui.add_space(6.0);
        ui.label(
            RichText::new(format!(
                "{} into {}. {} stays staged.",
                count_text(absorbed, "change", "changes"),
                count_text(commits, "commit", "commits"),
                count_text(remaining, "change", "changes")
            ))
            .small()
            .color(c.muted),
        );

        let reason = if !cx.idle {
            Some("Wait for the current Git action to finish")
        } else if self.head.is_none() {
            Some("Check out a branch first")
        } else if commits == 0 {
            Some("Check at least one commit")
        } else {
            None
        };

        ui.add_space(12.0);
        ui.separator();
        ui.add_space(8.0);
        let mut confirmed = false;
        ui.horizontal(|ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let button = widgets::primary_button(ui, "Absorb", reason.is_none());
                let button = match reason {
                    Some(reason) => button.on_disabled_hover_text(reason),
                    None => button.on_hover_text("Folds the checked changes into their commits"),
                };
                confirmed = button.clicked();
                if ui.button("Cancel").clicked() {
                    self.closing = true;
                }
                if !cx.idle {
                    ui.spinner();
                    ui.label(RichText::new("Waiting for the current action").small().color(c.muted));
                }
            });
        });

        if confirmed {
            // The plan must still be for the checkout the window was opened on; the core checks it again.
            if self.head.as_deref() == Some(plan.head.as_str()) && self.branch == plan.branch {
                let plan = plan.clone();
                let accepted = self.accepted();
                let detail = Some(count_text(commits, "commit", "commits"));
                cx.act_recording("Absorb staged changes", "Absorb staged changes", detail, UndoMode::Soft, move |client, repo| {
                    let absorbed = apply_absorb(client, &plan, &accepted, repo)?;
                    Ok(Some(absorbed_notice(absorbed, accepted.len(), remaining)))
                });
                self.closing = true;
            } else {
                cx.notice("The checkout changed. Open Absorb staged changes again.", true);
                self.closing = true;
            }
        }
    }

    fn wants_close(&self) -> bool {
        self.closing
    }
}
