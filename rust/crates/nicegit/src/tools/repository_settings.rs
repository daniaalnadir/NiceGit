#![allow(dead_code)]
//! Repository settings: the commit identity for this repository, saved identity profiles, and
//! remotes.

use std::path::{Path, PathBuf};

use egui::{Context, RichText, Ui};
use egui_phosphor::regular as icon;
use nicegit_core::settings::IdentityProfile;
use nicegit_core::{GitError, Snapshot};
use serde::{Deserialize, Serialize};

use crate::theme;
use crate::tools::{query, widgets, Ctx, Task, ToolWindow};

/// Where saved identity profiles are kept in egui's persisted memory. Profiles belong to the
/// application rather than to any one repository.
pub const PROFILES_KEY: &str = "nicegit.identity_profiles";

/// An identity profile as it is saved between sessions. The core profile type has no serde
/// support, so the window keeps its own serialisable copy and converts it for display.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct StoredProfile {
    pub name: String,
    pub email: String,
    pub signing_key: Option<String>,
}

impl StoredProfile {
    fn to_core(&self) -> IdentityProfile {
        IdentityProfile::new(&self.name, &self.email, self.signing_key.as_deref())
    }

    fn matches(&self, other: &StoredProfile) -> bool {
        self.name == other.name && self.email == other.email
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Section {
    Identity,
    Profiles,
    Remotes,
}

/// A remote's edit in progress, with the name it had when editing started.
#[derive(Clone, Debug)]
struct RemoteEdit {
    original: String,
    name: String,
    address: String,
}

/// A button press on one remote card, handled once the card is drawn.
enum RemoteAction {
    StartEdit,
    CancelEdit,
    SaveEdit,
    AskRemove,
    CancelRemove,
    ConfirmRemove,
}

/// A remote action whose outcome the next repository snapshot shows. The form closes or clears
/// only when the snapshot matches what the action should have produced, so a failed action keeps
/// what the user typed.
enum RemotePending {
    Add(String),
    Edit { name: String, address: String },
    Remove(String),
}

/// Edits the commit identity of the open repository, keeps saved identity profiles, and manages
/// its remotes. Every change is confirmed against the state shown when it was chosen.
pub struct RepositorySettingsWindow {
    section: Section,
    /// The repository the identity fields were read from; a different repository reloads them.
    repo: Option<PathBuf>,
    name: String,
    email: String,
    signing_key: String,
    identity_task: Option<Task<nicegit_core::Result<(String, String)>>>,
    identity_error: Option<String>,
    reload_identity: bool,
    /// Set when an identity action starts; the next snapshot reloads the fields from Git.
    identity_refresh_pending: bool,
    profiles: Vec<StoredProfile>,
    profiles_loaded: bool,
    edit: Option<RemoteEdit>,
    confirming_removal: Option<String>,
    new_remote_name: String,
    new_remote_address: String,
    pending: Option<RemotePending>,
}

impl RepositorySettingsWindow {
    pub fn new() -> Self {
        Self {
            section: Section::Identity,
            repo: None,
            name: String::new(),
            email: String::new(),
            signing_key: String::new(),
            identity_task: None,
            identity_error: None,
            reload_identity: true,
            identity_refresh_pending: false,
            profiles: Vec::new(),
            profiles_loaded: false,
            edit: None,
            confirming_removal: None,
            new_remote_name: String::new(),
            new_remote_address: String::new(),
            pending: None,
        }
    }

    /// Starts reading the identity when the repository changes, and stores the result when it
    /// arrives.
    fn sync_identity(&mut self, ctx: &Context, repo: &Path) {
        if self.repo.as_deref() != Some(repo) {
            self.repo = Some(repo.to_path_buf());
            self.reload_identity = true;
            self.identity_task = None;
        }
        if self.reload_identity && self.identity_task.is_none() {
            self.reload_identity = false;
            self.identity_task = Some(query(ctx, repo, |client, dir| Ok::<_, GitError>(client.identity(dir))));
        }
        if let Some(task) = self.identity_task.as_mut() {
            if let Some(result) = task.get().cloned() {
                self.identity_task = None;
                match result {
                    Ok((name, email)) => {
                        self.name = name;
                        self.email = email;
                        self.identity_error = None;
                    }
                    Err(error) => self.identity_error = Some(error.to_string()),
                }
            }
        }
    }

    fn load_profiles(&mut self, ctx: &Context) {
        if self.profiles_loaded {
            return;
        }
        self.profiles_loaded = true;
        self.profiles = ctx.data_mut(|data| data.get_persisted::<Vec<StoredProfile>>(egui::Id::new(PROFILES_KEY))).unwrap_or_default();
    }

    fn save_profiles(&self, ctx: &Context) {
        ctx.data_mut(|data| data.insert_persisted(egui::Id::new(PROFILES_KEY), self.profiles.clone()));
    }

    fn header(&mut self, ui: &mut Ui, cx: &Ctx) {
        let c = theme::of(ui);
        ui.label(RichText::new(cx.repo.display().to_string()).monospace().small().color(c.muted));
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            let tabs = [
                (Section::Identity, icon::USER_CIRCLE, "Identity"),
                (Section::Profiles, icon::IDENTIFICATION_CARD, "Profiles"),
                (Section::Remotes, icon::CLOUD, "Remotes"),
            ];
            for (section, glyph, text) in tabs {
                ui.selectable_value(&mut self.section, section, format!("{glyph}  {text}"));
            }
        });
        ui.separator();
    }

    fn identity_section(&mut self, ui: &mut Ui, cx: &mut Ctx) {
        let idle = cx.idle;
        let loading = self.identity_task.is_some();
        let editable = idle && !loading;
        widgets::section(ui, "Commit identity");
        ui.label(
            RichText::new("Commits made in this repository use this name and email. Other repositories keep their own.")
                .color(theme::of(ui).muted),
        );
        ui.add_space(8.0);
        if loading {
            widgets::loading(ui, "Reading the identity");
        }
        egui::Grid::new("repository_identity_form").num_columns(2).spacing([16.0, 10.0]).show(ui, |ui| {
            ui.label("Name");
            ui.add_enabled(editable, egui::TextEdit::singleline(&mut self.name).hint_text("Ada Lovelace").desired_width(320.0))
                .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, "Name"));
            ui.end_row();
            ui.label("Email");
            ui.add_enabled(editable, egui::TextEdit::singleline(&mut self.email).hint_text("ada@example.com").desired_width(320.0))
                .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, "Email"));
            ui.end_row();
        });
        if let Some(error) = self.identity_error.clone() {
            ui.add_space(6.0);
            widgets::error(ui, &error);
        }
        ui.add_space(10.0);
        let ready = editable && valid_identity(&self.name, &self.email);
        let save = ui.horizontal(|ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                widgets::primary_button(ui, &format!("{}  Save identity", icon::FLOPPY_DISK), ready).clicked()
            })
            .inner
        });
        if save.inner {
            let name = self.name.trim().to_string();
            let email = self.email.trim().to_string();
            self.identity_refresh_pending = true;
            cx.act("Save identity", move |client, dir| {
                client.set_identity(&name, &email, dir)?;
                Ok(Some("Saved the commit identity for this repository.".to_string()))
            });
        }
    }

    fn profiles_section(&mut self, ui: &mut Ui, cx: &mut Ctx) {
        let idle = cx.idle;
        let c = theme::of(ui);
        widgets::section(ui, "Saved profiles");
        ui.label(
            RichText::new("A profile stores a name, email, and optional signing key. Applying one sets this repository's identity. A profile with a key also turns on commit signing.")
                .color(c.muted),
        );
        ui.add_space(8.0);

        let can_save = idle && valid_identity(&self.name, &self.email);
        egui::Grid::new("identity_profile_form").num_columns(2).spacing([16.0, 10.0]).show(ui, |ui| {
            ui.label("Signing key");
            ui.add_enabled(
                idle,
                egui::TextEdit::singleline(&mut self.signing_key)
                    .hint_text("Optional: GPG key ID or SSH public key path")
                    .desired_width(320.0),
            )
            .on_hover_text("When set, applying this profile signs commits with this key.");
            ui.end_row();
        });
        ui.add_space(6.0);
        let save = ui.horizontal(|ui| {
            ui.label(
                RichText::new(format!("Saves {} from the Identity tab.", describe_identity(&self.name, &self.email)))
                    .small()
                    .color(c.muted),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                widgets::primary_button(ui, &format!("{}  Save as profile", icon::PLUS), can_save).clicked()
            })
            .inner
        });
        if save.inner {
            let key = self.signing_key.trim();
            let profile = StoredProfile {
                name: self.name.trim().to_string(),
                email: self.email.trim().to_string(),
                signing_key: (!key.is_empty()).then(|| key.to_string()),
            };
            match self.profiles.iter_mut().find(|existing| existing.matches(&profile)) {
                Some(existing) => *existing = profile,
                None => self.profiles.push(profile),
            }
            let ctx = ui.ctx().clone();
            self.save_profiles(&ctx);
            self.signing_key.clear();
        }

        ui.add_space(12.0);
        if self.profiles.is_empty() {
            widgets::empty_state(ui, icon::IDENTIFICATION_CARD, "No saved profiles yet");
            return;
        }

        let mut apply: Option<usize> = None;
        let mut delete: Option<usize> = None;
        for (index, profile) in self.profiles.iter().enumerate() {
            let core = profile.to_core();
            card(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(icon::USER_CIRCLE).color(c.muted));
                    ui.label(RichText::new(&core.label).strong());
                    if core.signing_key.is_some() {
                        widgets::pill(ui, "Signs commits", c.accent);
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button(format!("{}  Delete", icon::TRASH)).on_hover_text("Forget this profile on this computer").clicked() {
                            delete = Some(index);
                        }
                        if ui.add_enabled(idle, egui::Button::new(format!("{}  Apply", icon::CHECK))).clicked() {
                            apply = Some(index);
                        }
                    });
                });
                if let Some(key) = core.signing_key.as_deref() {
                    ui.label(RichText::new(key).monospace().small().color(c.muted));
                }
            });
            ui.add_space(6.0);
        }

        if let Some(index) = apply {
            let profile = self.profiles[index].to_core();
            self.name = profile.name.clone();
            self.email = profile.email.clone();
            self.identity_refresh_pending = true;
            self.section = Section::Identity;
            let label = profile.label.clone();
            cx.act(format!("Apply {label}"), move |client, dir| {
                client.apply_identity(&profile.name, &profile.email, profile.signing_key.as_deref(), dir)?;
                Ok(Some(format!("Applied {label} to this repository.")))
            });
        }
        if let Some(index) = delete {
            self.profiles.remove(index);
            let ctx = ui.ctx().clone();
            self.save_profiles(&ctx);
        }
    }

    fn remotes_section(&mut self, ui: &mut Ui, cx: &mut Ctx) {
        let snapshot: &Snapshot = cx.snapshot;
        let idle = cx.idle;
        widgets::section(ui, "Remotes");
        if snapshot.remotes.is_empty() {
            widgets::empty_state(ui, icon::CLOUD, "No remotes yet. Add one below.");
        }
        for remote in &snapshot.remotes {
            let fetch = snapshot.remote_fetch_addresses.get(remote).cloned().unwrap_or_default();
            let push = snapshot.remote_push_addresses.get(remote).cloned().unwrap_or_default();
            if let Some(action) = self.remote_card(ui, idle, remote, &fetch, &push) {
                self.handle_remote_action(cx, remote, fetch, action);
            }
            ui.add_space(6.0);
        }

        ui.add_space(10.0);
        widgets::section(ui, "Add a remote");
        egui::Grid::new("add_remote_form").num_columns(2).spacing([16.0, 10.0]).show(ui, |ui| {
            ui.label("Name");
            ui.add_enabled(idle, egui::TextEdit::singleline(&mut self.new_remote_name).hint_text("origin").desired_width(320.0));
            ui.end_row();
            ui.label("URL");
            ui.add_enabled(
                idle,
                egui::TextEdit::singleline(&mut self.new_remote_address)
                    .hint_text("https://example.com/repo.git or /path/to/repo.git")
                    .desired_width(320.0),
            );
            ui.end_row();
        });
        ui.add_space(8.0);
        let name = self.new_remote_name.trim().to_string();
        let address = self.new_remote_address.trim().to_string();
        let taken = snapshot.remotes.contains(&name);
        let ready = idle && !name.is_empty() && !address.is_empty() && !taken;
        let add = ui.horizontal(|ui| {
            if taken && !name.is_empty() {
                ui.label(RichText::new(format!("{}  A remote named {name} already exists.", icon::WARNING)).color(theme::of(ui).warning));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                widgets::primary_button(ui, &format!("{}  Add remote", icon::PLUS), ready).clicked()
            })
            .inner
        });
        if add.inner {
            self.pending = Some(RemotePending::Add(name.clone()));
            cx.act(format!("Add remote {name}"), move |client, dir| {
                client.add_remote(&name, &address, dir)?;
                Ok(Some(format!("Added remote {name}.")))
            });
        }
    }

    /// One remote's card: its addresses, and either its edit form or its removal confirmation.
    fn remote_card(&mut self, ui: &mut Ui, idle: bool, name: &str, fetch: &[String], push: &[String]) -> Option<RemoteAction> {
        let c = theme::of(ui);
        let displayed = fetch.first().cloned().unwrap_or_default();
        let mut action = None;
        card(ui, |ui| {
            if let Some(edit) = self.edit.as_mut().filter(|edit| edit.original == name) {
                ui.label(RichText::new(icon::PENCIL_SIMPLE).color(c.muted));
                egui::Grid::new(("edit_remote", name)).num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
                    ui.label("Name");
                    ui.add_enabled(idle, egui::TextEdit::singleline(&mut edit.name).desired_width(320.0));
                    ui.end_row();
                    ui.label("Fetch URL");
                    ui.add_enabled(idle, egui::TextEdit::singleline(&mut edit.address).desired_width(320.0));
                    ui.end_row();
                });
                let valid = !edit.name.trim().is_empty() && !edit.address.trim().is_empty();
                let changed = edit.name.trim() != name || edit.address.trim() != displayed;
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    if ui.add_enabled(idle, egui::Button::new("Cancel")).clicked() {
                        action = Some(RemoteAction::CancelEdit);
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if widgets::primary_button(ui, "Save remote", idle && valid && changed).clicked() {
                            action = Some(RemoteAction::SaveEdit);
                        }
                    });
                });
                return;
            }

            ui.horizontal(|ui| {
                ui.label(RichText::new(icon::CLOUD).color(c.muted));
                ui.label(RichText::new(name).monospace().strong());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.add_enabled(idle, egui::Button::new(format!("{}  Remove", icon::TRASH))).clicked() {
                        action = Some(RemoteAction::AskRemove);
                    }
                    if ui.add_enabled(idle, egui::Button::new(format!("{}  Edit", icon::PENCIL_SIMPLE))).clicked() {
                        action = Some(RemoteAction::StartEdit);
                    }
                });
            });
            let fetch_text = if displayed.is_empty() { "No fetch URL".to_string() } else { displayed.clone() };
            ui.add(egui::Label::new(RichText::new(fetch_text).monospace().small().color(c.muted)).wrap());
            // A remote's push URLs replace its fetch URL for pushes, so show them when they differ.
            if !push.is_empty() && push != fetch {
                for address in push {
                    ui.add(egui::Label::new(RichText::new(format!("Push: {address}")).monospace().small().color(c.muted)).wrap());
                }
            }

            if self.confirming_removal.as_deref() == Some(name) {
                ui.add_space(6.0);
                widgets::callout(
                    ui,
                    "NiceGit forgets this remote and deletes its remote-tracking branches from this repository. Local branches that track it lose their upstream. Nothing on the server changes.",
                    true,
                );
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    if widgets::danger_button(ui, &format!("{}  Remove remote {name}", icon::TRASH), idle).clicked() {
                        action = Some(RemoteAction::ConfirmRemove);
                    }
                    if ui.button("Cancel").clicked() {
                        action = Some(RemoteAction::CancelRemove);
                    }
                });
            }
        });
        action
    }

    fn handle_remote_action(&mut self, cx: &mut Ctx, name: &str, fetch: Vec<String>, action: RemoteAction) {
        match action {
            RemoteAction::StartEdit => {
                self.edit = Some(RemoteEdit {
                    original: name.to_string(),
                    name: name.to_string(),
                    address: fetch.first().cloned().unwrap_or_default(),
                });
            }
            RemoteAction::CancelEdit => self.edit = None,
            RemoteAction::SaveEdit => {
                let Some(edit) = self.edit.clone() else { return };
                let new_name = edit.name.trim().to_string();
                let new_address = edit.address.trim().to_string();
                let displayed = fetch.first().cloned().unwrap_or_default();
                let old = name.to_string();
                self.pending = Some(RemotePending::Edit { name: new_name.clone(), address: new_address.clone() });
                cx.act(format!("Save remote {name}"), move |client, dir| {
                    let mut current = old.clone();
                    if new_name != old {
                        client.rename_remote(&old, &new_name, &fetch, dir)?;
                        current = new_name.clone();
                    }
                    if new_address != displayed {
                        client.set_remote_address(&current, &new_address, &fetch, dir)?;
                    }
                    Ok(Some(format!("Saved remote {new_name}.")))
                });
            }
            RemoteAction::AskRemove => self.confirming_removal = Some(name.to_string()),
            RemoteAction::CancelRemove => self.confirming_removal = None,
            RemoteAction::ConfirmRemove => {
                self.pending = Some(RemotePending::Remove(name.to_string()));
                let name = name.to_string();
                cx.act(format!("Remove remote {name}"), move |client, dir| {
                    client.remove_remote(&name, &fetch, dir)?;
                    Ok(Some(format!("Removed remote {name}.")))
                });
            }
        }
    }
}

impl Default for RepositorySettingsWindow {
    fn default() -> Self {
        Self::new()
    }
}

impl ToolWindow for RepositorySettingsWindow {
    fn id(&self) -> String {
        "repository-settings".to_string()
    }

    fn title(&self) -> String {
        "Repository Settings".to_string()
    }

    fn default_size(&self) -> egui::Vec2 {
        egui::vec2(640.0, 580.0)
    }

    fn ui(&mut self, ui: &mut Ui, cx: &mut Ctx) {
        let ctx = ui.ctx().clone();
        let repo = cx.repo.to_path_buf();
        self.sync_identity(&ctx, &repo);
        self.load_profiles(&ctx);

        self.header(ui, cx);
        if !cx.idle {
            widgets::loading(ui, "An action is running. Controls return when it finishes.");
            ui.add_space(4.0);
        }
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.set_width(ui.available_width());
            match self.section {
                Section::Identity => self.identity_section(ui, cx),
                Section::Profiles => self.profiles_section(ui, cx),
                Section::Remotes => self.remotes_section(ui, cx),
            }
        });
    }

    fn repository_changed(&mut self, snapshot: &Snapshot) {
        if self.identity_refresh_pending {
            self.identity_refresh_pending = false;
            self.reload_identity = true;
        }
        match self.pending.take() {
            Some(RemotePending::Add(name)) if snapshot.remotes.contains(&name) => {
                self.new_remote_name.clear();
                self.new_remote_address.clear();
            }
            Some(RemotePending::Edit { name, address })
                if snapshot.remote_fetch_addresses.get(&name).and_then(|addresses| addresses.first()) == Some(&address) =>
            {
                self.edit = None;
            }
            Some(RemotePending::Remove(name)) if !snapshot.remotes.contains(&name) => {
                self.confirming_removal = None;
            }
            _ => {}
        }
    }
}

/// A card with a subtle border, grouping one remote or profile.
fn card<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    let c = theme::of(ui);
    egui::Frame::new()
        .fill(c.card_bg)
        .stroke(egui::Stroke::new(1.0, c.border))
        .corner_radius(8.0)
        .inner_margin(egui::Margin::same(12))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui)
        })
        .inner
}

fn valid_identity(name: &str, email: &str) -> bool {
    !name.trim().is_empty() && !email.trim().is_empty()
}

fn describe_identity(name: &str, email: &str) -> String {
    if valid_identity(name, email) {
        format!("{} <{}>", name.trim(), email.trim())
    } else {
        "the name and email".to_string()
    }
}
