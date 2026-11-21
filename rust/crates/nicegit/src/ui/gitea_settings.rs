//! Gitea and Forgejo in Settings: the self-hosted hosts whose remotes get pull requests and issues, and
//! the access token each host uses. A token is written to its own file, never to Settings, and is
//! shown only as saved or not. The token being typed is kept while Settings is open and then forgotten.

use std::collections::BTreeMap;
use std::path::Path;

use egui::RichText;
use nicegit_core::gitea::is_plausible_token;
use nicegit_core::hosting::ALWAYS_GITEA;

use crate::app::NiceGitApp;
use crate::forge_tokens;
use crate::i18n::{tr, tr_fmt};
use crate::theme;
use crate::tools::widgets;

/// A token as it is being typed for one host, and why the last save or removal failed.
#[derive(Clone, Debug, Default)]
struct TokenDraft {
    text: String,
    problem: Option<String>,
}

/// The typed tokens by host, kept while Settings is open.
type TokenDrafts = BTreeMap<String, TokenDraft>;

fn token_drafts_id() -> egui::Id {
    egui::Id::unique("gitea_token_drafts")
}

fn hosts_draft_id() -> egui::Id {
    egui::Id::unique("gitea_hosts_draft")
}

/// Forgets the host list and the tokens being typed, so they do not stay in memory after Settings closes.
pub fn forget_drafts(ctx: &egui::Context) {
    ctx.data_mut(|data| {
        data.remove::<TokenDrafts>(token_drafts_id());
        data.remove::<String>(hosts_draft_id());
    });
}

/// Where a token problem is worded: the file cannot be used, the token is not plausible, or the file
/// reported an error. The token text is never part of a message.
fn save_problem(path: Option<&Path>, host: &str, token: &str) -> Option<String> {
    if !is_plausible_token(token) {
        return Some(tr("That does not look like a token. It must have no spaces.").to_string());
    }
    let Some(path) = path else {
        return Some(tr("No settings folder is available, so the token cannot be saved here.").to_string());
    };
    forge_tokens::save_token(path, host, token)
        .err()
        .map(|error| tr_fmt!("The token for {host} could not be saved: {error}", host = host, error = error))
}

/// The problem removing a host's token, or `None` when it was removed.
fn remove_problem(path: Option<&Path>, host: &str) -> Option<String> {
    let path = path?;
    forge_tokens::remove_token(path, host)
        .err()
        .map(|error| tr_fmt!("The token for {host} could not be removed: {error}", host = host, error = error))
}

impl NiceGitApp {
    /// The Gitea and Forgejo section of Settings: the hosts listed by the person, then a token row for
    /// each host that can be used.
    pub fn gitea_section(&mut self, ui: &mut egui::Ui) {
        let c = theme::of(ui);
        widgets::section(ui, tr("Gitea and Forgejo"));
        ui.label(
            RichText::new(tr("Remotes on these hosts get pull requests, issues, and Open on links. Public repositories can be read without a token; creating a pull request needs one."))
                .small()
                .color(c.muted),
        );
        ui.add_space(4.0);
        let mut draft =
            ui.ctx().data(|data| data.get_temp::<String>(hosts_draft_id())).unwrap_or_else(|| self.settings.gitea_hosts.join("\n"));
        let field = ui.add(
            egui::TextEdit::multiline(&mut draft)
                .hint_text(tr("One hostname per line, such as git.example.com"))
                .desired_rows(3)
                .desired_width(f32::INFINITY),
        );
        field.widget_info(|| egui::WidgetInfo::labeled(egui::Role::TextInput, true, tr("Gitea and Forgejo hosts")));
        let (valid, invalid) = crate::settings::parse_gitea_hosts(&draft);
        if valid != self.settings.gitea_hosts {
            self.settings.gitea_hosts = valid;
        }
        if !invalid.is_empty() {
            let text = tr_fmt!(
                "Not saved, as these are not hostnames: {hosts}. Use a name such as git.example.com, without https:// or a port.",
                hosts = invalid.join(", ")
            );
            ui.label(RichText::new(text).small().color(c.warning));
        }
        ui.ctx().data_mut(|data| data.insert_temp(hosts_draft_id(), draft));

        ui.add_space(6.0);
        let path = self.forge_token_file.clone();
        if path.is_none() {
            ui.label(RichText::new(tr("No settings folder is available, so tokens cannot be saved here.")).small().color(c.warning));
        }
        if forge_tokens::environment_token().is_some() {
            ui.label(
                RichText::new(tr_fmt!(
                    "Using {variable} from the environment for hosts with no saved token.",
                    variable = forge_tokens::ENVIRONMENT
                ))
                .small()
                .color(c.muted),
            );
        }
        let mut hosts: Vec<String> = ALWAYS_GITEA.iter().map(|host| host.to_string()).collect();
        hosts.extend(self.settings.gitea_hosts.iter().cloned());
        for host in hosts {
            self.gitea_token_row(ui, path.as_deref(), &host);
        }
    }

    /// One host's token on one wrapped line: the host, whether a token is saved, a password field,
    /// and Save and Remove. Each button is named for its host, so the rows tell apart for screen readers.
    fn gitea_token_row(&mut self, ui: &mut egui::Ui, path: Option<&Path>, host: &str) {
        let c = theme::of(ui);
        let mut drafts: TokenDrafts = ui.ctx().data(|data| data.get_temp(token_drafts_id())).unwrap_or_default();
        let draft = drafts.entry(host.to_string()).or_default();
        let saved = forge_tokens::has_saved(path, host);
        let (mut save, mut remove) = (false, false);
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(host).strong());
            ui.label(RichText::new(if saved { tr("A token is saved.") } else { tr("No token is saved.") }).small().color(c.muted));
            let field =
                ui.add(egui::TextEdit::singleline(&mut draft.text).password(true).hint_text(tr("Paste a token")).desired_width(160.0));
            let field_label = tr_fmt!("Token for {host}", host = host);
            field.widget_info(move || egui::WidgetInfo::labeled(egui::Role::TextInput, true, field_label.clone()));
            let save_button = ui.add_enabled(path.is_some() && !draft.text.trim().is_empty(), egui::Button::new(tr("Save")));
            let save_label = tr_fmt!("Save the token for {host}", host = host);
            save_button.widget_info(move || egui::WidgetInfo::labeled(egui::Role::Button, true, save_label.clone()));
            save = save_button.clicked();
            if saved {
                let remove_button = ui.button(tr("Remove")).on_hover_text(tr("Delete the saved token"));
                let remove_label = tr_fmt!("Remove the token for {host}", host = host);
                remove_button.widget_info(move || egui::WidgetInfo::labeled(egui::Role::Button, true, remove_label.clone()));
                remove = remove_button.clicked();
            }
        });
        if save {
            draft.problem = save_problem(path, host, draft.text.trim());
            if draft.problem.is_none() {
                draft.text.clear();
            }
        }
        if remove {
            draft.problem = remove_problem(path, host);
        }
        if let Some(problem) = &draft.problem {
            ui.label(RichText::new(problem).small().color(c.warning));
        }
        ui.ctx().data_mut(|data| data.insert_temp(token_drafts_id(), drafts));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_token_problem_never_repeats_the_token() {
        let problem = save_problem(None, "codeberg.org", "secret-token-value").expect("no folder");
        assert!(!problem.contains("secret-token-value"));
        let refused = save_problem(None, "codeberg.org", "has space secret").expect("not plausible");
        assert!(!refused.contains("secret"), "{refused}");
    }

    #[test]
    fn a_token_is_saved_through_the_settings_row_rules() {
        let dir = tempfile::tempdir().expect("temporary folder");
        let path = dir.path().join("forge-tokens.ron");
        assert_eq!(save_problem(Some(&path), "codeberg.org", "abc123"), None);
        assert!(forge_tokens::has_saved(Some(&path), "codeberg.org"));
        assert_eq!(remove_problem(Some(&path), "codeberg.org"), None);
        assert!(!forge_tokens::has_saved(Some(&path), "codeberg.org"));
    }
}
