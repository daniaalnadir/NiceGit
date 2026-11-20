//! GitFlow: sets up the production and development branches and their prefixes, starts feature,
//! release, and hotfix branches, and finishes the one that is checked out.
#![allow(dead_code)]

use crate::i18n::{tr, tr_fmt};
use egui::{RichText, Ui};
use egui_phosphor::regular as icon;
use nicegit_core::gitflow::{GitFlowConfiguration, GitFlowKind};
use nicegit_core::{Snapshot, StatusKind};

use crate::theme;
use crate::tools::{query, widgets, Ctx, Task, ToolWindow};

type Loaded = nicegit_core::Result<Option<GitFlowConfiguration>>;

pub struct GitFlowWindow {
    /// The settings, read in the background. None until read.
    configuration: Option<Task<Loaded>>,
    /// The last settings read, shown while newer ones load so the window does not flicker.
    shown: Option<Loaded>,
    /// The setup form, filled in from the repository's branches the first time it is shown.
    draft: Option<GitFlowConfiguration>,
    kind: GitFlowKind,
    name: String,
    tag_message: String,
    /// The branch a start was requested for; its name is cleared once that branch is checked out.
    pending_start: Option<String>,
    /// The branch a finish was requested for; the tag message is cleared once it is no longer checked out.
    pending_finish: Option<String>,
}

impl Default for GitFlowWindow {
    fn default() -> Self {
        Self::new()
    }
}

impl GitFlowWindow {
    pub fn new() -> Self {
        Self {
            configuration: None,
            shown: None,
            draft: None,
            kind: GitFlowKind::Feature,
            name: String::new(),
            tag_message: String::new(),
            pending_start: None,
            pending_finish: None,
        }
    }
}

impl ToolWindow for GitFlowWindow {
    fn id(&self) -> String {
        "gitflow".to_string()
    }

    fn title(&self) -> String {
        tr("GitFlow").to_string()
    }

    fn default_size(&self) -> egui::Vec2 {
        egui::vec2(540.0, 560.0)
    }

    fn repository_changed(&mut self, _snapshot: &Snapshot) {
        self.configuration = None;
    }

    fn ui(&mut self, ui: &mut Ui, cx: &mut Ctx) {
        if self.configuration.is_none() {
            self.configuration = Some(query(ui.ctx(), cx.repo, |client, directory| client.gitflow_configuration(directory)));
        }
        if let Some(loaded) = self.configuration.as_mut().and_then(|task| task.get().cloned()) {
            self.shown = Some(loaded);
        }
        let loaded = self.shown.clone();

        let c = theme::of(ui);
        ui.label(
            RichText::new(tr("Branches for releases, ongoing development, and short-lived work, compatible with the git-flow tool."))
                .small()
                .color(c.muted),
        );
        ui.add_space(8.0);

        match loaded {
            None => widgets::loading(ui, tr("Reading GitFlow settings…")),
            Some(Err(error)) => widgets::error(ui, &error.to_string()),
            Some(Ok(None)) => self.setup(ui, cx),
            Some(Ok(Some(configuration))) => self.flow(ui, cx, &configuration),
        }
    }
}

impl GitFlowWindow {
    /// The form that sets GitFlow up. Suggests the repository's existing production branch.
    fn setup(&mut self, ui: &mut Ui, cx: &mut Ctx) {
        let draft = self.draft.get_or_insert_with(|| GitFlowConfiguration {
            main_branch: suggested_production_branch(cx.snapshot),
            ..GitFlowConfiguration::default()
        });
        egui::Grid::new("gitflow_setup").num_columns(2).spacing([14.0, 8.0]).show(ui, |ui| {
            let fields: [(&str, &mut String); 6] = [
                (tr("Production branch"), &mut draft.main_branch),
                (tr("Development branch"), &mut draft.develop_branch),
                (tr("Feature prefix"), &mut draft.feature_prefix),
                (tr("Release prefix"), &mut draft.release_prefix),
                (tr("Hotfix prefix"), &mut draft.hotfix_prefix),
                (tr("Version tag prefix"), &mut draft.version_tag_prefix),
            ];
            for (label, value) in fields {
                ui.label(label);
                ui.add(egui::TextEdit::singleline(value).desired_width(260.0));
                ui.end_row();
            }
        });
        ui.add_space(8.0);
        let main = draft.main_branch.trim().to_string();
        let develop = draft.develop_branch.trim().to_string();
        let valid = !main.is_empty() && !develop.is_empty() && main != develop;
        if !valid {
            widgets::callout(ui, tr("The production and development branches need different names."), true);
            ui.add_space(6.0);
        }
        let to_create = draft.clone();
        ui.horizontal(|ui| {
            if widgets::primary_button(ui, tr("Set up GitFlow"), cx.idle && valid).clicked() {
                cx.act(tr("Set up GitFlow"), move |client, directory| {
                    client.initialize_gitflow(&to_create, directory).map(|()| Some(tr("GitFlow is set up.").to_string()))
                });
            }
        });
    }

    /// Start and finish sections, for a repository that has GitFlow set up.
    fn flow(&mut self, ui: &mut Ui, cx: &mut Ctx, configuration: &GitFlowConfiguration) {
        let c = theme::of(ui);
        ui.horizontal_wrapped(|ui| {
            widgets::pill(ui, &tr_fmt!("Production  {branch}", branch = configuration.main_branch), c.local_branch);
            widgets::pill(ui, &tr_fmt!("Development  {branch}", branch = configuration.develop_branch), c.local_branch);
        });
        ui.add_space(8.0);

        // Clear the typed names once the branch they created is checked out.
        if let Some(started) = &self.pending_start {
            if cx.snapshot.current_branch == *started {
                self.name.clear();
                self.pending_start = None;
            }
        }
        if let Some(finishing) = &self.pending_finish {
            if cx.snapshot.current_branch != *finishing {
                self.tag_message.clear();
                self.pending_finish = None;
            }
        }

        widgets::section(ui, tr("Start"));
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            for kind in GitFlowKind::ALL {
                ui.selectable_value(&mut self.kind, kind, kind_title(kind));
            }
        });
        ui.add_space(4.0);
        let hint = if self.kind == GitFlowKind::Feature { tr("Feature name") } else { tr("Version, such as 1.2.0") };
        ui.add(egui::TextEdit::singleline(&mut self.name).hint_text(hint).desired_width(f32::INFINITY))
            .widget_info(|| egui::WidgetInfo::labeled(egui::Role::TextInput, true, hint));
        let name = self.name.trim().to_string();
        let prefix = configuration.prefix(self.kind).to_string();
        let base = configuration.start_branch(self.kind).to_string();
        let shown_name = if name.is_empty() { "…".to_string() } else { name.clone() };
        ui.add_space(2.0);
        ui.label(
            RichText::new(tr_fmt!("Creates {name} from {base} and checks it out.", name = format!("{prefix}{shown_name}"), base = base))
                .small()
                .color(c.muted),
        );
        ui.add_space(6.0);
        let can_start = cx.idle && !name.is_empty() && cx.snapshot.operation.is_none();
        ui.horizontal(|ui| {
            let label = tr_fmt!("Start {kind}", kind = kind_name(self.kind));
            if widgets::primary_button(ui, &label, can_start).clicked() {
                let kind = self.kind;
                let expected_branch = cx.snapshot.current_branch.clone();
                let expected_head = cx.snapshot.head_hash.clone();
                self.pending_start = Some(format!("{prefix}{name}"));
                let started = name.clone();
                cx.act(tr_fmt!("Start {kind} {name}", kind = kind_name(kind), name = started), move |client, directory| {
                    client
                        .start_gitflow(kind, &started, &expected_branch, expected_head.as_deref(), directory)
                        .map(|branch| Some(tr_fmt!("Started {branch}.", branch = branch)))
                });
            }
        });

        ui.add_space(12.0);
        ui.separator();
        ui.add_space(8.0);
        widgets::section(ui, tr("Finish"));
        ui.add_space(4.0);
        let current = cx.snapshot.current_branch.clone();
        let Some((kind, version)) = configuration.classify(&current) else {
            ui.label(
                RichText::new(tr_fmt!(
                    "Check out a {feature}…, {release}…, or {hotfix}… branch to finish it.",
                    feature = configuration.prefix(GitFlowKind::Feature),
                    release = configuration.prefix(GitFlowKind::Release),
                    hotfix = configuration.prefix(GitFlowKind::Hotfix),
                ))
                .small()
                .color(c.muted),
            );
            return;
        };
        let version = version.to_string();
        let caption = match kind {
            GitFlowKind::Feature => tr_fmt!(
                "Merges {branch} into {develop} without fast-forwarding, then deletes {branch}. If a merge conflicts, NiceGit stops so you can resolve it, then finish again.",
                branch = current,
                develop = configuration.develop_branch
            ),
            _ => tr_fmt!(
                "Merges {branch} into {main} and {develop} without fast-forwarding, tags {tag_prefix}{version} on {main}, then deletes {branch}. If a merge conflicts, NiceGit stops so you can resolve it, then finish again.",
                branch = current,
                main = configuration.main_branch,
                develop = configuration.develop_branch,
                tag_prefix = configuration.version_tag_prefix,
                version = version
            ),
        };
        ui.label(RichText::new(caption).small().color(c.muted));
        if kind != GitFlowKind::Feature {
            ui.add_space(4.0);
            ui.add(egui::TextEdit::singleline(&mut self.tag_message).hint_text(tr("Tag message (optional)")).desired_width(f32::INFINITY));
        }
        let uncommitted = cx.snapshot.status.iter().any(|entry| entry.kind != StatusKind::Untracked);
        if uncommitted {
            ui.add_space(4.0);
            widgets::callout(ui, tr("Commit or stash your changes before finishing."), true);
        }
        ui.add_space(6.0);
        let can_finish = cx.idle && cx.snapshot.operation.is_none() && !uncommitted;
        ui.horizontal(|ui| {
            let label = format!("{}  {}", icon::GIT_MERGE, tr_fmt!("Finish {branch}", branch = current));
            if widgets::primary_button(ui, &label, can_finish).clicked() {
                let expected_branch = current.clone();
                let expected_head = cx.snapshot.head_hash.clone();
                let message = self.tag_message.trim().to_string();
                let tag_message = (!message.is_empty()).then_some(message);
                self.pending_finish = Some(current.clone());
                cx.act(tr_fmt!("Finish {branch}", branch = current), move |client, directory| {
                    client.finish_gitflow(&expected_branch, expected_head.as_deref(), tag_message.as_deref(), directory).map(Some)
                });
            }
        });
    }
}

/// The name shown for a kind of branch in the interface, such as "Feature".
fn kind_title(kind: GitFlowKind) -> &'static str {
    match kind {
        GitFlowKind::Feature => tr("Feature"),
        GitFlowKind::Release => tr("Release"),
        GitFlowKind::Hotfix => tr("Hotfix"),
    }
}

/// The lowercase kind name used in the start and finish labels, such as "feature".
fn kind_name(kind: GitFlowKind) -> &'static str {
    match kind {
        GitFlowKind::Feature => tr("feature"),
        GitFlowKind::Release => tr("release"),
        GitFlowKind::Hotfix => tr("hotfix"),
    }
}

/// The repository's existing production branch, as git-flow users expect it to be named.
fn suggested_production_branch(snapshot: &Snapshot) -> String {
    let has_local = |name: &str| snapshot.branches.iter().any(|branch| !branch.is_remote && branch.name == name);
    if has_local("main") {
        "main".to_string()
    } else if has_local("master") {
        "master".to_string()
    } else {
        snapshot.current_branch.clone()
    }
}
