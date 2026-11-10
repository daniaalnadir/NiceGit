//! Repository > Git settings: the repository's own Git configuration, one key at a time. Global
//! values are shown read-only for reference; NiceGit never writes global configuration.

use std::path::PathBuf;

use egui::{Margin, RichText, Ui};
use egui_phosphor::regular as icon;
use nicegit_core::git_config::{same_config_key, validate_config_key, ConfigEntry};
use nicegit_core::{GitClient, Snapshot};

use crate::theme;
use crate::tools::{query, widgets, Ctx, Task, ToolWindow};

/// The global settings shown read-only, most used first.
const GLOBAL_KEYS: [&str; 5] = ["user.name", "user.email", "pull.rebase", "init.defaultBranch", "commit.gpgsign"];

type Loaded = nicegit_core::Result<Vec<ConfigEntry>>;
type GlobalValues = Vec<(&'static str, nicegit_core::Result<Option<String>>)>;

/// A value being edited, with the values the row showed when Edit was chosen.
#[derive(Clone)]
struct Editing {
    key: String,
    expected: Vec<String>,
    value: String,
}

/// A removal waiting for its confirmation, with the values the row showed.
#[derive(Clone)]
struct Removing {
    key: String,
    expected: Vec<String>,
}

/// A write chosen this frame, started after the window is drawn.
enum Change {
    Set { key: String, expected: Vec<String>, value: String },
    Unset { key: String, expected: Vec<String> },
}

#[derive(Default)]
pub struct GitSettingsWindow {
    /// The local entries, read in the background. Read on opening, on Refresh, and after the
    /// repository changes.
    entries: Option<Task<Loaded>>,
    /// The last entries read, shown while a newer read loads.
    shown: Option<Loaded>,
    /// The global values, read once per window and again on Refresh.
    globals: Option<Task<GlobalValues>>,
    /// The last global values read, shown while a newer read loads.
    shown_globals: Option<GlobalValues>,
    filter: String,
    new_key: String,
    new_value: String,
    /// Why the add row's key was refused, shown under the row.
    add_error: Option<String>,
    editing: Option<Editing>,
    removing: Option<Removing>,
}

impl GitSettingsWindow {
    pub fn new() -> Self {
        Self::default()
    }
}

/// A value as one table line: a newline shows as `\n`, so a multi-line value keeps its row.
fn one_line(value: &str) -> String {
    value.replace('\n', "\\n")
}

impl ToolWindow for GitSettingsWindow {
    fn id(&self) -> String {
        "git_settings".to_string()
    }

    fn title(&self) -> String {
        "Git Settings".to_string()
    }

    fn default_size(&self) -> egui::Vec2 {
        egui::vec2(720.0, 580.0)
    }

    fn repository_changed(&mut self, _snapshot: &Snapshot) {
        self.entries = None;
        self.globals = None;
    }

    fn ui(&mut self, ui: &mut Ui, cx: &mut Ctx) {
        if self.entries.is_none() {
            self.entries = Some(query(ui.ctx(), cx.repo, |client, directory| client.config_entries(directory)));
        }
        if self.globals.is_none() {
            let repo: PathBuf = cx.repo.to_path_buf();
            self.globals = Some(Task::spawn(ui.ctx(), move || {
                let client = GitClient::new();
                GLOBAL_KEYS.iter().map(|&key| (key, client.global_config_value(key, &repo))).collect()
            }));
        }
        if let Some(state) = self.entries.as_mut().and_then(|task| task.get()) {
            self.shown = Some(state.clone());
        }
        if let Some(state) = self.globals.as_mut().and_then(|task| task.get()) {
            self.shown_globals = Some(state.clone());
        }
        let shown = self.shown.clone();
        let idle = cx.idle;
        let c = theme::of(ui);
        let mut reload = false;
        let mut change: Option<Change> = None;

        ui.label(RichText::new("Settings saved in this repository's own Git configuration.").small().color(c.muted));
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            let width = (ui.available_width() - 40.0).max(160.0);
            ui.add(egui::TextEdit::singleline(&mut self.filter).hint_text("Filter settings").desired_width(width))
                .widget_info(|| egui::WidgetInfo::labeled(egui::Role::TextInput, true, "Filter settings"));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if widgets::icon_button(ui, icon::ARROW_CLOCKWISE, "Read the settings again", true).clicked() {
                    reload = true;
                }
            });
        });
        ui.add_space(6.0);

        let body_height = (ui.available_height() - 150.0).max(140.0);
        egui::ScrollArea::vertical().id_salt("git_settings_body").max_height(body_height).auto_shrink([false, false]).show(ui, |ui| {
            let entries = match &shown {
                None => {
                    widgets::loading(ui, "Reading the settings…");
                    return;
                }
                Some(Err(error)) => {
                    widgets::error(ui, &error.to_string());
                    return;
                }
                Some(Ok(entries)) => entries,
            };
            if entries.is_empty() {
                ui.label(RichText::new("No settings are saved in this repository yet. Add one below.").small().color(c.muted));
            }
            let needle = self.filter.trim().to_lowercase();
            let matching: Vec<&ConfigEntry> = entries
                .iter()
                .filter(|entry| {
                    needle.is_empty()
                        || entry.key.to_lowercase().contains(&needle)
                        || entry.values.iter().any(|value| value.to_lowercase().contains(&needle))
                })
                .collect();
            if !entries.is_empty() && matching.is_empty() {
                ui.label(RichText::new("No setting matches the filter.").small().color(c.muted));
            }

            for entry in matching {
                let editing_this = self.editing.as_ref().is_some_and(|editing| editing.key == entry.key);
                let removing_this = self.removing.as_ref().is_some_and(|removing| removing.key == entry.key);
                egui::Frame::new().fill(c.subtle_bg).corner_radius(6.0).inner_margin(Margin::symmetric(10, 6)).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.add(egui::Label::new(RichText::new(&entry.key).monospace()).truncate());
                        if !entry.is_multi_valued() && !editing_this && !removing_this {
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                let remove = ui.add_enabled(idle, egui::Button::new("Remove"));
                                remove.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, format!("Remove {}", entry.key)));
                                if remove.clicked() {
                                    self.editing = None;
                                    self.removing = Some(Removing { key: entry.key.clone(), expected: entry.values.clone() });
                                }
                                let edit = ui.add_enabled(idle, egui::Button::new("Edit"));
                                edit.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, format!("Edit {}", entry.key)));
                                if edit.clicked() {
                                    self.removing = None;
                                    self.editing = Some(Editing {
                                        key: entry.key.clone(),
                                        expected: entry.values.clone(),
                                        value: entry.values.first().cloned().unwrap_or_default(),
                                    });
                                }
                            });
                        }
                    });

                    if editing_this {
                        let mut cancel = false;
                        if let Some(editing) = self.editing.as_mut() {
                            ui.add(egui::TextEdit::multiline(&mut editing.value).desired_rows(2).desired_width(f32::INFINITY))
                                .widget_info(|| egui::WidgetInfo::labeled(egui::Role::TextInput, true, "Setting value"));
                            ui.horizontal(|ui| {
                                if widgets::primary_button(ui, "Save", idle).clicked() {
                                    change = Some(Change::Set {
                                        key: editing.key.clone(),
                                        expected: editing.expected.clone(),
                                        value: editing.value.clone(),
                                    });
                                }
                                cancel = ui.button("Cancel").clicked();
                            });
                        }
                        if cancel {
                            self.editing = None;
                        }
                    } else if removing_this {
                        ui.label(format!(
                            "Remove {}? Git deletes this setting from this repository. Other settings stay as they are.",
                            entry.key
                        ));
                        ui.horizontal(|ui| {
                            if widgets::danger_button(ui, "Remove", idle).clicked() {
                                // The values from when Remove was chosen, not the row's current ones.
                                let expected = self.removing.as_ref().map(|removing| removing.expected.clone()).unwrap_or_default();
                                change = Some(Change::Unset { key: entry.key.clone(), expected });
                            }
                            if ui.button("Cancel").clicked() {
                                self.removing = None;
                            }
                        });
                    } else if entry.is_multi_valued() {
                        ui.label(RichText::new("Has several values; edit in a terminal").small().color(c.muted));
                        for value in &entry.values {
                            ui.add(egui::Label::new(RichText::new(one_line(value)).monospace()).truncate());
                        }
                    } else {
                        let value = entry.values.first().map(|value| one_line(value)).unwrap_or_default();
                        ui.add(egui::Label::new(RichText::new(value).monospace()).truncate());
                    }
                });
                ui.add_space(3.0);
            }

            ui.add_space(10.0);
            egui::CollapsingHeader::new("Global settings, read only").id_salt("git_settings_globals").default_open(false).show(ui, |ui| {
                ui.label(
                    RichText::new("From your global Git configuration, which applies to every repository. Change these in a terminal with git config --global.")
                        .small()
                        .color(c.muted),
                );
                ui.add_space(4.0);
                match &self.shown_globals {
                    None => widgets::loading(ui, "Reading global settings…"),
                    Some(values) => {
                        for (key, result) in values {
                            ui.horizontal(|ui| {
                                ui.add(egui::Label::new(RichText::new(*key).monospace()).truncate());
                                match result {
                                    Ok(Some(value)) => {
                                        ui.add(egui::Label::new(RichText::new(one_line(value)).monospace()).truncate());
                                    }
                                    Ok(None) => {
                                        ui.label(RichText::new("Not set").small().color(c.muted));
                                    }
                                    Err(error) => {
                                        ui.label(RichText::new(error.to_string()).small().color(c.conflict));
                                    }
                                }
                            });
                        }
                    }
                }
            });
        });

        ui.separator();
        widgets::section(ui, "Add a setting");
        ui.add_space(4.0);
        let loaded = matches!(shown, Some(Ok(_)));
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut self.new_key).hint_text("section.name").desired_width(200.0))
                .widget_info(|| egui::WidgetInfo::labeled(egui::Role::TextInput, true, "Setting name"));
            let width = (ui.available_width() - 90.0).max(140.0);
            ui.add(egui::TextEdit::singleline(&mut self.new_value).hint_text("Value").desired_width(width))
                .widget_info(|| egui::WidgetInfo::labeled(egui::Role::TextInput, true, "Setting value"));
            let can_add = idle && loaded && !self.new_key.trim().is_empty();
            if widgets::primary_button(ui, "Add", can_add).clicked() {
                let key = self.new_key.trim().to_string();
                self.add_error = match validate_config_key(&key) {
                    Err(reason) => Some(reason),
                    Ok(()) => match &shown {
                        Some(Ok(entries)) => entries
                            .iter()
                            .find(|entry| same_config_key(&entry.key, &key))
                            .map(|existing| format!("{} is already set. Edit its row instead.", existing.key)),
                        _ => None,
                    },
                };
                if self.add_error.is_none() {
                    change = Some(Change::Set { key, expected: Vec::new(), value: self.new_value.clone() });
                    self.new_key.clear();
                    self.new_value.clear();
                }
            }
        });
        if let Some(error) = &self.add_error {
            ui.label(RichText::new(error).small().color(c.conflict));
        }

        if reload {
            self.entries = None;
            self.globals = None;
        }
        match change {
            Some(Change::Set { key, expected, value }) => {
                self.editing = None;
                cx.act(format!("Set {key}"), move |client, directory| {
                    client.set_config_value(directory, &key, &expected, &value).map(|()| Some(format!("Set {key}.")))
                });
            }
            Some(Change::Unset { key, expected }) => {
                self.removing = None;
                cx.act(format!("Remove {key}"), move |client, directory| {
                    client.unset_config_value(directory, &key, &expected).map(|()| Some(format!("Removed {key}.")))
                });
            }
            None => {}
        }
    }
}
