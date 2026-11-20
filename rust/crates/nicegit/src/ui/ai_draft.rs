//! The Draft button in the commit box, which writes a commit message with Claude from the staged
//! changes, and the Claude section of Settings where the model and the key are set.
//!
//! A draft only reads the repository. It runs on a background thread, and its result fills the
//! summary and description, after the person confirms when they already have text there.

use std::path::PathBuf;

use egui::RichText;
use egui_phosphor::regular as icon;
use nicegit_core::ai_message::{Cancel, Drafted};

use crate::app::NiceGitApp;
use crate::claude::{self, ClaudeModel, Job};
use crate::i18n::{tr, tr_fmt};
use crate::settings::Draft;
use crate::theme;
use crate::tools::widgets;
use crate::tools::Task;

/// A draft running in the background, and the repository it was asked for.
pub struct DraftRun {
    pub repo: PathBuf,
    pub task: Task<Result<Drafted, String>>,
    pub cancel: Cancel,
}

/// A finished draft that waits for the person to say whether it replaces the current message.
pub struct ReadyDraft {
    pub repo: PathBuf,
    pub drafted: Drafted,
}

/// What the last draft reported for a repository: a problem, or the notes that came with it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DraftStatus {
    pub repo: PathBuf,
    pub problem: Option<String>,
    pub notes: Vec<String>,
}

/// The key as it is being typed in Settings, and why the last save or removal failed.
#[derive(Clone, Debug, Default)]
struct KeyDraft {
    text: String,
    problem: Option<String>,
}

fn key_draft_id() -> egui::Id {
    egui::Id::unique("claude_key_draft")
}

/// Forgets the key being typed, so it does not stay in memory after Settings closes.
pub fn forget_key_draft(ctx: &egui::Context) {
    ctx.data_mut(|data| data.remove::<KeyDraft>(key_draft_id()));
}

/// The Draft button. Returns whether it was clicked. While a draft runs it shows a spinner, and a
/// click stops the draft instead.
pub fn draft_button(ui: &mut egui::Ui, drafting: bool, enabled: bool) -> bool {
    let c = theme::of(ui);
    if drafting {
        ui.spinner();
    }
    let text = if drafting { tr("Drafting…") } else { tr("Draft") };
    let hover = if drafting { tr("Drafting. Click to stop.") } else { tr("Draft a message from the staged changes with Claude") };
    let button = egui::Button::new(RichText::new(format!("{}  {text}", icon::SPARKLE)).color(c.accent)).small();
    let response = ui.add_enabled(enabled, button).on_hover_text(hover);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, tr("Draft a message with Claude")));
    response.clicked()
}

/// The problem or notes a draft reported, shown under the commit box.
pub fn draft_messages(ui: &mut egui::Ui, status: &DraftStatus) {
    let c = theme::of(ui);
    if let Some(problem) = &status.problem {
        ui.label(RichText::new(problem).small().color(c.warning));
    }
    for note in &status.notes {
        ui.label(RichText::new(note).small().color(c.muted));
    }
}

/// The question shown when a finished draft would replace text already there. Returns the answer
/// once the person clicks one of its buttons.
pub fn replace_question(ui: &mut egui::Ui) -> Option<bool> {
    let mut answer = None;
    ui.horizontal_wrapped(|ui| {
        ui.label(tr("Replace the current message?"));
        if ui.button(tr("Replace")).clicked() {
            answer = Some(true);
        }
        if ui.button(tr("Keep mine")).clicked() {
            answer = Some(false);
        }
    });
    answer
}

impl NiceGitApp {
    /// Hands a finished draft to the commit box. It fills the message when it is empty, and waits
    /// for the person's answer otherwise. A draft for a repository no longer open is dropped.
    pub fn finish_draft(&mut self) {
        let Some(run) = self.draft_run.as_mut() else { return };
        let Some(result) = run.task.get().cloned() else { return };
        let repo = run.repo.clone();
        self.draft_run = None;
        match result {
            Err(problem) => self.draft_status = Some(DraftStatus { repo, problem: Some(problem), notes: Vec::new() }),
            Ok(drafted) => {
                let Some(index) = self.repos.iter().position(|open| open.path == repo) else { return };
                let current = &self.repos[index].draft;
                let has_text = !current.summary.trim().is_empty() || !current.description.trim().is_empty();
                self.draft_status = Some(DraftStatus { repo: repo.clone(), problem: None, notes: drafted.notes.clone() });
                if has_text {
                    self.draft_ready = Some(ReadyDraft { repo, drafted });
                } else {
                    self.apply_draft(index, drafted.summary, drafted.body);
                }
            }
        }
    }

    /// Starts a draft of the staged changes in the open repository, in the background.
    pub fn start_draft(&mut self, ctx: &egui::Context) {
        let Some(repo) = self.repo() else { return };
        let path = repo.path.clone();
        let cancel = Cancel::new();
        let job = Job {
            repo: path.clone(),
            model: self.settings.claude_model,
            conventional: self.conventional_in_use(),
            key_file: self.key_file.clone(),
            cancel: cancel.clone(),
        };
        let drafter = self.drafter;
        let task = Task::spawn(ctx, move || drafter(job));
        self.draft_run = Some(DraftRun { repo: path, task, cancel });
        self.draft_status = None;
        self.draft_ready = None;
    }

    /// Stops the draft running, if any. Its answer is dropped.
    pub fn cancel_draft(&mut self) {
        if let Some(run) = self.draft_run.take() {
            run.cancel.cancel();
        }
    }

    /// Acts on the person's answer to the replace question for the open repository.
    pub fn answer_draft(&mut self, replace: bool) {
        let Some(ready) = self.draft_ready.take() else { return };
        if !replace {
            return;
        }
        if let Some(index) = self.repos.iter().position(|open| open.path == ready.repo) {
            self.apply_draft(index, ready.drafted.summary, ready.drafted.body);
        }
    }

    fn apply_draft(&mut self, index: usize, summary: String, description: String) {
        let repo = &mut self.repos[index];
        repo.draft = Draft { summary, description };
        repo.draft_from_template = None;
    }

    /// Whether the commit box shows Conventional Commits types, so a draft follows the same rule.
    fn conventional_in_use(&self) -> bool {
        use crate::conventional::{repository_uses_convention, ConventionalCommits};
        match self.settings.conventional_commits {
            ConventionalCommits::Off => false,
            ConventionalCommits::Always => true,
            ConventionalCommits::Automatic => self.snapshot().is_some_and(|snapshot| {
                repository_uses_convention(
                    snapshot.commits.iter().filter(|commit| commit.parents.len() <= 1).map(|commit| commit.subject.as_str()),
                )
            }),
        }
    }

    /// The Claude section of Settings: the switch, the model, and the saved key.
    pub fn claude_section(&mut self, ui: &mut egui::Ui) {
        let c = theme::of(ui);
        widgets::section(ui, "Claude");
        widgets::checkbox(ui, true, &mut self.settings.draft_with_claude, tr("Draft commit messages with Claude"));
        ui.label(
            RichText::new(
                tr("When you click Draft, the staged diff and recent commit subjects are sent to Anthropic's API using your key. Nothing is sent otherwise."),
            )
            .small()
            .color(c.muted),
        );
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label(tr("Model"));
            egui::ComboBox::from_id_salt("claude_model").selected_text(self.settings.claude_model.title()).show_ui(ui, |ui| {
                for model in ClaudeModel::ALL {
                    ui.selectable_value(&mut self.settings.claude_model, model, model.title());
                }
            });
        });
        ui.add_space(4.0);
        let saved = self.key_file.as_deref().is_some_and(|path| path.exists());
        let status = if claude::environment_key().is_some() {
            tr_fmt!("Using {variable} from the environment.", variable = claude::KEY_ENVIRONMENT)
        } else if saved {
            tr("A key is saved.").to_string()
        } else {
            tr("No API key is saved.").to_string()
        };
        ui.label(RichText::new(status).small().color(c.muted));

        let mut draft: KeyDraft = ui.ctx().data(|data| data.get_temp(key_draft_id())).unwrap_or_default();
        let (mut save, mut remove) = (false, false);
        ui.horizontal(|ui| {
            let field =
                ui.add(egui::TextEdit::singleline(&mut draft.text).password(true).hint_text(tr("Paste your key")).desired_width(200.0));
            field.widget_info(|| egui::WidgetInfo::labeled(egui::Role::TextInput, true, tr("Anthropic API key")));
            save = ui.add_enabled(!draft.text.trim().is_empty(), egui::Button::new(tr("Save"))).clicked();
            if saved {
                remove = ui.button(tr("Remove")).on_hover_text(tr("Delete the saved key")).clicked();
            }
        });
        if !draft.text.trim().is_empty() && !claude::is_plausible_key(draft.text.trim()) {
            ui.label(RichText::new(tr("That does not look like an API key. It must have no spaces.")).small().color(c.warning));
        }
        if save {
            draft.problem = self.save_key(draft.text.trim());
            if draft.problem.is_none() {
                draft.text.clear();
            }
        }
        if remove {
            draft.problem = self.remove_saved_key();
        }
        if let Some(problem) = &draft.problem {
            ui.label(RichText::new(problem).small().color(c.warning));
        }
        ui.ctx().data_mut(|data| data.insert_temp(key_draft_id(), draft));
    }

    /// Saves the key to the key file. Returns why it could not be saved, if it could not.
    fn save_key(&mut self, key: &str) -> Option<String> {
        if !claude::is_plausible_key(key) {
            return Some(tr("That does not look like an API key.").to_string());
        }
        let Some(path) = self.key_file.clone() else {
            return Some(tr("No settings folder is available, so the key cannot be saved here.").to_string());
        };
        claude::save_key(&path, key).err().map(|error| tr_fmt!("The key could not be saved: {error}", error = error))
    }

    fn remove_saved_key(&mut self) -> Option<String> {
        let path = self.key_file.clone()?;
        claude::remove_key(&path).err().map(|error| tr_fmt!("The key could not be removed: {error}", error = error))
    }
}
