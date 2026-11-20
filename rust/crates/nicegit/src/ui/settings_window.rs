use egui::RichText;

use crate::app::NiceGitApp;
use crate::issue_links::{check_rule, IssueLinkRule};
use crate::theme::{self, Appearance, GraphPalette};
use crate::tools::widgets;

/// A custom rule being added or edited in the Issue links section.
#[derive(Clone, Debug, Default)]
struct IssueLinkDraft {
    /// The index of the rule being edited, or None for a new rule.
    editing: Option<usize>,
    pattern: String,
    url: String,
    remote_contains: String,
    /// Why the last save was refused.
    problem: Option<String>,
}

/// Where the draft rule is kept while the Settings window is open.
fn issue_link_draft_id() -> egui::Id {
    egui::Id::unique("issue_link_draft")
}

impl NiceGitApp {
    pub fn settings_window(&mut self, ctx: &egui::Context) {
        // The GitHub Enterprise text is a draft: it is kept while the window is open, and read
        // again from the saved hosts the next time the window opens.
        let draft_id = egui::Id::unique("github_enterprise_draft");
        let gitlab_draft_id = egui::Id::unique("gitlab_hosts_draft");
        if !self.show_settings {
            ctx.data_mut(|data| {
                data.remove::<String>(draft_id);
                data.remove::<String>(gitlab_draft_id);
                data.remove::<IssueLinkDraft>(issue_link_draft_id());
                data.remove::<crate::custom_actions::ActionDraft>(egui::Id::unique("custom_action_draft"));
            });
            crate::ui::ai_draft::forget_key_draft(ctx);
            return;
        }
        let mut open = true;
        // Escape closes Settings, unless it is closing one of the window's own menus.
        if !egui::Popup::is_any_open(ctx) && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
            open = false;
        }
        let mut reload_diff = false;
        let mut show_shortcuts = false;
        // The window never grows past the app's window: its title bar, with the close button, stays
        // on screen and the settings scroll beneath it.
        let max_height = (ctx.content_rect().height() - 190.0).max(200.0);
        egui::Window::new("Settings")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(420.0)
            .default_height(max_height)
            .pivot(egui::Align2::CENTER_CENTER)
            .default_pos(ctx.content_rect().center())
            .show(ctx, |ui| {
              egui::ScrollArea::vertical().id_salt("settings-scroll").max_height(max_height).auto_shrink([false, true]).show(ui, |ui| {
                let c = theme::of(ui);
                widgets::section(ui, "General");
                crate::tools::widgets::checkbox(ui, true, &mut self.settings.check_for_updates, "Check for new versions");
                ui.label(
                    RichText::new("Asks GitHub once a day for the latest version number. Nothing else is sent.").small().color(c.muted),
                );
                ui.add_space(4.0);
                self.update_check_controls(ui);
                ui.add_space(10.0);
                crate::tools::widgets::checkbox(ui, true, &mut self.settings.notify_when_finished, "Notify me when work finishes in the background");
                ui.label(
                    RichText::new("Shows a desktop notification when an action you started finishes while NiceGit is not in focus.").small().color(c.muted),
                );
                ui.add_space(10.0);
                widgets::section(ui, "Appearance");
                ui.horizontal(|ui| {
                    for (value, label) in [(Appearance::System, "System"), (Appearance::Light, "Light"), (Appearance::Dark, "Dark")] {
                        ui.selectable_value(&mut self.settings.appearance, value, label);
                    }
                });
                ui.add_space(10.0);
                widgets::section(ui, "Graph colours");
                for palette in [GraphPalette::Standard, GraphPalette::ColorBlindSafe, GraphPalette::Muted] {
                    ui.horizontal(|ui| {
                        // A fixed-width label column keeps every row's swatches at the same x.
                        let label_size = egui::vec2(150.0, ui.spacing().interact_size.y);
                        ui.allocate_ui_with_layout(label_size, egui::Layout::left_to_right(egui::Align::Center), |ui| {
                            // The space is otherwise shrunk to the label's own width.
                            ui.set_min_width(label_size.x);
                            if crate::tools::widgets::radio(ui, self.settings.graph_palette == palette, palette.title()).clicked() {
                                self.settings.graph_palette = palette;
                            }
                        });
                        let previous = theme::graph_palette();
                        theme::set_graph_palette(palette);
                        for index in 0..theme::GRAPH_COLOR_COUNT {
                            let (rect, _) = ui.allocate_exact_size(egui::vec2(14.0, 14.0), egui::Sense::hover());
                            ui.painter().circle_filled(rect.center(), 6.5, theme::graph_color(index));
                        }
                        theme::set_graph_palette(previous);
                    });
                }
                ui.add_space(10.0);
                widgets::section(ui, "Diffs");
                reload_diff |=
                    crate::tools::widgets::checkbox(ui, true, &mut self.settings.ignore_whitespace, "Hide whitespace-only changes")
                        .changed();
                crate::tools::widgets::checkbox(ui, true, &mut self.settings.split_diff, "Show diffs side by side");
                // Said plainly when the choice is not in effect at the panel's current width.
                let minimum = crate::diff_view::MIN_SPLIT_WIDTH;
                let too_narrow = self.diff_panel_width.is_some_and(|width| !crate::diff_view::split_fits(width));
                let (note, color) = if self.settings.split_diff && too_narrow {
                    (
                        format!("Not in effect now: the diff panel is narrower than {minimum} points. Widen it to see both sides."),
                        theme::of(ui).warning,
                    )
                } else {
                    (format!("Needs a diff panel at least {minimum} points wide."), theme::of(ui).muted)
                };
                // A warning is set at body size so it reads as clearly as the setting it explains.
                let note = RichText::new(note).color(color);
                ui.label(if too_narrow && self.settings.split_diff { note } else { note.small() });
                // Words takes the place of side by side while it is on.
                crate::tools::widgets::checkbox(ui, true, &mut self.settings.word_diff, "Show changed words inline");
                crate::tools::widgets::checkbox(ui, true, &mut self.settings.syntax_highlighting, "Syntax highlighting");
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.label("External editor");
                    let field = ui.add(
                        egui::TextEdit::singleline(&mut self.settings.external_editor).hint_text("code, zed, subl…").desired_width(180.0),
                    );
                    field.widget_info(|| egui::WidgetInfo::labeled(egui::Role::TextInput, true, "External editor"));
                })
                .response
                .on_hover_text(
                    "The command that opens a file in your editor; the file's path is added after it. Changed files then offer Open in it.",
                );
                ui.add_space(10.0);
                widgets::section(ui, "Committing");
                ui.horizontal(|ui| {
                    ui.label("Conventional Commits");
                    for choice in crate::conventional::ConventionalCommits::ALL {
                        ui.selectable_value(&mut self.settings.conventional_commits, choice, choice.title());
                    }
                })
                .response
                .on_hover_text(
                    "Offers a type menu, scope, and Breaking above the summary, which write the type into the summary. \
                     Automatic turns this on when at least five of the last twenty commits use the convention.",
                );
                ui.add_space(10.0);
                self.claude_section(ui);
                ui.add_space(10.0);
                widgets::section(ui, "Repository");
                crate::tools::widgets::checkbox(
                    ui,
                    true,
                    &mut self.settings.auto_refresh,
                    "Refresh automatically when files change outside NiceGit",
                );
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.label("Fetch from remotes");
                    for (minutes, label) in [(0, "Off"), (5, "5 min"), (15, "15 min"), (30, "30 min"), (60, "1 hour")] {
                        ui.selectable_value(&mut self.settings.auto_fetch_minutes, minutes, label);
                    }
                })
                .response
                .on_hover_text("How often the open repository is fetched in the background, so ahead and behind counts stay current");
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.label("Pull with");
                    egui::ComboBox::from_id_salt("pull_with").selected_text(self.settings.pull_setting.title()).show_ui(ui, |ui| {
                        for choice in crate::settings::PullSetting::ALL {
                            ui.selectable_value(&mut self.settings.pull_setting, choice, choice.title());
                        }
                    });
                })
                .response
                .on_hover_text(
                    "How Pull brings in the upstream's commits. Follow Git's settings reads pull.rebase, branch.<name>.rebase and pull.ff; \
                     with none of them set, Pull only fast-forwards. The Fetch and Pull menu also runs a merge or rebase pull once.",
                );
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.label("Clone into");
                    match self.settings.clone_folder.as_ref() {
                        Some(folder) => {
                            ui.add(egui::Label::new(RichText::new(folder.display().to_string()).monospace()).truncate())
                                .on_hover_text(folder.display().to_string());
                        }
                        None => {
                            ui.label(RichText::new("Not set").color(c.muted));
                        }
                    }
                    if ui.button("Choose…").on_hover_text("Clones go into this folder by default").clicked() {
                        if let Some(folder) = crate::file_dialog::pick_folder("Default folder for clones") {
                            self.settings.clone_folder = Some(folder);
                        }
                    }
                    if ui.add_enabled(self.settings.clone_folder.is_some(), egui::Button::new("Clear")).clicked() {
                        self.settings.clone_folder = None;
                    }
                })
                .response
                .on_hover_text("The Clone dialog fills in its destination from this folder. The first clone's parent folder is remembered when none is set.");
                ui.add_space(10.0);
                widgets::section(ui, "Custom actions");
                self.custom_actions_section(ui);
                ui.add_space(10.0);
                widgets::section(ui, "GitHub");
                ui.checkbox(&mut self.settings.show_branch_checks, "Show pull request checks on branches")
                    .on_hover_text("A dot on a branch's label and in the sidebar shows whether its open pull or merge request's checks passed, failed, or are pending. Turning this off also stops reading them.");
                ui.add_space(10.0);
                widgets::section(ui, "GitHub Enterprise");
                let mut draft = ui
                    .ctx()
                    .data(|data| data.get_temp::<String>(draft_id))
                    .unwrap_or_else(|| self.settings.github_enterprise_hosts.join("\n"));
                ui.label(
                    RichText::new("Remotes on these hosts get pull requests, issues, and Open on GitHub, as github.com remotes do.")
                        .small()
                        .color(c.muted),
                );
                ui.add_space(4.0);
                let field = ui.add(
                    egui::TextEdit::multiline(&mut draft)
                        .hint_text("One hostname per line, such as github.example.com")
                        .desired_rows(3)
                        .desired_width(f32::INFINITY),
                );
                field.widget_info(|| egui::WidgetInfo::labeled(egui::Role::TextInput, true, "GitHub Enterprise hosts"));
                let (valid, invalid) = crate::settings::parse_enterprise_hosts(&draft);
                // Only valid hostnames are saved; the rest stay in the box, named below.
                if valid != self.settings.github_enterprise_hosts {
                    self.settings.github_enterprise_hosts = valid;
                }
                if !invalid.is_empty() {
                    let text = format!(
                        "Not saved, as these are not hostnames: {}. Use a name such as github.example.com, without https:// or a port.",
                        invalid.join(", ")
                    );
                    ui.label(RichText::new(text).small().color(c.warning));
                }
                ui.ctx().data_mut(|data| data.insert_temp(draft_id, draft));
                ui.add_space(10.0);
                widgets::section(ui, "GitLab");
                // As the GitHub Enterprise text, a draft kept while the window is open.
                let gitlab_draft_id = egui::Id::unique("gitlab_hosts_draft");
                let mut gitlab_draft = ui
                    .ctx()
                    .data(|data| data.get_temp::<String>(gitlab_draft_id))
                    .unwrap_or_else(|| self.settings.gitlab_hosts.join("\n"));
                ui.label(RichText::new("Remotes on these hosts get Open on GitLab links, as gitlab.com remotes do.").small().color(c.muted));
                ui.add_space(4.0);
                let field = ui.add(
                    egui::TextEdit::multiline(&mut gitlab_draft)
                        .hint_text("One hostname per line, such as gitlab.example.com")
                        .desired_rows(3)
                        .desired_width(f32::INFINITY),
                );
                field.widget_info(|| egui::WidgetInfo::labeled(egui::Role::TextInput, true, "GitLab hosts"));
                let (valid, invalid) = crate::settings::parse_gitlab_hosts(&gitlab_draft);
                if valid != self.settings.gitlab_hosts {
                    self.settings.gitlab_hosts = valid;
                }
                if !invalid.is_empty() {
                    let text = format!(
                        "Not saved, as these are not hostnames: {}. Use a name such as gitlab.example.com, without https:// or a port.",
                        invalid.join(", ")
                    );
                    ui.label(RichText::new(text).small().color(c.warning));
                }
                ui.ctx().data_mut(|data| data.insert_temp(gitlab_draft_id, gitlab_draft));
                ui.add_space(10.0);
                self.issue_links_section(ui);
                ui.add_space(10.0);
                widgets::section(ui, "Layout");
                ui.horizontal(|ui| {
                    ui.label("Interface size");
                    let zoom = ctx.zoom_factor();
                    let step = |zoom: f32, by: f32| ((zoom + by) * 10.0).round() / 10.0;
                    if ui
                        .add_enabled(zoom > crate::app::MIN_ZOOM + 0.01, egui::Button::new("−"))
                        .on_hover_text("Smaller (Ctrl/Cmd-minus)")
                        .clicked()
                    {
                        ctx.set_zoom_factor(step(zoom, -0.1).max(crate::app::MIN_ZOOM));
                    }
                    ui.label(RichText::new(format!("{:.0}%", zoom * 100.0)).monospace());
                    if ui
                        .add_enabled(zoom < crate::app::MAX_ZOOM - 0.01, egui::Button::new("+"))
                        .on_hover_text("Larger (Ctrl/Cmd-plus)")
                        .clicked()
                    {
                        ctx.set_zoom_factor(step(zoom, 0.1).min(crate::app::MAX_ZOOM));
                    }
                    if ui.add_enabled((zoom - 1.0).abs() > 0.01, egui::Button::new("Normal size")).on_hover_text("Ctrl/Cmd-0").clicked() {
                        ctx.set_zoom_factor(1.0);
                    }
                });
                ui.add_space(4.0);
                if ui.button("Restore default panel widths").on_hover_text("Columns and panels return to their original sizes").clicked() {
                    crate::app::restore_panel_widths(ctx);
                    self.settings.history_columns.widths.clear();
                }
              });
                // The version and shortcuts stay below the scrolling settings, always in view.
                ui.separator();
                let c = theme::of(ui);
                ui.horizontal(|ui| {
                    ui.label(RichText::new(format!("NiceGit {}", env!("CARGO_PKG_VERSION"))).small().color(c.muted));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        // Laid out from the right, so the links read Report a problem, Request a feature,
                        // Keyboard shortcuts.
                        if ui.link("Keyboard shortcuts").clicked() {
                            show_shortcuts = true;
                        }
                        if ui.link("Request a feature").clicked() {
                            crate::ui::changes::open_url(nicegit_core::feedback::FEATURE_REQUEST_URL);
                        }
                        if ui.link("Report a problem").clicked() {
                            crate::ui::changes::open_url(&crate::updates::bug_report_url());
                        }
                    });
                });
            });
        if reload_diff {
            self.reload_diff();
        }
        self.show_settings = open;
        self.show_shortcuts |= show_shortcuts;
    }

    /// The Issue links section: whether `#123` links to the host, and the custom rules with their
    /// inline form. A rule is saved only when it checks out; the problem is shown otherwise.
    fn issue_links_section(&mut self, ui: &mut egui::Ui) {
        let c = theme::of(ui);
        widgets::section(ui, "Issue links");
        crate::tools::widgets::checkbox(ui, true, &mut self.settings.link_host_issues, "Link #123 to the repository's host");
        ui.label(
            RichText::new(
                "For the origin remote on GitHub, GitLab, or Bitbucket, #123 links to the issue (on GitLab, !123 to the merge request). \
                 On Azure DevOps, AB#123 links to the work item.",
            )
            .small()
            .color(c.muted),
        );
        ui.add_space(6.0);

        let mut draft: Option<IssueLinkDraft> = ui.ctx().data(|data| data.get_temp(issue_link_draft_id()));
        let mut remove = None;
        let mut edit = None;
        if self.settings.issue_links.is_empty() {
            ui.label(RichText::new("No custom rules. A rule can link text such as JIRA-7 to your tracker.").small().color(c.muted));
        }
        for (index, rule) in self.settings.issue_links.iter().enumerate() {
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.label(RichText::new(&rule.pattern).monospace());
                    ui.label(RichText::new(&rule.url).small().color(c.muted));
                    if !rule.remote_contains.trim().is_empty() {
                        let text = format!("Only for repositories whose remote contains \"{}\"", rule.remote_contains.trim());
                        ui.label(RichText::new(text).small().color(c.muted));
                    }
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Remove").on_hover_text("Remove this rule").clicked() {
                        remove = Some(index);
                    }
                    if ui.button("Edit").on_hover_text("Change this rule").clicked() {
                        edit = Some(index);
                    }
                });
            });
        }
        if remove.is_some() {
            draft = None;
        }
        if let Some(index) = remove {
            self.settings.issue_links.remove(index);
        }
        if let Some(index) = edit {
            let rule = &self.settings.issue_links[index];
            draft = Some(IssueLinkDraft {
                editing: Some(index),
                pattern: rule.pattern.clone(),
                url: rule.url.clone(),
                remote_contains: rule.remote_contains.clone(),
                problem: None,
            });
        }
        if draft.is_none() && ui.button("Add rule").on_hover_text("Add a rule that links text in messages").clicked() {
            draft = Some(IssueLinkDraft::default());
        }

        // Set when the form closes: after a save that checks out, or Cancel.
        let mut close = false;
        if let Some(form) = draft.as_mut() {
            ui.add_space(6.0);
            ui.label("Pattern (a regular expression)");
            let field = ui.add(egui::TextEdit::singleline(&mut form.pattern).hint_text(r"\b[A-Z]+-\d+\b").desired_width(f32::INFINITY));
            field.widget_info(|| egui::WidgetInfo::labeled(egui::Role::TextInput, true, "Rule pattern"));
            ui.label("Address to open ($0 is the match, $1 to $9 its groups)");
            let field = ui.add(
                egui::TextEdit::singleline(&mut form.url).hint_text("https://example.atlassian.net/browse/$1").desired_width(f32::INFINITY),
            );
            field.widget_info(|| egui::WidgetInfo::labeled(egui::Role::TextInput, true, "Rule address"));
            ui.label("Only for repositories whose remote contains (empty for all)");
            let field =
                ui.add(egui::TextEdit::singleline(&mut form.remote_contains).hint_text("github.com/acme").desired_width(f32::INFINITY));
            field.widget_info(|| egui::WidgetInfo::labeled(egui::Role::TextInput, true, "Rule remote filter"));
            if let Some(problem) = &form.problem {
                ui.label(RichText::new(problem).color(c.warning));
            }
            ui.horizontal(|ui| {
                if ui.button("Save rule").clicked() {
                    let rule = IssueLinkRule {
                        pattern: form.pattern.clone(),
                        url: form.url.trim().to_string(),
                        remote_contains: form.remote_contains.trim().to_string(),
                    };
                    match check_rule(&rule) {
                        Ok(()) => {
                            match form.editing {
                                Some(index) if index < self.settings.issue_links.len() => self.settings.issue_links[index] = rule,
                                _ => self.settings.issue_links.push(rule),
                            }
                            close = true;
                        }
                        Err(problem) => form.problem = Some(problem),
                    }
                }
                if ui.button("Cancel").clicked() {
                    close = true;
                }
            });
        }
        if close {
            draft = None;
        }
        ui.ctx().data_mut(|data| {
            if let Some(form) = draft {
                data.insert_temp(issue_link_draft_id(), form);
            } else {
                data.remove::<IssueLinkDraft>(issue_link_draft_id());
            }
        });
    }
}
