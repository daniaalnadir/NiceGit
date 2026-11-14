//! Custom actions: commands the person adds to NiceGit's menus. Their arguments are split into
//! words the way a shell splits them, but no shell runs. The program starts directly, and a
//! placeholder's value stays inside the one word it appears in, so a file name with spaces or
//! `;` or `$(...)` is passed on as it is.

use std::path::Path;
use std::process::Stdio;

use egui_phosphor::regular as icon;

use crate::app::NiceGitApp;
use crate::settings::{ActionScope, CustomAction};
use crate::ui::dialogs::{Dialog, Pending};

/// How much of each output stream is kept, in bytes.
pub const KEPT_OUTPUT: usize = 200 * 1024;

/// The placeholders an argument can use, by name between braces.
pub const PLACEHOLDERS: [&str; 6] = ["repo", "branch", "sha", "short_sha", "file", "file_path"];

/// What the placeholders stand for. Each action fills in only what its scope has.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Values {
    /// The repository folder.
    pub repo: String,
    /// The checked-out branch.
    pub branch: String,
    /// The commit, in full, and its short form.
    pub sha: String,
    pub short_sha: String,
    /// The file, relative to the repository, and the same path in full.
    pub file: String,
    pub file_path: String,
}

impl Values {
    fn get(&self, name: &str) -> Option<&str> {
        match name {
            "repo" => Some(&self.repo),
            "branch" => Some(&self.branch),
            "sha" => Some(&self.sha),
            "short_sha" => Some(&self.short_sha),
            "file" => Some(&self.file),
            "file_path" => Some(&self.file_path),
            _ => None,
        }
    }
}

/// Splits `text` into words the way a shell does, without running one: whitespace separates
/// words, single quotes keep everything literal, double quotes keep spaces and accept `\"` and
/// `\\`, and a backslash outside quotes escapes only a quote, whitespace, or another backslash, so
/// a Windows path keeps its backslashes. An unclosed quote is an error. Empty text has no words.
pub fn split_arguments(text: &str) -> Result<Vec<String>, String> {
    let mut words = Vec::new();
    let mut word = String::new();
    // Set once a word has begun, so an empty quoted word such as `""` still counts.
    let mut started = false;
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        match c {
            _ if c.is_whitespace() => {
                if started {
                    words.push(std::mem::take(&mut word));
                    started = false;
                }
            }
            '\'' => {
                started = true;
                loop {
                    match chars.next() {
                        Some('\'') => break,
                        Some(c) => word.push(c),
                        None => return Err("A single quote is not closed.".into()),
                    }
                }
            }
            '"' => {
                started = true;
                loop {
                    match chars.next() {
                        Some('"') => break,
                        Some('\\') => match chars.clone().next() {
                            Some(next @ ('"' | '\\')) => {
                                chars.next();
                                word.push(next);
                            }
                            _ => word.push('\\'),
                        },
                        Some(c) => word.push(c),
                        None => return Err("A double quote is not closed.".into()),
                    }
                }
            }
            // A backslash escapes only a quote, a space, or another backslash, so a Windows path
            // such as C:\tools\bin keeps its backslashes without quoting.
            '\\' => {
                started = true;
                match chars.clone().next() {
                    Some(next) if next == '"' || next == '\'' || next == '\\' || next.is_whitespace() => {
                        chars.next();
                        word.push(next);
                    }
                    _ => word.push('\\'),
                }
            }
            _ => {
                started = true;
                word.push(c);
            }
        }
    }
    if started {
        words.push(word);
    }
    Ok(words)
}

/// Fills the placeholders in each word. A value is written once and never scanned again, so a file
/// name that contains `{branch}` stays as it is. Unknown `{...}` text is left as it was.
pub fn expand(arguments: &[String], values: &Values) -> Vec<String> {
    arguments.iter().map(|argument| fill(argument, values)).collect()
}

fn fill(text: &str, values: &Values) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        match after.find('}') {
            Some(close) if is_name(&after[..close]) => {
                match values.get(&after[..close]) {
                    Some(value) => out.push_str(value),
                    None => out.push_str(&rest[open..open + close + 2]),
                }
                rest = &after[close + 1..];
            }
            _ => {
                out.push('{');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

fn is_name(text: &str) -> bool {
    !text.is_empty() && text.chars().all(|c| c.is_ascii_lowercase() || c == '_')
}

/// The known placeholders a word uses, in order.
fn placeholder_names(text: &str) -> Vec<&str> {
    let mut names = Vec::new();
    let mut rest = text;
    while let Some(open) = rest.find('{') {
        let after = &rest[open + 1..];
        let Some(close) = after.find('}') else { break };
        let name = &after[..close];
        if PLACEHOLDERS.contains(&name) {
            names.push(name);
        }
        rest = &after[close + 1..];
    }
    names
}

/// Checks an action before it is saved: it needs a name and a program, its arguments must split,
/// and a placeholder that its scope cannot fill is refused.
pub fn validate(action: &CustomAction) -> Result<(), String> {
    if action.name.trim().is_empty() {
        return Err("Enter a name for the action.".into());
    }
    if action.program.trim().is_empty() {
        return Err("Enter the program to run, such as code or git.".into());
    }
    let words = split_arguments(&action.arguments).map_err(|error| format!("The arguments are not valid. {error}"))?;
    for word in &words {
        for name in placeholder_names(word) {
            let allowed = match name {
                // A file action from a commit's file list knows its commit too; from the Changes
                // panel it does not, and such an action is unavailable there.
                "sha" | "short_sha" => matches!(action.scope, ActionScope::Commit | ActionScope::File),
                "file" | "file_path" => action.scope == ActionScope::File,
                _ => continue,
            };
            if !allowed {
                let scopes = if matches!(name, "sha" | "short_sha") { "Commit and File" } else { "File" };
                return Err(format!("{{{name}}} is only filled in for {scopes} actions. Change the scope, or remove it."));
            }
        }
    }
    Ok(())
}

/// What a finished program left behind. `code` is `None` when a signal stopped it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Captured {
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

/// Runs `program` with `arguments` in `directory`, with NiceGit's search path and without Git's
/// repository variables inherited from the environment. Waits for it to finish.
pub fn run(program: &str, arguments: &[String], directory: &Path) -> Result<Captured, String> {
    let mut command = std::process::Command::new(program);
    command
        .args(arguments)
        .current_dir(directory)
        .env("PATH", nicegit_core::runner::search_path())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for key in std::env::vars_os().map(|(key, _)| key) {
        if key.to_str().is_some_and(nicegit_core::runner::is_removed_variable) {
            command.env_remove(key);
        }
    }
    let output = command.output().map_err(|error| format!("Could not start {program}: {error}"))?;
    Ok(Captured { code: output.status.code(), stdout: kept(&output.stdout), stderr: kept(&output.stderr) })
}

fn kept(bytes: &[u8]) -> String {
    if bytes.len() <= KEPT_OUTPUT {
        return String::from_utf8_lossy(bytes).into_owned();
    }
    let mut text = String::from_utf8_lossy(&bytes[..KEPT_OUTPUT]).into_owned();
    text.push_str("\n… output shortened.");
    text
}

/// The notice for a finished run, and whether it is an error. Success says it finished; failure
/// names the first line of error output, or the exit status when there is none.
pub fn notice(name: &str, result: &Result<Captured, String>) -> (String, bool) {
    match result {
        Err(error) => (format!("{name} failed: {error}"), true),
        Ok(captured) if captured.code == Some(0) => (format!("{name} finished."), false),
        Ok(captured) => {
            let reason = match captured.stderr.lines().map(str::trim).find(|line| !line.is_empty()) {
                Some(line) => line.to_string(),
                None => match captured.code {
                    Some(code) => format!("exit status {code}."),
                    None => "the program was stopped before it finished.".to_string(),
                },
            };
            (format!("{name} failed: {reason}"), true)
        }
    }
}

/// A run that has finished, handed from the background thread to the interface.
pub struct Outcome {
    pub name: String,
    pub show_output: bool,
    pub result: Result<Captured, String>,
}

/// The settings draft shown in Settings while an action is added or edited.
#[derive(Clone, Debug, Default)]
pub struct ActionDraft {
    /// The index of the action being edited, or `None` for a new one.
    editing: Option<usize>,
    action: CustomAction,
    error: Option<String>,
}

const DRAFT_ID: &str = "custom_action_draft";

impl NiceGitApp {
    /// The values the placeholders stand for in the open repository: its folder and branch.
    pub fn repository_values(&self) -> Values {
        Values {
            repo: self.repo().map(|repo| repo.path.display().to_string()).unwrap_or_default(),
            branch: self.snapshot().map(|s| s.current_branch.clone()).unwrap_or_default(),
            ..Values::default()
        }
    }

    /// The actions for `scope`, each with its position in the settings.
    pub fn custom_actions_in(&self, scope: ActionScope) -> Vec<(usize, CustomAction)> {
        self.settings.custom_actions.iter().cloned().enumerate().filter(|(_, action)| action.scope == scope).collect()
    }

    /// A Custom actions submenu of the actions for `scope`. Nothing is drawn when there are none.
    pub fn custom_actions_submenu(&mut self, ui: &mut egui::Ui, scope: ActionScope, values: &Values, enabled: bool) {
        let actions = self.custom_actions_in(scope);
        if actions.is_empty() {
            return;
        }
        ui.menu_button(format!("{}  Custom actions", icon::TERMINAL_WINDOW), |ui| {
            for (index, action) in actions {
                // An action that names the commit cannot run where no commit is known.
                let needs_commit = action.arguments.contains("{sha}") || action.arguments.contains("{short_sha}");
                let runnable = !needs_commit || !values.sha.is_empty();
                let button = ui.add_enabled(enabled && runnable, egui::Button::new(action.name.as_str()));
                let button =
                    if runnable { button } else { button.on_disabled_hover_text("Needs a commit: run it from a commit's file list.") };
                if button.clicked() {
                    ui.close();
                    self.start_custom_action(index, values.clone());
                }
            }
        });
    }

    /// Starts the action at `index` with `values`. An action that asks for confirmation shows its
    /// exact command first; nothing runs until it is confirmed.
    pub fn start_custom_action(&mut self, index: usize, values: Values) {
        let Some(action) = self.settings.custom_actions.get(index).cloned() else { return };
        if !self.idle() {
            return;
        }
        if self.has_unsaved_editor() {
            self.notify("Save or close the file you are editing before changing the repository.", true);
            return;
        }
        let words = match split_arguments(&action.arguments) {
            Ok(words) => words,
            Err(error) => {
                self.notify(format!("{} was not run. {error}", action.name), true);
                return;
            }
        };
        let arguments = expand(&words, &values);
        let Some(snapshot) = self.snapshot() else { return };
        let (branch, head) = (snapshot.current_branch.clone(), snapshot.head_hash.clone());
        if action.confirm {
            let name = action.name.clone();
            self.dialog = Some(Dialog::Confirm {
                title: format!("Run {name}?"),
                message: "NiceGit runs the program directly, with these arguments, in the repository's folder.".into(),
                button: "Run".into(),
                pending: Pending::RunCustomAction {
                    name,
                    program: action.program,
                    arguments,
                    show_output: action.show_output,
                    branch,
                    head,
                },
            });
        } else {
            self.run_custom_action(action.name, action.program, arguments, action.show_output, branch, head);
        }
    }

    /// Runs a program in the background, then refreshes the repository. Refused, with nothing run,
    /// when the branch or HEAD is no longer the one shown when the action was started.
    pub fn run_custom_action(
        &mut self,
        name: String,
        program: String,
        arguments: Vec<String>,
        show_output: bool,
        branch: String,
        head: Option<String>,
    ) {
        let Some(snapshot) = self.snapshot() else { return };
        if snapshot.current_branch != branch || snapshot.head_hash != head {
            self.notify("The branch or HEAD changed, so nothing ran. Start the action again.", true);
            return;
        }
        let slot = self.custom_outcome.clone();
        let label = name.clone();
        self.act(&label, move |_client, directory| {
            let result = run(&program, &arguments, directory);
            *slot.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(Outcome { name, show_output, result });
            Ok(None)
        });
    }

    /// Takes the outcome of the custom action that just finished, if there is one.
    pub fn take_custom_outcome(&mut self) -> Option<Outcome> {
        self.custom_outcome.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).take()
    }

    /// Says how a custom action finished, and shows its output when it asked to. Returns whether
    /// it failed.
    pub fn report_custom_outcome(&mut self, outcome: Outcome) -> bool {
        let (text, is_error) = notice(&outcome.name, &outcome.result);
        self.notify(text, is_error);
        if let (true, Ok(captured)) = (outcome.show_output, outcome.result) {
            self.open_tool(Box::new(crate::tools::custom_output::CustomOutputWindow::new(outcome.name, captured)));
        }
        is_error
    }

    /// The Custom actions section of Settings: the saved actions, and the form that adds or
    /// edits one.
    pub fn custom_actions_section(&mut self, ui: &mut egui::Ui) {
        let id = egui::Id::unique(DRAFT_ID);
        let c = crate::theme::of(ui);
        let mut draft: Option<ActionDraft> = ui.ctx().data(|data| data.get_temp(id));
        ui.label(
            egui::RichText::new(
                "Commands that run from the Repository, Commit, and File menus and the command palette. They start directly, never through a shell.",
            )
            .small()
            .color(c.muted),
        );
        ui.add_space(4.0);
        if self.settings.custom_actions.is_empty() {
            ui.label(egui::RichText::new("No custom actions yet.").color(c.muted));
        }
        let mut edit = None;
        let mut remove = None;
        for (index, action) in self.settings.custom_actions.iter().enumerate() {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(&action.name).strong());
                crate::tools::widgets::pill(ui, action.scope.title(), c.accent);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Remove").clicked() {
                        remove = Some(index);
                    }
                    if ui.button("Edit").clicked() {
                        edit = Some(index);
                    }
                });
            });
        }
        if let Some(index) = edit {
            draft = Some(ActionDraft { editing: Some(index), action: self.settings.custom_actions[index].clone(), error: None });
        }
        if let Some(index) = remove {
            self.settings.custom_actions.remove(index);
            draft = None;
        }
        let mut save = false;
        let mut cancel = false;
        match draft.as_mut() {
            Some(form) => {
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.label("Name");
                    let field = ui.add(egui::TextEdit::singleline(&mut form.action.name).desired_width(260.0));
                    field.widget_info(|| egui::WidgetInfo::labeled(egui::Role::TextInput, true, "Name"));
                });
                ui.horizontal(|ui| {
                    ui.label("Program");
                    let field = ui.add(
                        egui::TextEdit::singleline(&mut form.action.program)
                            .hint_text("code, git, /usr/local/bin/script.sh")
                            .desired_width(260.0),
                    );
                    field.widget_info(|| egui::WidgetInfo::labeled(egui::Role::TextInput, true, "Program"));
                });
                ui.horizontal(|ui| {
                    ui.label("Arguments");
                    let field = ui.add(
                        egui::TextEdit::singleline(&mut form.action.arguments)
                            .hint_text("tag custom-{short_sha} {sha}")
                            .desired_width(260.0),
                    );
                    field.widget_info(|| egui::WidgetInfo::labeled(egui::Role::TextInput, true, "Arguments"));
                });
                ui.horizontal(|ui| {
                    ui.label("Scope");
                    for scope in ActionScope::ALL {
                        ui.selectable_value(&mut form.action.scope, scope, scope.title());
                    }
                });
                crate::tools::widgets::checkbox(ui, true, &mut form.action.show_output, "Show output");
                crate::tools::widgets::checkbox(ui, true, &mut form.action.confirm, "Ask before running");
                ui.label(
                    egui::RichText::new(
                        "Placeholders: {repo} repository folder, {branch} current branch, {sha} commit, {short_sha} short commit ID, \
                         {file} file path relative to the repository, {file_path} absolute file path. {sha} and {short_sha} need a \
                         Commit or File action (a File action using them runs from a commit's file list); {file} and {file_path} \
                         need a File action. Quote words that contain spaces. A backslash before a quote, a space, or another \
                         backslash keeps that character as it is.",
                    )
                    .small()
                    .color(c.muted),
                );
                if let Some(error) = &form.error {
                    ui.label(egui::RichText::new(error).color(c.warning));
                }
                ui.horizontal(|ui| {
                    if crate::tools::widgets::primary_button(ui, "Save action", true).clicked() {
                        save = true;
                    }
                    if ui.button("Cancel").clicked() {
                        cancel = true;
                    }
                });
            }
            None => {
                if ui.button(format!("{}  Add action", icon::PLUS)).clicked() {
                    draft = Some(ActionDraft::default());
                }
            }
        }
        if save {
            if let Some(form) = draft.as_mut() {
                let trimmed = CustomAction {
                    name: form.action.name.trim().to_string(),
                    program: form.action.program.trim().to_string(),
                    ..form.action.clone()
                };
                match validate(&trimmed) {
                    Ok(()) => {
                        match form.editing {
                            Some(index) if index < self.settings.custom_actions.len() => self.settings.custom_actions[index] = trimmed,
                            _ => self.settings.custom_actions.push(trimmed),
                        }
                        draft = None;
                    }
                    // The entry is not saved; the form stays open with the reason.
                    Err(error) => form.error = Some(error),
                }
            }
        }
        if cancel {
            draft = None;
        }
        ui.ctx().data_mut(|data| match draft {
            Some(draft) => {
                data.insert_temp(id, draft);
            }
            None => data.remove::<ActionDraft>(id),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(text: &str) -> Vec<String> {
        split_arguments(text).expect("splits")
    }

    fn values() -> Values {
        Values {
            repo: "/work/repo".into(),
            branch: "main".into(),
            sha: "0123456789abcdef0123456789abcdef01234567".into(),
            short_sha: "0123456".into(),
            file: "a b;c.txt".into(),
            file_path: "/work/repo/a b;c.txt".into(),
        }
    }

    fn action(scope: ActionScope, arguments: &str) -> CustomAction {
        CustomAction { name: "Run it".into(), program: "git".into(), arguments: arguments.into(), scope, ..Default::default() }
    }

    #[test]
    fn splitting_follows_shell_quoting_without_a_shell() {
        assert_eq!(words("tag custom-1 abc"), ["tag", "custom-1", "abc"]);
        assert_eq!(words("  spaced   out  "), ["spaced", "out"]);
        assert_eq!(words("\"a b\" c"), ["a b", "c"]);
        assert_eq!(words("'x y' z"), ["x y", "z"]);
        assert_eq!(words("a\\ b"), ["a b"]);
        assert_eq!(words("\"quote \\\" and \\\\\""), ["quote \" and \\"]);
        assert_eq!(words("\"back\\slash\""), ["back\\slash"]);
        assert_eq!(words("'back\\slash'"), ["back\\slash"]);
        assert_eq!(words("\"\""), [""]);
        assert_eq!(words("''"), [""]);
        assert_eq!(words("a\"\"b"), ["ab"]);
        assert!(words("").is_empty());
        assert!(words("   ").is_empty());
    }

    #[test]
    fn unbalanced_quotes_and_a_trailing_backslash_are_errors() {
        assert!(split_arguments("\"open").unwrap_err().contains("double quote"));
        assert!(split_arguments("'open").unwrap_err().contains("single quote"));
    }

    #[test]
    fn windows_paths_keep_their_backslashes() {
        assert_eq!(words(r"C:\tools\bin\run.exe --flag"), [r"C:\tools\bin\run.exe", "--flag"]);
        assert_eq!(words(r#"a\ b \"q\" end\"#), ["a b", "\"q\"", "end\\"]);
        assert_eq!(words(r"two\\back"), [r"two\back"]);
    }

    #[test]
    fn a_file_name_with_shell_characters_stays_one_literal_argument() {
        let arguments = expand(&words("add -- {file}"), &values());
        assert_eq!(arguments, ["add", "--", "a b;c.txt"]);

        let hostile = Values { file: "x; rm -rf /".into(), ..values() };
        assert_eq!(expand(&["{file}".to_string()], &hostile), ["x; rm -rf /"]);

        let command_substitution = Values { file: "$(rm -rf /)".into(), ..values() };
        assert_eq!(expand(&["{file}".to_string()], &command_substitution), ["$(rm -rf /)"]);

        let quotes = Values { file: "it's \"quoted\"".into(), ..values() };
        assert_eq!(expand(&["{file}".to_string()], &quotes), ["it's \"quoted\""]);
    }

    #[test]
    fn a_value_is_never_filled_again() {
        let tricky = Values { file: "{branch}".into(), ..values() };
        assert_eq!(expand(&["{file}".to_string()], &tricky), ["{branch}"]);
    }

    #[test]
    fn placeholders_fill_inside_a_word_and_unknown_text_stays() {
        let arguments = expand(&words("tag custom-{short_sha} {sha} {nope} {}"), &values());
        assert_eq!(arguments, ["tag", "custom-0123456", "0123456789abcdef0123456789abcdef01234567", "{nope}", "{}"]);
        assert_eq!(expand(&["{repo}:{branch}".to_string()], &values()), ["/work/repo:main"]);
        assert_eq!(expand(&["open{".to_string(), "{branch".to_string()], &values()), ["open{", "{branch"]);
    }

    #[test]
    fn each_scope_accepts_only_its_placeholders() {
        assert!(validate(&action(ActionScope::Repository, "status {branch} {repo}")).is_ok());
        assert!(validate(&action(ActionScope::Commit, "tag x-{short_sha} {sha} {branch}")).is_ok());
        assert!(validate(&action(ActionScope::File, "add -- {file} {file_path}")).is_ok());

        let sha_outside = validate(&action(ActionScope::Repository, "tag {sha}")).unwrap_err();
        assert!(sha_outside.contains("{sha}") && sha_outside.contains("Commit"), "{sha_outside}");
        assert!(validate(&action(ActionScope::File, "show {short_sha}:{file}")).is_ok(), "a File action may name its commit");
        assert!(validate(&action(ActionScope::Commit, "add {file}")).is_err());
        assert!(validate(&action(ActionScope::Repository, "add {file_path}")).is_err());
    }

    #[test]
    fn an_action_needs_a_name_a_program_and_valid_arguments() {
        let no_name = CustomAction { name: "  ".into(), ..action(ActionScope::Repository, "status") };
        assert!(validate(&no_name).is_err());
        let no_program = CustomAction { program: String::new(), ..action(ActionScope::Repository, "status") };
        assert!(validate(&no_program).is_err());
        let unbalanced = validate(&action(ActionScope::Repository, "tag \"open")).unwrap_err();
        assert!(unbalanced.contains("not valid"), "{unbalanced}");
        assert!(validate(&action(ActionScope::Repository, "")).is_ok());
    }

    #[test]
    fn notices_name_the_first_error_line_or_the_exit_status() {
        let ok = Ok(Captured { code: Some(0), ..Default::default() });
        assert_eq!(notice("Tag it", &ok), ("Tag it finished.".to_string(), false));

        let stderr = Ok(Captured { code: Some(128), stderr: "\n  fatal: bad revision\nmore\n".into(), ..Default::default() });
        assert_eq!(notice("Tag it", &stderr), ("Tag it failed: fatal: bad revision".to_string(), true));

        let silent = Ok(Captured { code: Some(3), ..Default::default() });
        assert_eq!(notice("Tag it", &silent), ("Tag it failed: exit status 3.".to_string(), true));

        let signalled = Ok(Captured { code: None, ..Default::default() });
        assert!(notice("Tag it", &signalled).0.starts_with("Tag it failed: "));

        let spawn = Err("Could not start nothing: not found".to_string());
        assert_eq!(notice("Tag it", &spawn), ("Tag it failed: Could not start nothing: not found".to_string(), true));
    }

    #[test]
    fn a_program_runs_directly_in_its_folder() {
        let dir = std::env::temp_dir();
        let result = run("git", &["--version".to_string()], &dir).expect("git runs");
        assert_eq!(result.code, Some(0));
        assert!(result.stdout.starts_with("git version"), "{}", result.stdout);
        let missing = run("nicegit-no-such-program", &[], &dir);
        assert!(missing.unwrap_err().contains("Could not start"));
    }

    #[test]
    fn kept_output_is_limited() {
        let long = vec![b'a'; KEPT_OUTPUT + 10];
        let text = kept(&long);
        assert!(text.len() < KEPT_OUTPUT + 40);
        assert!(text.ends_with("output shortened."));
        assert_eq!(kept(b"short"), "short");
    }
}
