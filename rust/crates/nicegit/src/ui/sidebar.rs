use std::path::PathBuf;

use egui::{RichText, Ui};
use egui_phosphor::regular as icon;
use nicegit_core::models::short;
use nicegit_core::{Branch, GitClient, Snapshot};

use crate::app::{NiceGitApp, Selection};
use crate::theme;
use crate::tools::{self, widgets};
use crate::ui::dialogs::{Dialog, InputKind, Pending};

/// A collapsible sidebar section: icon, uppercase title, and a count.
pub(crate) fn section<R>(ui: &mut Ui, id: &str, glyph: &str, title: &str, count: usize, open: bool, add: impl FnOnce(&mut Ui) -> R) {
    let c = theme::of(ui);
    let id = ui.make_persistent_id(id);
    let mut state = egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, open);
    let header = ui.horizontal(|ui| {
        let caret = if state.is_open() { icon::CARET_DOWN } else { icon::CARET_RIGHT };
        let response = ui.add(
            egui::Button::new(RichText::new(format!("{caret}  {glyph}  {}", title.to_uppercase())).small().strong().color(c.muted))
                .frame(false),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add_space(6.0);
            // An empty section has nothing to highlight, so its zero is muted.
            let count_color = if count == 0 { c.muted } else { c.accent };
            ui.label(RichText::new(count.to_string()).small().monospace().color(count_color));
        });
        response
    });
    if header.inner.clicked() {
        state.toggle(ui);
    }
    state.show_body_unindented(ui, |ui| {
        ui.add_space(2.0);
        add(ui);
        ui.add_space(4.0);
    });
    ui.add_space(2.0);
    ui.separator();
}

/// A sidebar row: icon, label, optional trailing text. Returns the row's response.
fn row(ui: &mut Ui, glyph: &str, glyph_color: egui::Color32, text: RichText, trailing: Option<RichText>, selected: bool) -> egui::Response {
    let c = theme::of(ui);
    let height = 26.0;
    let (rect, response) = ui.allocate_exact_size(egui::vec2(ui.available_width(), height), egui::Sense::click_and_drag());
    if selected {
        ui.painter().rect_filled(rect.shrink2(egui::vec2(6.0, 1.0)), 6.0, c.card_bg);
    } else if response.hovered() {
        ui.painter().rect_filled(rect.shrink2(egui::vec2(6.0, 1.0)), 6.0, ui.visuals().widgets.hovered.weak_bg_fill);
    }
    let mut child = ui.new_child(
        egui::UiBuilder::new().max_rect(rect.shrink2(egui::vec2(14.0, 0.0))).layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    child.label(RichText::new(glyph).color(glyph_color));
    if let Some(trailing) = trailing {
        child.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(trailing);
            ui.add(egui::Label::new(text).truncate().selectable(false));
        });
    } else {
        child.add(egui::Label::new(text).truncate().selectable(false));
    }
    response
}

impl NiceGitApp {
    pub fn sidebar(&mut self, ui: &mut Ui) {
        let Some(snapshot) = self.snapshot().cloned() else { return };
        let c = theme::of(ui);
        egui::Panel::bottom("sidebar_footer").frame(egui::Frame::new().inner_margin(egui::Margin::symmetric(14, 6))).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("NiceGit").small().strong().color(c.muted));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if widgets::icon_button(ui, icon::GEAR, "Settings", true).clicked() {
                        self.show_settings = true;
                    }
                    if !self.settings.show_repositories
                        && widgets::icon_button(ui, icon::SIDEBAR_SIMPLE, "Show repositories", true).clicked()
                    {
                        self.settings.show_repositories = true;
                    }
                });
            });
        });
        egui::Frame::new().inner_margin(egui::Margin { left: 8, right: 8, top: 12, bottom: 0 }).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.add_space(6.0);
                ui.label(RichText::new(icon::CUBE).size(16.0).color(c.accent));
                ui.menu_button(
                    RichText::new(format!("{} {}", snapshot.name, icon::CARET_DOWN)).strong().size(15.0).color(c.accent),
                    |ui| self.repository_menu(ui),
                );
            });
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.add_space(6.0);
                let idle = self.idle();
                let filter = &mut self.repos[self.active].branch_filter;
                ui.label(RichText::new(icon::MAGNIFYING_GLASS).color(c.muted));
                ui.add(
                    egui::TextEdit::singleline(filter)
                        .hint_text("Filter references")
                        .frame(egui::Frame::NONE)
                        .desired_width((ui.available_width() - 34.0).max(40.0)),
                )
                .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, "Filter references"));
                if widgets::icon_button(ui, icon::PLUS, "New branch at HEAD", idle && snapshot.head_hash.is_some()).clicked() {
                    self.dialog = Some(Dialog::input(
                        "New branch",
                        "Create a branch at HEAD and switch to it",
                        "",
                        InputKind::CreateBranch { branch: snapshot.current_branch.clone(), head: snapshot.head_hash.clone() },
                    ));
                }
            });
            ui.separator();
            egui::ScrollArea::vertical().id_salt("sidebar").auto_shrink(false).show(ui, |ui| self.sidebar_sections(ui, &snapshot));
        });
    }

    fn sidebar_sections(&mut self, ui: &mut Ui, snapshot: &Snapshot) {
        let c = theme::of(ui);
        let idle = self.idle();
        let can_switch = idle && snapshot.operation.is_none();
        let filter = self.repo().map(|r| r.branch_filter.to_lowercase()).unwrap_or_default();
        let matches = |text: &str| filter.is_empty() || text.to_lowercase().contains(&filter);
        let other_worktrees = GitClient::branches_in_other_worktrees(snapshot);

        let locals: Vec<Branch> = snapshot.branches.iter().filter(|b| !b.is_remote && matches(&b.name)).cloned().collect();
        section(ui, "local", icon::LAPTOP, "Local", locals.len(), true, |ui| {
            if locals.is_empty() {
                ui.label(RichText::new("   No local branches").small().color(c.muted));
            }
            for branch in &locals {
                let in_other = other_worktrees.contains(&branch.name);
                let (glyph, color) = if branch.is_current { (icon::CHECK_CIRCLE, c.accent) } else { (icon::GIT_BRANCH, c.muted) };
                let mut text = RichText::new(branch.display_name());
                if branch.is_current {
                    text = text.strong().color(c.accent);
                }
                let trailing = if branch.is_current {
                    match (snapshot.ahead, snapshot.behind) {
                        (Some(a), Some(b)) if a + b > 0 => Some(RichText::new(format!("↑{a} ↓{b}")).small().monospace().color(c.muted)),
                        _ => None,
                    }
                } else if in_other {
                    Some(RichText::new(icon::FOLDERS).color(c.muted))
                } else {
                    None
                };
                let response = row(ui, glyph, color, text, trailing, false);
                let response = response.on_hover_text(format!(
                    "{}  {}{}",
                    short(&branch.tip),
                    branch.subject,
                    if in_other { "\nChecked out in another worktree" } else { "" }
                ));
                // Drag a branch onto the current branch to merge it in or rebase onto it.
                if !branch.is_current {
                    response.dnd_set_drag_payload(branch.clone());
                } else if let Some(source) = response.dnd_release_payload::<Branch>() {
                    if can_switch {
                        self.dialog = Some(Dialog::Integrate { source: (*source).clone() });
                    }
                }
                if response.double_clicked() && !branch.is_current && can_switch && !in_other {
                    self.checkout(branch.clone());
                }
                if response.clicked() {
                    self.select_commit(branch.tip.clone());
                }
                response.context_menu(|ui| self.branch_menu(ui, branch, snapshot, can_switch && !in_other));
            }
        });

        let remote_count = snapshot.branches.iter().filter(|b| b.is_remote).count();
        section(ui, "remote", icon::GLOBE, "Remote", remote_count, true, |ui| {
            if snapshot.remotes.is_empty() {
                ui.horizontal(|ui| {
                    ui.add_space(14.0);
                    if ui.small_button("Add a remote…").clicked() {
                        self.open_tool(Box::new(tools::repository_settings::RepositorySettingsWindow::new()));
                    }
                });
            }
            for remote in &snapshot.remotes {
                let id = ui.make_persistent_id(("remote", remote));
                egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, !filter.is_empty())
                    .show_header(ui, |ui| {
                        ui.label(RichText::new(format!("{}  {remote}", icon::CLOUD)).strong());
                    })
                    .body_unindented(|ui| {
                        for branch in snapshot.branches.iter().filter(|b| b.remote_name(&snapshot.remotes) == Some(remote.as_str())) {
                            let display = branch.display_name();
                            let name = display.strip_prefix(&format!("{remote}/")).unwrap_or(&display).to_string();
                            if !matches(&display) {
                                continue;
                            }
                            let response = row(ui, icon::GIT_BRANCH, c.remote_branch, RichText::new(name), None, false)
                                .on_hover_text(format!("{}  {}", short(&branch.tip), branch.subject));
                            response.dnd_set_drag_payload(branch.clone());
                            if response.double_clicked() && can_switch {
                                self.checkout(branch.clone());
                            }
                            if response.clicked() {
                                self.select_commit(branch.tip.clone());
                            }
                            response.context_menu(|ui| self.branch_menu(ui, branch, snapshot, can_switch));
                        }
                    });
            }
        });

        let worktrees: Vec<&nicegit_core::Worktree> =
            snapshot.worktrees.iter().filter(|w| matches(&w.path) || w.branch.as_deref().is_some_and(matches)).collect();
        {
            section(ui, "worktrees", icon::FOLDERS, "Worktrees", worktrees.len(), true, |ui| {
                for worktree in worktrees {
                    let current = worktree.path == snapshot.root_path;
                    let label = worktree.branch.clone().unwrap_or_else(|| "Detached HEAD".into());
                    let response = row(
                        ui,
                        if current { icon::FOLDER_OPEN } else { icon::FOLDER },
                        if current { c.accent } else { c.muted },
                        RichText::new(label),
                        worktree.is_prunable.then(|| RichText::new("missing").small().color(c.warning)),
                        current,
                    )
                    .on_hover_text(&worktree.path);
                    if response.clicked() && !current && idle && !worktree.is_prunable {
                        self.open(PathBuf::from(&worktree.path));
                    }
                    response.context_menu(|ui| {
                        if ui
                            .add_enabled(!current && !worktree.is_prunable, egui::Button::new(format!("{}  Open", icon::FOLDER_OPEN)))
                            .clicked()
                        {
                            ui.close();
                            self.open(PathBuf::from(&worktree.path));
                        }
                        if ui.button(format!("{}  Copy path", icon::COPY)).clicked() {
                            ui.close();
                            ui.ctx().copy_text(worktree.path.clone());
                        }
                        if ui
                            .add_enabled(!worktree.is_prunable, egui::Button::new(format!("{}  Show in file manager", icon::FOLDER)))
                            .clicked()
                        {
                            ui.close();
                            crate::ui::changes::reveal(std::path::Path::new(&worktree.path));
                        }
                        if ui.button(format!("{}  Manage worktrees…", icon::GEAR)).clicked() {
                            ui.close();
                            self.open_tool(Box::new(tools::worktrees::WorktreesWindow::new()));
                        }
                    });
                }
            });
        }

        let tags: Vec<&String> = snapshot.tags.iter().filter(|t| matches(t)).collect();
        section(ui, "tags", icon::TAG, "Tags", tags.len(), false, |ui| {
            if tags.is_empty() {
                ui.label(RichText::new("   No tags").small().color(c.muted));
            }
            for tag in tags {
                let tip = snapshot.tag_tips.get(tag).cloned().unwrap_or_default();
                let response = row(ui, icon::TAG, c.tag, RichText::new(tag), None, false);
                if response.clicked() {
                    if let Some(commit) = self.tag_commit(tag) {
                        self.select_commit(commit);
                    }
                }
                response.context_menu(|ui| self.tag_menu(ui, tag, &tip, snapshot));
            }
        });

        section(ui, "stashes", icon::ARCHIVE, "Stashes", snapshot.stashes.len(), true, |ui| {
            if snapshot.stashes.is_empty() {
                ui.label(RichText::new("   No stashes").small().color(c.muted));
            }
            ui.horizontal(|ui| {
                ui.add_space(14.0);
                if ui.small_button("Manage stashes…").clicked() {
                    self.open_tool(Box::new(tools::stash::StashWindow::new()));
                }
            });
            for stash in &snapshot.stashes {
                let selected = matches!(self.repo().map(|r| &r.selection), Some(Selection::Stash { hash }) if *hash == stash.hash);
                let response =
                    row(ui, icon::ARCHIVE_BOX, c.muted, RichText::new(&stash.message), None, selected).on_hover_text(&stash.reference);
                if response.clicked() {
                    self.select_stash(stash.clone());
                }
                response.context_menu(|ui| {
                    let allowed = idle && snapshot.operation.is_none();
                    if ui.add_enabled(allowed, egui::Button::new(format!("{}  Apply", icon::TRAY_ARROW_DOWN))).clicked() {
                        ui.close();
                        let stash = stash.clone();
                        self.act("Apply stash", move |client, path| {
                            client.apply_stash(&stash, path).map(|_| Some("Applied the stash.".into()))
                        });
                    }
                    if ui.add_enabled(allowed, egui::Button::new(format!("{}  Pop (apply and delete)", icon::TRAY_ARROW_UP))).clicked() {
                        ui.close();
                        let stash = stash.clone();
                        self.confirm(
                            format!("Pop {}?", stash.reference),
                            format!(
                                "“{}” is applied to your working files and then deleted. If applying fails, the stash is kept.",
                                stash.message
                            ),
                            "Pop",
                            Pending::PopStash(stash),
                        );
                    }
                    ui.separator();
                    if ui.add_enabled(idle, egui::Button::new(format!("{}  Delete…", icon::TRASH))).clicked() {
                        ui.close();
                        self.confirm(
                            format!("Delete {}?", stash.reference),
                            format!("“{}” will be deleted.", stash.message),
                            "Delete",
                            Pending::DropStash(stash.clone()),
                        );
                    }
                });
            }
        });

        let submodules =
            self.repos.get_mut(self.active).and_then(|r| r.submodules.as_mut()).and_then(|t| t.get().cloned()).unwrap_or_default();
        if !submodules.is_empty() {
            section(ui, "submodules", icon::PACKAGE, "Submodules", submodules.len(), false, |ui| {
                use nicegit_core::submodule::SubmoduleState;
                for submodule in &submodules {
                    let (state, color) = match &submodule.state {
                        SubmoduleState::NotCheckedOut => ("not checked out".to_string(), c.muted),
                        SubmoduleState::AtRecordedCommit => ("at recorded commit".to_string(), c.added),
                        SubmoduleState::OnAnotherCommit(commit) => (format!("on {}", short(commit)), c.warning),
                    };
                    let state = if submodule.has_local_changes { format!("{state} · changed") } else { state };
                    let response = row(
                        ui,
                        icon::PACKAGE,
                        color,
                        RichText::new(&submodule.path),
                        Some(RichText::new(state).small().color(color)),
                        false,
                    )
                    .on_hover_text(format!("Recorded commit {}", short(&submodule.recorded_commit)));
                    let checked_out = submodule.state != SubmoduleState::NotCheckedOut;
                    let repo_path = self.repo().map(|r| r.path.clone()).unwrap_or_default();
                    let folder = submodule.path.split('/').fold(repo_path, |path, part| path.join(part));
                    if response.double_clicked() && checked_out {
                        self.open(folder.clone());
                    }
                    response.context_menu(|ui| {
                        if ui.add_enabled(checked_out, egui::Button::new(format!("{}  Open", icon::FOLDER_OPEN))).clicked() {
                            ui.close();
                            self.open(folder.clone());
                        }
                        let label = if checked_out { "Check out recorded commit" } else { "Initialise and check out" };
                        if ui.add_enabled(idle, egui::Button::new(format!("{}  {label}", icon::ARROW_COUNTER_CLOCKWISE))).clicked() {
                            ui.close();
                            let path = submodule.path.clone();
                            self.act("Update submodule", move |client, repo| {
                                client.update_submodule(&path, repo).map(|_| Some(format!("Checked out {path} at its recorded commit.")))
                            });
                        }
                        if ui.button(format!("{}  Show in file manager", icon::FOLDER)).clicked() {
                            ui.close();
                            crate::ui::changes::reveal(&folder);
                        }
                    });
                }
            });
        }

        // Pull requests and issues load on demand from a chosen github.com remote.
        self.github_section(ui, snapshot, nicegit_core::github::ItemKind::PullRequest);
        self.github_section(ui, snapshot, nicegit_core::github::ItemKind::Issue);
    }

    /// The commit a tag points to, from the loaded history's decorations.
    fn tag_commit(&self, tag: &str) -> Option<String> {
        let decoration = format!("tag: {tag}");
        self.snapshot()?.commits.iter().find(|c| c.refs.contains(&decoration)).map(|c| c.hash.clone())
    }

    fn tag_menu(&mut self, ui: &mut Ui, tag: &str, tip: &str, snapshot: &Snapshot) {
        let idle = self.idle();
        if ui.button(format!("{}  Copy name", icon::COPY)).clicked() {
            ui.close();
            ui.ctx().copy_text(tag.to_string());
        }
        for remote in &snapshot.remotes {
            if ui.add_enabled(idle, egui::Button::new(format!("{}  Push to {remote}", icon::ARROW_LINE_UP))).clicked() {
                ui.close();
                let (tag, tip, remote, addresses) =
                    (tag.to_string(), tip.to_string(), remote.clone(), snapshot.remote_push_addresses.clone());
                self.act("Push tag", move |client, path| {
                    client
                        .push_tag(&tag, &remote, &tip, addresses.get(&remote).map(Vec::as_slice).unwrap_or_default(), path)
                        .map(|_| Some(format!("Pushed {tag} to {remote}.")))
                });
            }
            if ui.add_enabled(idle, egui::Button::new(format!("{}  Delete from {remote}…", icon::CLOUD_X))).clicked() {
                ui.close();
                let (tag, tip, remote, addresses) =
                    (tag.to_string(), tip.to_string(), remote.clone(), snapshot.remote_push_addresses.clone());
                self.confirm(
                    format!("Delete {tag} from {remote}?"),
                    format!("The tag is removed from {remote} for everyone who uses it, while it still matches your local tag. Your local tag is kept."),
                    "Delete from remote",
                    Pending::DeleteRemoteTag { tag, tip, remote, addresses },
                );
            }
        }
        ui.separator();
        if ui.add_enabled(idle, egui::Button::new(format!("{}  Delete local tag…", icon::TRASH))).clicked() {
            ui.close();
            self.confirm(
                format!("Delete tag {tag}?"),
                "The tag is removed from this repository only, not from remotes.",
                "Delete",
                Pending::DeleteTag { name: tag.to_string(), tip: tip.to_string() },
            );
        }
    }

    pub fn branch_menu(&mut self, ui: &mut Ui, branch: &Branch, snapshot: &Snapshot, can_switch: bool) {
        let idle = self.idle();
        let current = snapshot.current_branch.clone();
        if !branch.is_current && ui.add_enabled(can_switch, egui::Button::new(format!("{}  Check out", icon::CHECK))).clicked() {
            ui.close();
            self.checkout(branch.clone());
        }
        if !snapshot.remotes.is_empty() && ui.add_enabled(idle, egui::Button::new(format!("{}  Fetch", icon::CLOUD_ARROW_DOWN))).clicked() {
            ui.close();
            self.fetch();
        }
        if branch.is_current {
            if ui
                .add_enabled(
                    idle && snapshot.operation.is_none() && snapshot.upstream.is_some(),
                    egui::Button::new(format!("{}  Pull", icon::ARROW_LINE_DOWN)),
                )
                .clicked()
            {
                ui.close();
                self.pull();
            }
            if ui.add_enabled(idle && !snapshot.remotes.is_empty(), egui::Button::new(format!("{}  Push", icon::ARROW_LINE_UP))).clicked() {
                ui.close();
                self.push();
            }
        }
        if !branch.is_remote && !branch.is_detached() {
            for remote in &snapshot.remotes {
                if ui.add_enabled(idle, egui::Button::new(format!("{}  Push to {remote}/{}…", icon::UPLOAD_SIMPLE, branch.name))).clicked()
                {
                    ui.close();
                    self.confirm(
                        format!("Push {} to {remote}?", branch.name),
                        format!(
                            "Updates {remote}/{name} to {} ({}). Only this branch is pushed; tags are not.",
                            short(&branch.tip),
                            branch.subject,
                            name = branch.name
                        ),
                        "Push",
                        Pending::PushBranch {
                            branch: branch.clone(),
                            remote: remote.clone(),
                            addresses: snapshot.remote_push_addresses.clone(),
                        },
                    );
                }
            }
        }
        if !branch.is_current && snapshot.is_on_branch() {
            if ui.add_enabled(can_switch, egui::Button::new(format!("{}  Merge into {current}…", icon::GIT_MERGE))).clicked() {
                ui.close();
                self.request_merge(branch.clone());
            }
            if ui.add_enabled(can_switch, egui::Button::new(format!("{}  Rebase {current} onto this…", icon::GIT_PULL_REQUEST))).clicked()
            {
                ui.close();
                self.request_rebase(branch.clone());
            }
        }
        if !branch.is_current
            && snapshot.is_on_branch()
            && ui
                .add_enabled(
                    idle && snapshot.operation.is_none(),
                    egui::Button::new(format!("{}  Reset {current} to here…", icon::ARROW_ARC_LEFT)),
                )
                .clicked()
        {
            ui.close();
            self.open_tool(Box::new(tools::reset::ResetWindow::new(branch.tip.clone(), branch.subject.clone(), snapshot)));
        }
        ui.separator();
        if ui.add_enabled(idle, egui::Button::new(format!("{}  Create branch here…", icon::GIT_BRANCH))).clicked() {
            ui.close();
            self.dialog = Some(Dialog::input(
                "New branch",
                &format!("Create a branch at {}", branch.display_name()),
                "",
                InputKind::CreateBranchFrom(branch.clone()),
            ));
        }
        if ui.add_enabled(idle, egui::Button::new(format!("{}  Create tag here…", icon::TAG))).clicked() {
            ui.close();
            self.dialog = Some(Dialog::with_second(
                "New tag",
                "Tag name",
                "Tag message",
                InputKind::CreateTag { target: branch.tip.clone(), annotated: false },
            ));
        }
        if !branch.is_remote && !branch.is_detached() {
            if !branch.is_current && ui.add_enabled(idle, egui::Button::new(format!("{}  Create worktree…", icon::FOLDERS))).clicked() {
                ui.close();
                self.open_tool(Box::new(tools::worktrees::WorktreesWindow::for_branch(branch.name.clone(), branch.tip.clone())));
            }
            let remotes = snapshot.remotes.clone();
            if !remotes.is_empty() {
                ui.menu_button(format!("{}  Upstream", icon::ARROWS_LEFT_RIGHT), |ui| {
                    for remote_branch in snapshot.branches.iter().filter(|b| b.is_remote) {
                        let name = remote_branch.display_name();
                        let current = branch.upstream.as_deref() == Some(format!("refs/{}", remote_branch.name).as_str());
                        if ui.add_enabled(idle, egui::Button::selectable(current, name.clone())).clicked() {
                            ui.close();
                            let (branch, tip) = (branch.name.clone(), branch.tip.clone());
                            self.act("Set upstream", move |client, path| {
                                client.set_upstream(&branch, Some(&name), &tip, path).map(|_| Some(format!("{branch} now tracks {name}.")))
                            });
                        }
                    }
                    ui.separator();
                    if ui.add_enabled(idle && branch.upstream.is_some(), egui::Button::new("Stop tracking")).clicked() {
                        ui.close();
                        let (branch, tip) = (branch.name.clone(), branch.tip.clone());
                        self.act("Unset upstream", move |client, path| client.set_upstream(&branch, None, &tip, path).map(|_| None));
                    }
                });
            }
            if ui.add_enabled(idle, egui::Button::new(format!("{}  Rename…", icon::PENCIL_SIMPLE))).clicked() {
                ui.close();
                self.dialog = Some(Dialog::input(
                    &format!("Rename {}", branch.name),
                    "New name",
                    &branch.name,
                    InputKind::RenameBranch(branch.clone()),
                ));
            }
            if !branch.is_current && ui.add_enabled(idle, egui::Button::new(format!("{}  Delete…", icon::TRASH))).clicked() {
                ui.close();
                let merged = Self::branch_is_merged(branch, snapshot);
                // Like the Mac app, this is Git's safe delete: an unmerged branch is refused.
                // Clean Up Branches can delete unmerged branches after a separate choice.
                let message = if merged {
                    "This branch is merged into the current branch, so no commits are lost.".to_string()
                } else {
                    format!(
                        "This branch has commits that are not in the current branch, so Git will refuse to delete it. Merge it first, or use Clean Up Branches to delete unmerged branches. Its tip is {}.",
                        short(&branch.tip)
                    )
                };
                self.confirm(
                    format!("Delete branch {}?", branch.name),
                    message,
                    "Delete",
                    Pending::DeleteBranch { branch: branch.clone(), force: false },
                );
            }
        }
        ui.separator();
        if ui.button(format!("{}  Copy name", icon::COPY)).clicked() {
            ui.close();
            ui.ctx().copy_text(branch.display_name());
        }
        if ui.button(format!("{}  Copy commit ID", icon::HASH)).clicked() {
            ui.close();
            ui.ctx().copy_text(branch.tip.clone());
        }
        crate::git_ext::github_link_menu(ui, snapshot, &branch.tip);
    }

    pub fn request_merge(&mut self, source: Branch) {
        let Some(snapshot) = self.snapshot().cloned() else { return };
        let preview = self.integration_preview(&source, false);
        self.confirm(
            format!("Merge {} into {}?", source.display_name(), snapshot.current_branch),
            preview,
            "Merge",
            Pending::Merge { source, branch: snapshot.current_branch.clone(), head: snapshot.head_hash.clone() },
        );
    }

    pub fn request_rebase(&mut self, onto: Branch) {
        let Some(snapshot) = self.snapshot().cloned() else { return };
        let preview = self.integration_preview(&onto, true);
        self.confirm(
            format!("Rebase {} onto {}?", snapshot.current_branch, onto.display_name()),
            preview,
            "Rebase",
            Pending::Rebase { onto, branch: snapshot.current_branch.clone(), head: snapshot.head_hash.clone() },
        );
    }

    /// A short prediction of a merge or rebase, computed before the confirmation opens.
    fn integration_preview(&self, source: &Branch, rebase: bool) -> String {
        let Some(repo) = self.repo() else { return String::new() };
        match crate::git_ext::merge_preview_text(&source.tip, rebase, &repo.path) {
            Ok(text) => text,
            Err(_) if rebase => {
                "Your branch's commits are replayed on top of the other branch. Conflicts stop the rebase for you to resolve or abort."
                    .into()
            }
            Err(_) => {
                "Git merges the branch's commits into your current branch. Conflicts stop the merge for you to resolve or abort.".into()
            }
        }
    }
}
