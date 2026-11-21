//! Bitbucket and Azure DevOps in Settings: the credentials each service uses to read and create pull
//! requests. A Bitbucket username and app password, or API token, are saved together as one entry. An Azure
//! DevOps personal access token is saved under its organization. Each is written to the token file, never to
//! Settings, and is shown only as saved or not. The text being typed is kept while Settings is open and then
//! forgotten.

use std::collections::BTreeMap;
use std::path::Path;

use egui::RichText;
use nicegit_core::bitbucket::{self, HOST as BITBUCKET_HOST};
use nicegit_core::gitea::is_plausible_token;

use crate::app::NiceGitApp;
use crate::forge_tokens;
use crate::i18n::{tr, tr_fmt};
use crate::theme;
use crate::tools::widgets;

/// The key of the Bitbucket entry in the drafts, and the Azure DevOps one.
const BITBUCKET: &str = "bitbucket";
const AZURE: &str = "azure";

/// What is being typed for one service, and why the last save or removal failed.
#[derive(Clone, Debug, Default)]
struct Draft {
    username: String,
    secret: String,
    organization: String,
    problem: Option<String>,
}

/// The typed text by service, kept while Settings is open.
type Drafts = BTreeMap<String, Draft>;

fn drafts_id() -> egui::Id {
    egui::Id::unique("forge_credential_drafts")
}

/// Forgets the text being typed, so it does not stay in memory after Settings closes.
pub fn forget_drafts(ctx: &egui::Context) {
    ctx.data_mut(|data| data.remove::<Drafts>(drafts_id()));
}

/// Whether a Bitbucket username can be saved: not empty, and with no colon, space, or control character.
fn plain_username(username: &str) -> bool {
    !username.is_empty() && !username.chars().any(|c| c == ':' || c.is_whitespace() || c.is_control())
}

/// Whether an Azure DevOps organization name can be used: letters, digits, and hyphens.
fn plain_organization(organization: &str) -> bool {
    !organization.is_empty() && organization.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

/// Saves a Bitbucket username and secret together. The problem is worded, or none when saved. Neither
/// text is part of any message.
fn save_bitbucket(path: Option<&Path>, username: &str, secret: &str) -> Option<String> {
    if !plain_username(username) {
        return Some(tr("The username must have no spaces, colons, or control characters.").to_string());
    }
    if !is_plausible_token(secret) {
        return Some(tr("That does not look like a token. It must have no spaces.").to_string());
    }
    let Some(path) = path else {
        return Some(tr("No settings folder is available, so the token cannot be saved here.").to_string());
    };
    let Some(text) = bitbucket::credentials_text(username, secret) else {
        return Some(tr("That does not look like a token. It must have no spaces.").to_string());
    };
    forge_tokens::save_token(path, BITBUCKET_HOST, &text)
        .err()
        .map(|error| tr_fmt!("The credentials for {host} could not be saved: {error}", host = BITBUCKET_HOST, error = error))
}

/// Saves an Azure DevOps personal access token for an organization. The problem is worded, or none when saved.
fn save_azure(path: Option<&Path>, organization: &str, token: &str) -> Option<String> {
    if !plain_organization(organization) {
        return Some(tr("The organization name must use only letters, digits, and hyphens.").to_string());
    }
    if !is_plausible_token(token) {
        return Some(tr("That does not look like a token. It must have no spaces.").to_string());
    }
    let Some(path) = path else {
        return Some(tr("No settings folder is available, so the token cannot be saved here.").to_string());
    };
    let key = forge_tokens::azure_key(organization);
    forge_tokens::save_token(path, &key, token)
        .err()
        .map(|error| tr_fmt!("The token for {host} could not be saved: {error}", host = key, error = error))
}

/// The problem removing a saved entry, or none when it is removed. Bitbucket's and Azure DevOps's entries
/// are removed the same way, by their key.
fn remove_problem(path: Option<&Path>, key: &str) -> Option<String> {
    let path = path?;
    forge_tokens::remove_token(path, key)
        .err()
        .map(|error| tr_fmt!("The token for {host} could not be removed: {error}", host = key, error = error))
}

impl NiceGitApp {
    /// The Bitbucket and Azure DevOps sections of Settings, after the Gitea and Forgejo section.
    pub fn forge_credential_sections(&mut self, ui: &mut egui::Ui) {
        self.bitbucket_section(ui);
        ui.add_space(10.0);
        self.azure_section(ui);
    }

    /// The Bitbucket section: the username and app password for bitbucket.org, with Save and Remove.
    fn bitbucket_section(&mut self, ui: &mut egui::Ui) {
        let c = theme::of(ui);
        widgets::section(ui, tr("Bitbucket"));
        ui.label(
            RichText::new(tr("Remotes on bitbucket.org get pull requests and issues. Public repositories can be read without credentials; creating a pull request needs your username and an app password or API token."))
                .small()
                .color(c.muted),
        );
        ui.add_space(4.0);
        let path = self.forge_token_file.clone();
        let mut drafts: Drafts = ui.ctx().data(|data| data.get_temp(drafts_id())).unwrap_or_default();
        let draft = drafts.entry(BITBUCKET.to_string()).or_default();
        let saved = forge_tokens::saved_for(path.as_deref(), BITBUCKET_HOST).is_some();
        let (mut save, mut remove) = (false, false);
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(BITBUCKET_HOST).strong());
            ui.label(RichText::new(if saved { tr("A token is saved.") } else { tr("No token is saved.") }).small().color(c.muted));
            let username = ui.add(egui::TextEdit::singleline(&mut draft.username).hint_text(tr("Username")).desired_width(130.0));
            let username_label = tr_fmt!("Username for {host}", host = BITBUCKET_HOST);
            username.widget_info(move || egui::WidgetInfo::labeled(egui::Role::TextInput, true, username_label.clone()));
            let secret = ui.add(
                egui::TextEdit::singleline(&mut draft.secret)
                    .password(true)
                    .hint_text(tr("App password or API token"))
                    .desired_width(160.0),
            );
            let secret_label = tr_fmt!("App password or API token for {host}", host = BITBUCKET_HOST);
            secret.widget_info(move || egui::WidgetInfo::labeled(egui::Role::TextInput, true, secret_label.clone()));
            let ready = !draft.username.trim().is_empty() && !draft.secret.trim().is_empty();
            let save_button = ui.add_enabled(path.is_some() && ready, egui::Button::new(tr("Save")));
            let save_label = tr_fmt!("Save the credentials for {host}", host = BITBUCKET_HOST);
            save_button.widget_info(move || egui::WidgetInfo::labeled(egui::Role::Button, true, save_label.clone()));
            save = save_button.clicked();
            if saved {
                let remove_button = ui.button(tr("Remove")).on_hover_text(tr("Delete the saved token"));
                let remove_label = tr_fmt!("Remove the credentials for {host}", host = BITBUCKET_HOST);
                remove_button.widget_info(move || egui::WidgetInfo::labeled(egui::Role::Button, true, remove_label.clone()));
                remove = remove_button.clicked();
            }
        });
        if save {
            draft.problem = save_bitbucket(path.as_deref(), draft.username.trim(), draft.secret.trim());
            if draft.problem.is_none() {
                draft.username.clear();
                draft.secret.clear();
            }
        }
        if remove {
            draft.problem = remove_problem(path.as_deref(), BITBUCKET_HOST);
        }
        if let Some(problem) = &draft.problem {
            ui.label(RichText::new(problem).small().color(c.warning));
        }
        ui.ctx().data_mut(|data| data.insert_temp(drafts_id(), drafts));
    }

    /// The Azure DevOps section: a personal access token for each organization, typed with its name, and a
    /// row for each organization already saved, with Remove.
    fn azure_section(&mut self, ui: &mut egui::Ui) {
        let c = theme::of(ui);
        widgets::section(ui, tr("Azure DevOps"));
        ui.label(
            RichText::new(tr("Azure DevOps lists nothing without a personal access token for the organization. Creating a pull request needs one with code read and write access."))
                .small()
                .color(c.muted),
        );
        ui.add_space(4.0);
        let path = self.forge_token_file.clone();
        let mut drafts: Drafts = ui.ctx().data(|data| data.get_temp(drafts_id())).unwrap_or_default();
        let draft = drafts.entry(AZURE.to_string()).or_default();
        let mut save = false;
        ui.horizontal_wrapped(|ui| {
            let organization = ui
                .add(egui::TextEdit::singleline(&mut draft.organization).hint_text(tr("Organization, such as acme")).desired_width(150.0));
            organization.widget_info(|| egui::WidgetInfo::labeled(egui::Role::TextInput, true, tr("Organization")));
            let token = ui.add(
                egui::TextEdit::singleline(&mut draft.secret).password(true).hint_text(tr("Personal access token")).desired_width(160.0),
            );
            token.widget_info(|| egui::WidgetInfo::labeled(egui::Role::TextInput, true, tr("Personal access token")));
            let ready = !draft.organization.trim().is_empty() && !draft.secret.trim().is_empty();
            let key = forge_tokens::azure_key(draft.organization.trim());
            let save_button = ui.add_enabled(path.is_some() && ready, egui::Button::new(tr("Save")));
            let save_label = tr_fmt!("Save the token for {host}", host = key);
            save_button.widget_info(move || egui::WidgetInfo::labeled(egui::Role::Button, true, save_label.clone()));
            save = save_button.clicked();
        });
        if save {
            draft.problem = save_azure(path.as_deref(), draft.organization.trim(), draft.secret.trim());
            if draft.problem.is_none() {
                draft.organization.clear();
                draft.secret.clear();
            }
        }
        let mut removed: Option<String> = None;
        for organization in forge_tokens::saved_azure_organizations(path.as_deref()) {
            let key = forge_tokens::azure_key(&organization);
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(&key).strong());
                ui.label(RichText::new(tr("A token is saved.")).small().color(c.muted));
                let remove_button = ui.button(tr("Remove")).on_hover_text(tr("Delete the saved token"));
                let remove_label = tr_fmt!("Remove the token for {host}", host = key.clone());
                remove_button.widget_info(move || egui::WidgetInfo::labeled(egui::Role::Button, true, remove_label.clone()));
                if remove_button.clicked() {
                    removed = Some(key);
                }
            });
        }
        if let Some(key) = removed {
            draft.problem = remove_problem(path.as_deref(), &key);
        }
        if let Some(problem) = &draft.problem {
            ui.label(RichText::new(problem).small().color(c.warning));
        }
        ui.ctx().data_mut(|data| data.insert_temp(drafts_id(), drafts));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_problem_never_repeats_the_secret() {
        let refused = save_bitbucket(None, "octo", "secret-app-password").expect("no folder");
        assert!(!refused.contains("secret-app-password"), "{refused}");
        let spaced = save_bitbucket(None, "octo", "has space secret").expect("not plausible");
        assert!(!spaced.contains("secret"), "{spaced}");
        let colon = save_bitbucket(None, "oc:to", "secret-value").expect("the username cannot hold a colon");
        assert!(colon.contains("username"), "{colon}");
        let azure = save_azure(None, "acme", "secret-pat").expect("no folder");
        assert!(!azure.contains("secret-pat"), "{azure}");
    }

    #[test]
    fn credentials_are_saved_and_removed_per_service() {
        let dir = tempfile::tempdir().expect("temporary folder");
        let path = dir.path().join("forge-tokens.ron");
        assert_eq!(save_bitbucket(Some(&path), "octo", "app-password"), None);
        assert_eq!(forge_tokens::bitbucket_credentials(Some(&path)).as_deref(), Some("octo:app-password"));
        assert_eq!(save_azure(Some(&path), "Acme", "azure-pat"), None);
        assert_eq!(forge_tokens::saved_azure_organizations(Some(&path)), vec!["acme".to_string()]);
        assert_eq!(remove_problem(Some(&path), BITBUCKET_HOST), None);
        assert_eq!(forge_tokens::bitbucket_credentials(Some(&path)), None);
        assert_eq!(forge_tokens::saved_azure_organizations(Some(&path)).len(), 1, "Azure keeps its own entry");
    }

    #[test]
    fn organization_names_are_letters_digits_and_hyphens() {
        assert!(plain_organization("acme-1"));
        assert!(!plain_organization("acme/web"));
        assert!(!plain_organization(""));
        assert!(plain_username("octo.user"));
        assert!(!plain_username("octo user"));
    }
}
