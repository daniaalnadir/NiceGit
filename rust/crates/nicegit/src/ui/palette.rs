//! A keyboard-driven list of actions, branches, and repositories. Commands call the same app
//! actions as buttons, so their safety checks still apply.

use std::path::PathBuf;

use egui::{Key, RichText};
use egui_phosphor::regular as icon;
use nicegit_core::Branch;

use crate::app::NiceGitApp;
use crate::theme;
use crate::tools;

#[derive(Default)]
pub struct PaletteState {
    query: String,
    selection: usize,
    focused: bool,
}

#[derive(Clone)]
enum Run {
    Fetch,
    Pull,
    Push,
    Refresh,
    StageAll,
    UnstageAll,
    Undo,
    Redo,
    Terminal,
    Settings,
    Open,
    Clone,
    NewRepository,
    Tool(&'static str),
    Checkout(Branch),
    Repository(PathBuf),
    ApplyPatch,
    Identity { name: String, email: String, signing_key: Option<String> },
}

struct Command {
    title: String,
    detail: String,
    glyph: &'static str,
    enabled: bool,
    run: Run,
}

/// Orders commands by how well `query` matches their title: every query character must
/// appear in order; earlier, word-starting, and consecutive matches rank higher.
fn score(title: &[char], needle: &[char]) -> Option<i32> {
    let (mut score, mut position, mut previous) = (0i32, 0usize, -2isize);
    for character in needle {
        let found = title[position..].iter().position(|c| c == character)? + position;
        let word_start = found == 0 || !title[found - 1].is_alphanumeric();
        score += if word_start { 8 } else { 1 } + if found as isize == previous + 1 { 8 } else { 0 } - (found.min(20) / 4) as i32;
        previous = found as isize;
        position = found + 1;
    }
    Some(score)
}

fn ranked(commands: Vec<Command>, query: &str) -> Vec<Command> {
    let needle: Vec<char> = query.to_lowercase().chars().filter(|c| !c.is_whitespace()).collect();
    if needle.is_empty() {
        return commands;
    }
    let mut scored: Vec<(Command, i32)> = commands
        .into_iter()
        .filter_map(|command| {
            let title: Vec<char> = command.title.to_lowercase().chars().collect();
            score(&title, &needle).map(|s| (command, s))
        })
        .collect();
    scored.sort_by_key(|entry| std::cmp::Reverse(entry.1));
    scored.into_iter().map(|(command, _)| command).collect()
}

impl NiceGitApp {
    pub fn toggle_palette(&mut self) {
        self.palette = if self.palette.is_some() { None } else { Some(PaletteState::default()) };
    }

    fn commands(&self) -> Vec<Command> {
        let snapshot = self.snapshot();
        let has = snapshot.is_some();
        let idle = self.idle();
        let clean = snapshot.is_some_and(|s| s.operation.is_none());
        let command = |title: &str, detail: &str, glyph: &'static str, enabled: bool, run: Run| Command {
            title: title.into(),
            detail: detail.into(),
            glyph,
            enabled,
            run,
        };
        let mut list = vec![
            command("Fetch", "Download from all remotes", icon::CLOUD_ARROW_DOWN, idle, Run::Fetch),
            command("Pull", "Fetch and fast-forward the current branch", icon::ARROW_LINE_DOWN, idle && clean, Run::Pull),
            command("Push", "Send the current branch to its upstream", icon::ARROW_LINE_UP, idle, Run::Push),
            command("Refresh", "Ctrl/Cmd-R", icon::ARROWS_CLOCKWISE, has && self.busy.is_none(), Run::Refresh),
            command("Stage all changes", "", icon::PLUS_CIRCLE, idle && snapshot.is_some_and(|s| !s.status.is_empty()), Run::StageAll),
            command("Unstage all changes", "", icon::MINUS_CIRCLE, idle && snapshot.is_some_and(|s| s.staged_count() > 0), Run::UnstageAll),
            command("Show terminal", "Ctrl-`", icon::TERMINAL_WINDOW, has, Run::Terminal),
            command("Settings", "Appearance, graph colours, and diffs", icon::GEAR, true, Run::Settings),
            command("Open repository…", "Ctrl/Cmd-O", icon::FOLDER_OPEN, self.busy.is_none(), Run::Open),
            command("Clone repository…", "", icon::DOWNLOAD_SIMPLE, self.busy.is_none(), Run::Clone),
            command("New repository…", "", icon::PLUS, self.busy.is_none(), Run::NewRepository),
            command("Repository settings", "Identity, profiles, and remotes", icon::GEAR_SIX, has, Run::Tool("settings")),
            command("Stashes", "Save selected files, preview, apply, or delete stashes", icon::ARCHIVE, has, Run::Tool("stashes")),
            command("Search history", "Shift-Ctrl/Cmd-F · every branch", icon::MAGNIFYING_GLASS, has, Run::Tool("search")),
            command("Search file contents", "Alt-Ctrl/Cmd-F", icon::FILE_MAGNIFYING_GLASS, has, Run::Tool("grep")),
            command(
                "Recover lost work",
                "Commits from resets, rebases, and deleted branches",
                icon::CLOCK_COUNTER_CLOCKWISE,
                has,
                Run::Tool("reflog"),
            ),
            command("Clean up branches", "Delete merged or inactive local branches", icon::BROOM, has, Run::Tool("cleanup")),
            command("Bisect", "Find the commit that introduced a bug", icon::BUG, has, Run::Tool("bisect")),
            command("Worktrees", "Linked worktrees", icon::FOLDERS, has, Run::Tool("worktrees")),
            command("Submodules", "", icon::PACKAGE, has, Run::Tool("submodules")),
            command("GitFlow", "Feature, release, and hotfix branches", icon::FLOW_ARROW, has, Run::Tool("gitflow")),
            command("Git LFS", "Tracked patterns and large files", icon::DATABASE, has, Run::Tool("lfs")),
            command(
                "Pull requests and issues",
                "github.com through the GitHub CLI",
                icon::GITHUB_LOGO,
                has && snapshot.is_some_and(|s| !s.remotes.is_empty()),
                Run::Tool("github"),
            ),
            command("Apply patch…", "", icon::FILE_PLUS, idle && clean, Run::ApplyPatch),
        ];
        if let Some(step) = self.repo().and_then(|r| r.undo.as_ref()) {
            list.push(command(&format!("Undo {}", step.title.to_lowercase()), "", icon::ARROW_COUNTER_CLOCKWISE, idle, Run::Undo));
        }
        if let Some(step) = self.repo().and_then(|r| r.redo.as_ref()) {
            list.push(command(&format!("Redo {}", step.title.to_lowercase()), "", icon::ARROW_CLOCKWISE, idle, Run::Redo));
        }
        if let Some(snapshot) = snapshot {
            for branch in snapshot.branches.iter().filter(|b| !b.is_current && !b.is_detached()) {
                list.push(Command {
                    title: format!("Switch to {}", branch.display_name()),
                    detail: if branch.is_remote { "Remote branch".into() } else { "Local branch".into() },
                    glyph: if branch.is_remote { icon::CLOUD } else { icon::GIT_BRANCH },
                    enabled: idle && clean,
                    run: Run::Checkout(branch.clone()),
                });
            }
        }
        // Saved identity profiles, from Repository Settings.
        let profiles: Vec<tools::repository_settings::StoredProfile> = self
            .worker
            .context
            .data_mut(|data| data.get_persisted(egui::Id::new(tools::repository_settings::PROFILES_KEY)))
            .unwrap_or_default();
        for profile in profiles {
            let detail = if profile.signing_key.is_some() { format!("{} · signs commits", profile.email) } else { profile.email.clone() };
            list.push(Command {
                title: format!("Use identity {}", profile.name),
                detail,
                glyph: icon::USER_CIRCLE,
                enabled: idle,
                run: Run::Identity { name: profile.name, email: profile.email, signing_key: profile.signing_key },
            });
        }
        let open: Vec<PathBuf> = self.repos.iter().map(|r| r.path.clone()).collect();
        for path in self.settings.recent.iter().chain(open.iter()) {
            if self.repo().is_some_and(|r| &r.path == path) || list.iter().any(|c| matches!(&c.run, Run::Repository(p) if p == path)) {
                continue;
            }
            let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            list.push(Command {
                title: format!("Open {name}"),
                detail: path.display().to_string(),
                glyph: icon::FOLDER,
                enabled: self.busy.is_none(),
                run: Run::Repository(path.clone()),
            });
        }
        list
    }

    fn run_command(&mut self, run: Run, ctx: &egui::Context) {
        match run {
            Run::Fetch => self.fetch(),
            Run::Pull => self.pull(),
            Run::Push => self.push(),
            Run::Refresh => self.load(true),
            Run::StageAll => self.act("Stage all", |client, path| client.stage_all(path).map(|_| None)),
            Run::UnstageAll => self.act("Unstage all", |client, path| client.unstage_all(path).map(|_| None)),
            Run::Undo => self.undo(),
            Run::Redo => self.redo(),
            Run::Terminal => self.toggle_terminal(ctx),
            Run::Settings => self.show_settings = true,
            Run::Open => self.choose_folder(),
            Run::Clone => self.dialog = Some(crate::ui::dialogs::Dialog::clone_repository()),
            Run::NewRepository => self.create_repository(),
            Run::ApplyPatch => self.apply_patch(),
            Run::Identity { name, email, signing_key } => self.act("Apply identity", move |client, path| {
                client
                    .apply_identity(&name, &email, signing_key.as_deref(), path)
                    .map(|_| Some(format!("This repository now commits as {name} <{email}>.")))
            }),
            Run::Checkout(branch) => self.checkout(branch),
            Run::Repository(path) => self.open(path),
            Run::Tool(name) => {
                let remote = self.snapshot().and_then(|s| s.remotes.iter().find(|r| *r == "origin").or(s.remotes.first()).cloned());
                let tool: Option<Box<dyn tools::ToolWindow>> = match name {
                    "settings" => Some(Box::new(tools::repository_settings::RepositorySettingsWindow::new())),
                    "stashes" => Some(Box::new(tools::stash::StashWindow::new())),
                    "search" => Some(Box::new(tools::commit_search::CommitSearchWindow::new())),
                    "grep" => Some(Box::new(tools::content_search::ContentSearchWindow::new(None))),
                    "reflog" => Some(Box::new(tools::reflog::ReflogWindow::new())),
                    "cleanup" => Some(Box::new(tools::branch_cleanup::BranchCleanupWindow::new())),
                    "bisect" => Some(Box::new(tools::bisect::BisectWindow::new())),
                    "worktrees" => Some(Box::new(tools::worktrees::WorktreesWindow::new())),
                    "submodules" => Some(Box::new(tools::submodules::SubmodulesWindow::new())),
                    "gitflow" => Some(Box::new(tools::gitflow::GitFlowWindow::new())),
                    "lfs" => Some(Box::new(tools::lfs::LfsWindow::new())),
                    "github" => remote.map(|r| Box::new(tools::github::GitHubWindow::new(r)) as Box<dyn tools::ToolWindow>),
                    _ => None,
                };
                if let Some(tool) = tool {
                    self.open_tool(tool);
                }
            }
        }
    }

    /// Opens windows named in `NICEGIT_DEBUG_OPEN` (comma-separated) once the repository loads,
    /// so screenshots of each window can be taken without clicking. Debug builds only.
    pub fn debug_open(&mut self, ctx: &egui::Context) {
        if !cfg!(debug_assertions) || self.snapshot().is_none() {
            return;
        }
        let Some(names) = std::env::var("NICEGIT_DEBUG_OPEN").ok() else { return };
        // SAFETY: only read once at start-up in debug builds, before other threads use it.
        unsafe { std::env::remove_var("NICEGIT_DEBUG_OPEN") };
        let snapshot = self.snapshot().cloned().expect("loaded");
        let head = snapshot.head_hash.clone().unwrap_or_default();
        // Three commits back along HEAD's first parents.
        let mut base = head.clone();
        for _ in 0..3 {
            base = snapshot.commits.iter().find(|c| c.hash == base).and_then(|c| c.parents.first().cloned()).unwrap_or(base);
        }
        let file = snapshot.status.first().map(|e| e.path.clone()).unwrap_or_else(|| "README.md".into());
        for name in names.split(',') {
            match name {
                "palette" => self.palette = Some(PaletteState::default()),
                "settings" => self.show_settings = true,
                "light" => self.applied_light_preview(ctx),
                "repository" => self.run_command(Run::Tool("settings"), ctx),
                "terminal" => self.toggle_terminal(ctx),
                "inspector" => self.select_commit(head.clone()),
                "change" => self.select_working_tree(),
                "blame" => self.open_tool(Box::new(tools::blame::BlameWindow::new("README.md".into(), None))),
                "history" => self.open_tool(Box::new(tools::file_history::FileHistoryWindow::new("README.md".into()))),
                "rebase" => self.open_tool(Box::new(tools::interactive_rebase::InteractiveRebaseWindow::new(base.clone(), &snapshot))),
                "reset" => self.open_tool(Box::new(tools::reset::ResetWindow::new(base.clone(), "An older commit".into(), &snapshot))),
                "compare" => self.open_tool(Box::new(tools::compare::CompareWindow::new(base.clone(), None))),
                "editor" => self.open_tool(Box::new(tools::editor::EditorWindow::new(
                    self.repo().map(|r| r.path.clone()).unwrap_or_default(),
                    file.clone(),
                ))),
                other => self.run_command(Run::Tool(Box::leak(other.to_string().into_boxed_str())), ctx),
            }
        }
    }

    /// Shows the light appearance for this session only, without changing saved settings.
    fn applied_light_preview(&mut self, ctx: &egui::Context) {
        crate::theme::apply(ctx, crate::theme::Appearance::Light);
        ctx.set_theme(egui::ThemePreference::Light);
        self.preview_light = true;
    }

    pub fn palette_window(&mut self, ctx: &egui::Context) {
        let Some(mut state) = self.palette.take() else { return };
        let matches = ranked(self.commands(), &state.query);
        state.selection = state.selection.min(matches.len().saturating_sub(1));
        let mut chosen: Option<Run> = None;
        let mut close = false;
        let modal = egui::Modal::new(egui::Id::new("palette")).show(ctx, |ui| {
            let c = theme::of(ui);
            ui.set_width(580.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new(icon::COMMAND).size(18.0).color(c.muted));
                let field = ui.add(
                    egui::TextEdit::singleline(&mut state.query)
                        .hint_text("Type a command, branch, or repository")
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
            let (down, up, enter, escape) = ui.input(|i| {
                (i.key_pressed(Key::ArrowDown), i.key_pressed(Key::ArrowUp), i.key_pressed(Key::Enter), i.key_pressed(Key::Escape))
            });
            if down {
                state.selection = (state.selection + 1).min(matches.len().saturating_sub(1));
            }
            if up {
                state.selection = state.selection.saturating_sub(1);
            }
            if escape {
                close = true;
            }
            egui::ScrollArea::vertical().max_height(380.0).auto_shrink([false, true]).show(ui, |ui| {
                if matches.is_empty() {
                    ui.label(RichText::new("No matching commands").color(c.muted));
                }
                for (index, command) in matches.iter().enumerate() {
                    let selected = index == state.selection;
                    let frame = egui::Frame::new()
                        .fill(if selected { ui.visuals().selection.bg_fill } else { egui::Color32::TRANSPARENT })
                        .corner_radius(6.0)
                        .inner_margin(egui::Margin::symmetric(10, 6));
                    let response = frame
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.horizontal(|ui| {
                                let color = if command.enabled { ui.visuals().text_color() } else { c.muted.gamma_multiply(0.6) };
                                ui.label(RichText::new(command.glyph).color(c.muted));
                                ui.label(RichText::new(&command.title).color(color));
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    ui.add(egui::Label::new(RichText::new(&command.detail).small().color(c.muted)).truncate());
                                });
                            });
                        })
                        .response
                        .interact(egui::Sense::click());
                    if selected && (down || up) {
                        response.scroll_to_me(None);
                    }
                    if response.clicked() && command.enabled {
                        chosen = Some(command.run.clone());
                    }
                }
            });
            if enter {
                if let Some(command) = matches.get(state.selection).filter(|c| c.enabled) {
                    chosen = Some(command.run.clone());
                }
            }
        });
        if modal.should_close() {
            close = true;
        }
        if let Some(run) = chosen {
            self.run_command(run, ctx);
        } else if !close {
            self.palette = Some(state);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::score;

    fn s(title: &str, query: &str) -> Option<i32> {
        score(&title.chars().collect::<Vec<_>>(), &query.chars().collect::<Vec<_>>())
    }

    #[test]
    fn word_starts_and_runs_rank_higher() {
        assert!(s("fetch", "ft").is_some());
        assert!(s("fetch", "xz").is_none());
        assert!(s("stage all changes", "sac").unwrap() > s("discard all changes", "sac").unwrap_or(i32::MIN));
        assert!(s("push", "pu").unwrap() > s("stash pop", "pu").unwrap_or(i32::MIN));
    }
}
