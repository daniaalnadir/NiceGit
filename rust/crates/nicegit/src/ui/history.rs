use std::collections::{HashMap, HashSet};

use egui::{Color32, FontId, RichText, Sense, Ui};
use egui_phosphor::regular as icon;
use nicegit_core::{Branch, Commit, Snapshot};

use crate::app::{NiceGitApp, Selection};
use crate::graph_view::{self, Node};
use crate::theme;
use crate::tools::{self, widgets};
use crate::ui::dialogs::{Dialog, InputKind, Pending};

const ROW_HEIGHT: f32 = 30.0;
const LABEL_COLUMN: f32 = 168.0;

#[derive(Clone, Copy, PartialEq)]
enum LabelKind {
    Head,
    Local,
    Remote,
    /// A local branch shown together with the remote branch it tracks, when both point here.
    Both,
    Tag,
}

struct Label {
    text: String,
    kind: LabelKind,
    branch: Option<Branch>,
}

/// The branch and tag labels for a commit. A local branch and its upstream on the same commit
/// share one label; the pairing compares full ref names, since display names are ambiguous.
fn labels(commit: &Commit, snapshot: &Snapshot) -> Vec<Label> {
    let locals: HashMap<&str, &Branch> = snapshot.branches.iter().filter(|b| !b.is_remote).map(|b| (b.name.as_str(), b)).collect();
    let remotes: HashMap<String, &Branch> = snapshot.branches.iter().filter(|b| b.is_remote).map(|b| (b.display_name(), b)).collect();
    let mut out: Vec<Label> = Vec::new();
    let mut paired: HashSet<String> = HashSet::new();
    let mut remote_labels = Vec::new();
    for reference in &commit.refs {
        if let Some(tag) = reference.strip_prefix("tag: ") {
            out.push(Label { text: tag.to_string(), kind: LabelKind::Tag, branch: None });
        } else if let Some(name) = reference.strip_prefix("HEAD -> ") {
            let branch = locals.get(name).map(|b| (*b).clone());
            let upstream_here = branch.as_ref().and_then(|b| b.upstream.clone()).and_then(|upstream| {
                remotes.values().find(|r| format!("refs/{}", r.name) == upstream && r.tip == commit.hash).map(|r| r.display_name())
            });
            if let Some(remote) = &upstream_here {
                paired.insert(remote.clone());
            }
            out.insert(
                0,
                Label { text: name.to_string(), kind: if upstream_here.is_some() { LabelKind::Both } else { LabelKind::Head }, branch },
            );
        } else if reference == "HEAD" {
            out.insert(0, Label { text: "HEAD".into(), kind: LabelKind::Head, branch: None });
        } else if let Some(local) = locals.get(reference.as_str()) {
            let upstream_here = local.upstream.clone().and_then(|upstream| {
                remotes.values().find(|r| format!("refs/{}", r.name) == upstream && r.tip == commit.hash).map(|r| r.display_name())
            });
            if let Some(remote) = &upstream_here {
                paired.insert(remote.clone());
            }
            out.push(Label {
                text: reference.clone(),
                kind: if upstream_here.is_some() { LabelKind::Both } else { LabelKind::Local },
                branch: Some((*local).clone()),
            });
        } else if let Some(remote) = remotes.get(reference.as_str()) {
            remote_labels.push(Label { text: reference.clone(), kind: LabelKind::Remote, branch: Some((*remote).clone()) });
        }
        // Anything else, such as a remote's symbolic HEAD, is not shown.
    }
    out.extend(remote_labels.into_iter().filter(|l| !paired.contains(&l.text)));
    out
}

fn label_icons(kind: LabelKind) -> &'static str {
    match kind {
        LabelKind::Head => icon::CHECK,
        LabelKind::Local => icon::LAPTOP,
        LabelKind::Remote => icon::CLOUD,
        LabelKind::Both => icon::LAPTOP,
        LabelKind::Tag => icon::TAG,
    }
}

/// Lines up a row's text with a mid-height baseline.
fn text_at(painter: &egui::Painter, x: f32, y: f32, text: impl ToString, font: FontId, color: Color32) -> egui::Rect {
    painter.text(egui::pos2(x, y), egui::Align2::LEFT_CENTER, text.to_string(), font, color)
}

impl NiceGitApp {
    pub fn history(&mut self, ui: &mut Ui) {
        let Some(repo) = self.repo() else { return };
        let Some(snapshot) = repo.snapshot.clone() else { return };
        let c = theme::of(ui);
        let filter = repo.commit_filter.trim().to_lowercase();
        let marked = repo.marked.clone();
        let total_commits = snapshot.commits.len();

        egui::Frame::new().inner_margin(egui::Margin { left: 16, right: 16, top: 10, bottom: 6 }).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("Commit history").size(16.0).strong());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let more = if snapshot.has_more_commits { "+" } else { "" };
                    ui.label(RichText::new(format!("{total_commits}{more} commits")).small().monospace().color(c.muted));
                });
            });
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                let search_width = 170.0;
                egui::Frame::new().fill(ui.visuals().extreme_bg_color).corner_radius(6.0).inner_margin(egui::Margin::symmetric(8, 4)).show(
                    ui,
                    |ui| {
                        ui.set_width(ui.available_width() - search_width - 16.0);
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(icon::FUNNEL_SIMPLE).color(c.muted));
                            let filter = &mut self.repos[self.active].commit_filter;
                            ui.add(
                                egui::TextEdit::singleline(filter)
                                    .hint_text("Filter loaded commits by message, author, ID, or branch")
                                    .frame(egui::Frame::NONE)
                                    .desired_width(f32::INFINITY),
                            );
                        });
                    },
                );
                if ui
                    .add_sized([search_width, 28.0], egui::Button::new(format!("{}  Search all history", icon::MAGNIFYING_GLASS)))
                    .on_hover_text("Shift-Ctrl/Cmd-F")
                    .clicked()
                {
                    self.open_tool(Box::new(tools::commit_search::CommitSearchWindow::new()));
                }
            });
            if !marked.is_empty() {
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.label(RichText::new(format!("{}  {} commits selected", icon::CHECK_SQUARE, marked.len())).color(c.accent));
                    let can = self.idle() && snapshot.operation.is_none() && snapshot.is_on_branch() && snapshot.status.is_empty();
                    if widgets::primary_button(ui, "Cherry-pick", can)
                        .on_disabled_hover_text("Commit or stash your changes first")
                        .clicked()
                    {
                        self.confirm(
                            format!("Cherry-pick {} commits?", marked.len()),
                            format!(
                                "They are applied to {} oldest first. Conflicts stop the sequence for you to resolve or abort.",
                                snapshot.current_branch
                            ),
                            "Cherry-pick",
                            Pending::CherryPick {
                                commits: marked.iter().cloned().collect(),
                                branch: snapshot.current_branch.clone(),
                                head: snapshot.head_hash.clone(),
                            },
                        );
                    }
                    if ui.button("Clear").clicked() {
                        self.repos[self.active].marked.clear();
                    }
                });
            }
        });

        // Column headers.
        let graph_width = graph_view::width_for(self.repo().map(|r| r.lanes).unwrap_or(1)).clamp(44.0, 240.0);
        ui.horizontal(|ui| {
            ui.add_space(16.0);
            let header = |ui: &mut Ui, text: &str, width: f32| {
                ui.add_sized([width, 18.0], egui::Label::new(RichText::new(text).small().color(c.muted)));
            };
            header(ui, "Branch / tag", LABEL_COLUMN - 8.0);
            header(ui, "Graph", graph_width);
            ui.label(RichText::new("Commit message").small().color(c.muted));
        });

        let dirty = !snapshot.status.is_empty();
        let rows: Vec<usize> = if filter.is_empty() {
            (0..self.repo().map(|r| r.rows.len()).unwrap_or(0)).collect()
        } else {
            let repo = self.repo().expect("active repository");
            (0..repo.rows.len())
                .filter(|&index| {
                    repo.commit_at(index).is_some_and(|commit| {
                        commit.subject.to_lowercase().contains(&filter)
                            || commit.author_name.to_lowercase().contains(&filter)
                            || commit.hash.starts_with(&filter)
                            || commit.refs.iter().any(|r| r.to_lowercase().contains(&filter))
                    })
                })
                .collect()
        };
        let show_more = snapshot.has_more_commits && filter.is_empty();
        let count = rows.len() + usize::from(show_more);
        let background = ui.visuals().window_fill;

        // Scroll the selected row into view when it changed by keyboard or from elsewhere.
        let mut scroll_area = egui::ScrollArea::vertical().id_salt("history").auto_shrink(false);
        if self.repo().is_some_and(|r| r.scroll_to_selection) {
            let selected_index = self.repo().and_then(|repo| match &repo.selection {
                Selection::Commit { hash, .. } => rows.iter().position(|&i| repo.commit_at(i).is_some_and(|c| &c.hash == hash)),
                Selection::WorkingTree | Selection::Change { .. } if dirty => Some(0),
                _ => None,
            });
            let view_id = ui.make_persistent_id("history_view");
            let (offset, height): (f32, f32) = ui.data(|d| d.get_temp(view_id)).unwrap_or((0.0, 400.0));
            if let Some(index) = selected_index {
                let y = index as f32 * ROW_HEIGHT;
                if y < offset || y + ROW_HEIGHT > offset + height {
                    scroll_area = scroll_area.vertical_scroll_offset((y - height / 2.0).max(0.0));
                }
            }
            self.repos[self.active].scroll_to_selection = false;
        }

        let output = scroll_area.show_rows(ui, ROW_HEIGHT, count, |ui, range| {
            ui.spacing_mut().item_spacing.y = 0.0;
            for position in range {
                let Some(&index) = rows.get(position) else {
                    ui.horizontal(|ui| {
                        ui.add_space(LABEL_COLUMN + graph_width);
                        if ui
                            .add_enabled(self.idle(), egui::Button::new(format!("{}  Load older history", icon::CARET_DOUBLE_DOWN)))
                            .clicked()
                        {
                            self.repos[self.active].history_limit += crate::app::PAGE;
                            self.load(false);
                        }
                    });
                    continue;
                };
                self.history_row(ui, index, &snapshot, graph_width, background, filter.is_empty(), &marked);
            }
        });
        let view_id = ui.make_persistent_id("history_view");
        ui.data_mut(|d| d.insert_temp(view_id, (output.state.offset.y, output.inner_rect.height())));
    }

    #[allow(clippy::too_many_arguments)]
    fn history_row(
        &mut self,
        ui: &mut Ui,
        index: usize,
        snapshot: &Snapshot,
        graph_width: f32,
        background: Color32,
        draw_graph: bool,
        marked: &std::collections::BTreeSet<String>,
    ) {
        let c = theme::of(ui);
        let Some(repo) = self.repo() else { return };
        let Some(row) = repo.rows.get(index).cloned() else { return };
        let dirty = repo.dirty();
        let is_working_tree = dirty && index == 0;
        let commit = repo.commit_at(index).cloned();
        let selection = repo.selection.clone();
        let width = ui.available_width();
        let (rect, response) = ui.allocate_exact_size(egui::vec2(width, ROW_HEIGHT), Sense::click());
        // The row is painted, so describe it for screen readers and UI tests.
        let description = match &commit {
            Some(commit) => commit.subject.clone(),
            None => "Working tree".to_string(),
        };
        response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::SelectableLabel, true, &description));
        if !ui.is_rect_visible(rect) {
            return;
        }
        let lane_color = theme::graph_color(row.color);
        let selected = match (&selection, &commit) {
            (Selection::Commit { hash, .. }, Some(commit)) => *hash == commit.hash,
            (Selection::WorkingTree | Selection::Change { .. }, None) => true,
            _ => false,
        };
        let is_marked = commit.as_ref().is_some_and(|c| marked.contains(&c.hash));
        let painter = ui.painter().with_clip_rect(rect);
        let message_left = rect.left() + LABEL_COLUMN + graph_width;
        let message_rect = egui::Rect::from_min_max(egui::pos2(message_left - 4.0, rect.top()), rect.max);
        // A band in the line's colour, stronger when selected.
        let alpha = if selected || is_marked {
            58
        } else if response.hovered() {
            30
        } else {
            14
        };
        painter.rect_filled(message_rect, 0.0, Color32::from_rgba_unmultiplied(lane_color.r(), lane_color.g(), lane_color.b(), alpha));
        painter.rect_filled(egui::Rect::from_min_size(message_rect.min, egui::vec2(3.0, ROW_HEIGHT)), 0.0, lane_color);

        let graph_rect = egui::Rect::from_min_size(egui::pos2(rect.left() + LABEL_COLUMN, rect.top()), egui::vec2(graph_width, ROW_HEIGHT));
        let initials = commit.as_ref().map(|c| graph_view::initials(&c.author_name)).unwrap_or_default();
        let node = if is_working_tree {
            Node::WorkingTree
        } else {
            Node::Commit { initials: &initials, merge: commit.as_ref().is_some_and(|c| c.parents.len() > 1) }
        };
        if draw_graph {
            graph_view::paint_row(&ui.painter().with_clip_rect(graph_rect), graph_rect, &row, node, background);
        } else {
            let single = nicegit_core::graph::GraphRow { lane: 0, lane_count: 1, segments: vec![], color: row.color, line: row.line };
            graph_view::paint_row(&ui.painter().with_clip_rect(graph_rect), graph_rect, &single, node, background);
        }

        let y = rect.center().y;
        let body = FontId::proportional(13.5);
        let small = FontId::proportional(11.5);
        let mono = FontId::monospace(11.5);
        let text = ui.visuals().text_color();
        let message_width = rect.right() - message_left;
        let show_meta = message_width > 460.0;
        let meta_width = if show_meta { 250.0_f32.min(message_width * 0.42) } else { 70.0 };
        let subject_right = rect.right() - meta_width - 8.0;

        if is_working_tree {
            let count = snapshot.status.len();
            text_at(&painter, rect.left() + 18.0, y, format!("{}  Working tree", icon::PENCIL_SIMPLE), body.clone(), c.accent);
            let subject = format!("{count} uncommitted file{}", if count == 1 { "" } else { "s" });
            let clip = egui::Rect::from_min_max(egui::pos2(message_left, rect.top()), egui::pos2(subject_right, rect.bottom()));
            text_at(&painter.with_clip_rect(clip), message_left + 10.0, y, subject, FontId::proportional(13.5), text);
            if show_meta {
                painter.text(egui::pos2(rect.right() - 12.0, y), egui::Align2::RIGHT_CENTER, "Not committed yet", small, c.muted);
            }
            if response.clicked() {
                self.select_working_tree();
            }
            return;
        }
        let Some(commit) = commit else { return };

        // Branch and tag labels, joined to the node by a line in the lane colour.
        let all_labels = labels(&commit, snapshot);
        let mut draggable: Vec<(egui::Rect, Branch)> = Vec::new();
        if !all_labels.is_empty() {
            let mut x = rect.left() + 10.0;
            let label_limit = rect.left() + LABEL_COLUMN - 8.0;
            let shown = all_labels.len().min(2);
            let mut last_right = x;
            for (i, label) in all_labels.iter().take(shown).enumerate() {
                let remaining = all_labels.len() - shown;
                let suffix = if i == shown - 1 && remaining > 0 { format!(" +{remaining}") } else { String::new() };
                let glyph = label_icons(label.kind);
                let cloud = if label.kind == LabelKind::Both { format!(" {}", icon::CLOUD) } else { String::new() };
                let available = (label_limit - x - 6.0).max(30.0);
                let full = format!("{} {}{cloud}{suffix}", if label.kind == LabelKind::Tag { glyph } else { "" }, label.text);
                let galley =
                    painter.layout_no_wrap(truncate_to(&painter, &full, available - 12.0, small.clone()), small.clone(), Color32::WHITE);
                let pill = egui::Rect::from_min_size(egui::pos2(x, y - 10.0), egui::vec2(galley.size().x + 12.0, 20.0));
                let fill = match label.kind {
                    LabelKind::Tag => c.tag,
                    _ => shade(lane_color, 0.78),
                };
                painter.rect_filled(pill, 5.0, fill);
                if label.kind == LabelKind::Head || label.kind == LabelKind::Both && label.branch.as_ref().is_some_and(|b| b.is_current) {
                    painter.rect_stroke(pill, 5.0, egui::Stroke::new(1.5, Color32::WHITE.gamma_multiply(0.85)), egui::StrokeKind::Inside);
                }
                painter.galley(egui::pos2(pill.left() + 6.0, y - galley.size().y / 2.0), galley, Color32::WHITE);
                if let Some(branch) = label.branch.as_ref().filter(|b| !b.is_current && b.name != commit.hash) {
                    draggable.push((pill, branch.clone()));
                }
                let _ = glyph;
                last_right = pill.right();
                x = pill.right() + 4.0;
                if x > label_limit - 30.0 {
                    break;
                }
            }
            let node_x = graph_view::lane_x(graph_rect, if draw_graph { row.lane } else { 0 }) - graph_view::NODE_RADIUS - 2.0;
            if node_x > last_right + 2.0 {
                painter.line_segment(
                    [egui::pos2(last_right + 2.0, y), egui::pos2(node_x, y)],
                    egui::Stroke::new(1.5, lane_color.gamma_multiply(0.8)),
                );
            }
        }

        // Subject, "You are here", author, date, and ID.
        let is_head = snapshot.head_hash.as_deref() == Some(commit.hash.as_str());
        let subject_rect = egui::Rect::from_min_max(egui::pos2(message_left + 10.0, rect.top()), egui::pos2(subject_right, rect.bottom()));
        let subject_painter = painter.with_clip_rect(subject_rect);
        let subject_font = if is_head { FontId::new(13.5, egui::FontFamily::Proportional) } else { body.clone() };
        let drawn = text_at(&subject_painter, subject_rect.left(), y, &commit.subject, subject_font, text);
        if is_head && drawn.right() + 90.0 < subject_right {
            text_at(&painter, drawn.right() + 10.0, y, "You are here", FontId::proportional(11.0), c.accent);
        }
        let meta = if show_meta { format!("{} · {}", commit.author_name, commit.relative_date) } else { String::new() };
        let meta_rect = egui::Rect::from_min_max(egui::pos2(subject_right, rect.top()), egui::pos2(rect.right() - 78.0, rect.bottom()));
        painter.with_clip_rect(meta_rect).text(egui::pos2(meta_rect.right() - 6.0, y), egui::Align2::RIGHT_CENTER, meta, small, c.muted);
        painter.text(egui::pos2(rect.right() - 12.0, y), egui::Align2::RIGHT_CENTER, &commit.short_hash, mono, c.muted);

        // Drag a branch label onto the current commit to merge it in or rebase onto it.
        for (index, (pill, branch)) in draggable.into_iter().enumerate() {
            let label = ui.interact(pill, ui.id().with(("label", &commit.hash, index)), Sense::drag());
            label.dnd_set_drag_payload(branch);
        }
        if is_head && snapshot.is_on_branch() {
            if let Some(source) = response.dnd_release_payload::<Branch>() {
                if self.idle() && snapshot.operation.is_none() && !source.is_current {
                    self.dialog = Some(Dialog::Integrate { source: (*source).clone() });
                }
            }
        }
        let response = response.on_hover_text_at_pointer(format!("{}\n{} <{}>", commit.subject, commit.author_name, commit.author_email));
        if response.clicked() {
            let toggle = ui.input(|i| i.modifiers.command);
            if toggle {
                let repo = &mut self.repos[self.active];
                if !repo.marked.remove(&commit.hash) {
                    repo.marked.insert(commit.hash.clone());
                }
            } else {
                self.repos[self.active].marked.clear();
                self.select_commit(commit.hash.clone());
            }
        }
        // Drop a branch label onto the current branch's commit to merge or rebase.
        let can_switch = self.idle() && snapshot.operation.is_none();
        response.context_menu(|ui| self.commit_menu(ui, &commit, snapshot, can_switch));
    }

    pub fn commit_menu(&mut self, ui: &mut Ui, commit: &Commit, snapshot: &Snapshot, can_switch: bool) {
        let idle = self.idle();
        let on_branch = snapshot.is_on_branch();
        let clean = snapshot.status.is_empty() && snapshot.operation.is_none();
        let is_head = snapshot.head_hash.as_deref() == Some(commit.hash.as_str());
        ui.set_min_width(260.0);
        for branch in snapshot.branches.iter().filter(|b| b.tip == commit.hash && !b.is_current && !b.is_detached()) {
            if ui.add_enabled(can_switch, egui::Button::new(format!("{}  Check out {}", icon::CHECK, branch.display_name()))).clicked() {
                ui.close();
                self.checkout(branch.clone());
            }
        }
        if ui.add_enabled(idle, egui::Button::new(format!("{}  Create branch here…", icon::GIT_BRANCH))).clicked() {
            ui.close();
            let source = Branch {
                name: commit.hash.clone(),
                is_current: false,
                is_remote: false,
                tip: commit.hash.clone(),
                subject: commit.subject.clone(),
                upstream: None,
            };
            self.dialog = Some(Dialog::input(
                "New branch",
                &format!("Create a branch at {}", commit.short_hash),
                "",
                InputKind::CreateBranchFrom(source),
            ));
        }
        if ui.add_enabled(idle, egui::Button::new(format!("{}  Create tag here…", icon::TAG))).clicked() {
            ui.close();
            self.dialog = Some(Dialog::with_second(
                "New tag",
                "Tag name",
                "Message (optional; makes an annotated tag)",
                InputKind::CreateTag { target: commit.hash.clone() },
            ));
        }
        ui.separator();
        let can_rewrite = idle && on_branch && clean;
        // Cherry-picking or reverting a merge needs the parent its changes are measured against.
        let merge_parents =
            (commit.parents.len() > 1).then(|| commit.parents.iter().map(|p| (p.clone(), String::new())).collect::<Vec<_>>());
        let choose_parent = |revert: bool| Dialog::ChooseParent {
            commit: commit.hash.clone(),
            subject: commit.subject.clone(),
            parents: merge_parents.clone().unwrap_or_default(),
            selected: 0,
            revert,
            branch: snapshot.current_branch.clone(),
            head: snapshot.head_hash.clone(),
        };
        if !is_head
            && ui
                .add_enabled(can_rewrite, egui::Button::new(format!("{}  Cherry-pick", icon::ARROW_BEND_DOWN_RIGHT)))
                .on_disabled_hover_text("Commit or stash your changes first")
                .clicked()
        {
            ui.close();
            if merge_parents.is_some() {
                self.dialog = Some(choose_parent(false));
                return;
            }
            self.confirm(
                "Cherry-pick this commit?",
                format!("“{}” is applied to {} as a new commit.", commit.subject, snapshot.current_branch),
                "Cherry-pick",
                Pending::CherryPick {
                    commits: vec![commit.hash.clone()],
                    branch: snapshot.current_branch.clone(),
                    head: snapshot.head_hash.clone(),
                },
            );
        }
        if ui
            .add_enabled(can_rewrite, egui::Button::new(format!("{}  Revert", icon::ARROW_U_UP_LEFT)))
            .on_disabled_hover_text("Commit or stash your changes first")
            .clicked()
        {
            ui.close();
            if merge_parents.is_some() {
                self.dialog = Some(choose_parent(true));
                return;
            }
            self.confirm(
                "Revert this commit?",
                format!("A new commit on {} undoes the changes from “{}”.", snapshot.current_branch, commit.subject),
                "Revert",
                Pending::Revert { commit: commit.hash.clone(), branch: snapshot.current_branch.clone(), head: snapshot.head_hash.clone() },
            );
        }
        if !is_head
            && ui
                .add_enabled(
                    idle && on_branch && snapshot.operation.is_none(),
                    egui::Button::new(format!("{}  Reset {} to here…", icon::ARROW_ARC_LEFT, snapshot.current_branch)),
                )
                .clicked()
        {
            ui.close();
            self.open_tool(Box::new(tools::reset::ResetWindow::new(commit.hash.clone(), commit.subject.clone(), snapshot)));
        }
        if !is_head
            && ui
                .add_enabled(can_rewrite, egui::Button::new(format!("{}  Interactive rebase from here…", icon::LIST_NUMBERS)))
                .on_hover_text("Reorder, edit, squash, or drop the commits after this one")
                .clicked()
        {
            ui.close();
            self.open_tool(Box::new(tools::interactive_rebase::InteractiveRebaseWindow::new(commit.hash.clone(), snapshot)));
        }
        if is_head
            && ui
                .add_enabled(
                    idle && on_branch && snapshot.operation.is_none(),
                    egui::Button::new(format!("{}  Edit message…", icon::PENCIL_SIMPLE)),
                )
                .clicked()
        {
            ui.close();
            if let Some(head) = snapshot.head_hash.clone() {
                let message = self
                    .repo()
                    .and_then(|r| nicegit_core::GitClient::new().commit_message(&head, &r.path).ok())
                    .unwrap_or_else(|| commit.subject.clone());
                self.dialog =
                    Some(Dialog::EditMessage { message: message.trim_end().to_string(), branch: snapshot.current_branch.clone(), head });
            }
        }
        if is_head
            && ui
                .add_enabled(
                    idle && on_branch && snapshot.operation.is_none(),
                    egui::Button::new(format!("{}  Undo this commit…", icon::ARROW_COUNTER_CLOCKWISE)),
                )
                .clicked()
        {
            ui.close();
            if let Some(head) = snapshot.head_hash.clone() {
                self.confirm(
                    "Undo the last commit?",
                    format!(
                        "“{}” is removed from {}. Its changes stay staged so you can commit them again.",
                        commit.subject, snapshot.current_branch
                    ),
                    "Undo commit",
                    Pending::UndoCommit { branch: snapshot.current_branch.clone(), head },
                );
            }
        }
        ui.separator();
        if ui.button(format!("{}  Compare with working files", icon::GIT_DIFF)).clicked() {
            ui.close();
            self.open_tool(Box::new(tools::compare::CompareWindow::new(commit.hash.clone(), None)));
        }
        let base = self.repo().and_then(|r| r.compare_base.clone());
        match base {
            Some(base) if base != commit.hash => {
                if ui.button(format!("{}  Compare with marked commit", icon::ARROWS_LEFT_RIGHT)).clicked() {
                    ui.close();
                    self.open_tool(Box::new(tools::compare::CompareWindow::new(base, Some(commit.hash.clone()))));
                    self.repos[self.active].compare_base = None;
                }
            }
            _ => {
                if ui.button(format!("{}  Mark for comparison", icon::PUSH_PIN)).clicked() {
                    ui.close();
                    self.repos[self.active].compare_base = Some(commit.hash.clone());
                    self.notify(format!("Marked {}. Right-click another commit to compare.", commit.short_hash), false);
                }
            }
        }
        if ui.button(format!("{}  Search files in this commit…", icon::FILE_MAGNIFYING_GLASS)).clicked() {
            ui.close();
            self.open_tool(Box::new(tools::content_search::ContentSearchWindow::new(Some(commit.hash.clone()))));
        }
        if snapshot.operation.is_none()
            && ui.add_enabled(idle && clean, egui::Button::new(format!("{}  Start bisect: this commit is good", icon::BUG))).clicked()
        {
            ui.close();
            self.open_tool(Box::new(tools::bisect::BisectWindow::start_from(commit.hash.clone())));
        }
        ui.separator();
        if ui.button(format!("{}  Copy commit ID", icon::HASH)).clicked() {
            ui.close();
            ui.ctx().copy_text(commit.hash.clone());
        }
        crate::git_ext::github_link_menu(ui, snapshot, &commit.hash);
        if ui.button(format!("{}  Copy subject", icon::COPY)).clicked() {
            ui.close();
            ui.ctx().copy_text(commit.subject.clone());
        }
        if ui.button(format!("{}  Save as patch…", icon::FLOPPY_DISK)).clicked() {
            ui.close();
            self.export_patch(commit);
        }
    }

    fn export_patch(&mut self, commit: &Commit) {
        let Some(repo) = self.repo() else { return };
        let name = format!("{}.patch", commit.short_hash);
        if let Some(file) = rfd::FileDialog::new().set_file_name(&name).save_file() {
            match nicegit_core::GitClient::new().export_commit_patch(&commit.hash, &repo.path) {
                Ok(patch) => match std::fs::write(&file, patch) {
                    Ok(()) => self.notify(format!("Saved {}.", file.display()), false),
                    Err(error) => self.notify(error.to_string(), true),
                },
                Err(error) => self.notify(error.to_string(), true),
            }
        }
    }
}

/// Shortens `text` with an ellipsis so it fits in `width` points.
fn truncate_to(painter: &egui::Painter, text: &str, width: f32, font: FontId) -> String {
    let fits = |t: &str| painter.layout_no_wrap(t.to_string(), font.clone(), Color32::WHITE).size().x <= width;
    if fits(text) {
        return text.to_string();
    }
    let chars: Vec<char> = text.chars().collect();
    let mut end = chars.len();
    while end > 1 {
        end -= 1;
        let candidate: String = chars[..end].iter().collect::<String>() + "…";
        if fits(&candidate) {
            return candidate;
        }
    }
    "…".into()
}

/// An opaque, darker version of `color`, so white label text stays readable on it.
fn shade(color: Color32, factor: f32) -> Color32 {
    let scale = |v: u8| (v as f32 * factor).round() as u8;
    Color32::from_rgb(scale(color.r()), scale(color.g()), scale(color.b()))
}
