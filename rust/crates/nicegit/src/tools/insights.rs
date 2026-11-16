//! Insights: how much the history changed and when, who changed it, and which files and folders
//! changed most. Everything is read from the history in the background, and nothing in the
//! repository changes.

use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use egui::{Color32, Margin, Pos2, Rect, RichText, Sense, Stroke, Ui, Vec2};
use egui_phosphor::regular as icon;
use nicegit_core::insights::{civil_date, AuthorStat, Bucket, FileStat, FolderStat, Granularity, Insights, InsightsPeriod, InsightsRange};
use nicegit_core::{GitClient, GitError, Snapshot};

use crate::theme;
use crate::tools::file_history::FileHistoryWindow;
use crate::tools::{widgets, Ctx, Task, ToolWindow};

/// Authors listed before "and N more".
const AUTHOR_LIMIT: usize = 20;
/// Folders listed before "and N more".
const FOLDER_LIMIT: usize = 15;
const CHART_HEIGHT: f32 = 120.0;
const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

/// A read of the history in progress: its result, and the flags that stop it early.
struct Reading {
    task: Task<nicegit_core::Result<Insights>>,
    cancel: Arc<AtomicBool>,
    progress: Arc<AtomicUsize>,
}

pub struct InsightsWindow {
    range: InsightsRange,
    reading: Option<Reading>,
    /// The last finished read, or its error. `None` while reading and after a cancel.
    result: Option<Result<Insights, String>>,
    /// Set when the range or the repository changed, so the history is read again on the next frame.
    needs_load: bool,
    cancelled: bool,
}

impl Default for InsightsWindow {
    fn default() -> Self {
        Self::new()
    }
}

impl InsightsWindow {
    pub fn new() -> Self {
        let range = InsightsRange { period: InsightsPeriod::Year, all_branches: false };
        Self { range, reading: None, result: None, needs_load: true, cancelled: false }
    }

    /// Starts a read of the history for the current range, stopping any read still running.
    fn start(&mut self, ctx: &egui::Context, repo: &Path) {
        if let Some(previous) = self.reading.take() {
            previous.cancel.store(true, Ordering::Relaxed);
        }
        let cancel = Arc::new(AtomicBool::new(false));
        let progress = Arc::new(AtomicUsize::new(0));
        let (range, repo) = (self.range, repo.to_path_buf());
        let (read_cancel, read_progress) = (cancel.clone(), progress.clone());
        let task = Task::spawn(ctx, move || GitClient::new().insights_with_progress(&repo, range, &read_cancel, &read_progress));
        self.reading = Some(Reading { task, cancel, progress });
        self.result = None;
        self.cancelled = false;
        self.needs_load = false;
    }

    fn controls(&mut self, ui: &mut Ui) {
        let c = theme::of(ui);
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new("Period").small().color(c.muted));
            let periods = [
                (InsightsPeriod::Days30, "30 days"),
                (InsightsPeriod::Days90, "90 days"),
                (InsightsPeriod::Year, "1 year"),
                (InsightsPeriod::AllTime, "All time"),
            ];
            for (period, text) in periods {
                if ui.selectable_label(self.range.period == period, text).clicked() && self.range.period != period {
                    self.range.period = period;
                    self.needs_load = true;
                }
            }
            ui.add_space(12.0);
            for (all_branches, text) in [(false, "This branch"), (true, "All branches")] {
                if ui.selectable_label(self.range.all_branches == all_branches, text).clicked() && self.range.all_branches != all_branches {
                    self.range.all_branches = all_branches;
                    self.needs_load = true;
                }
            }
        });
    }

    /// The progress line while a read runs, with Cancel. Returns true when Cancel was clicked.
    fn reading_line(ui: &mut Ui, reading: &Reading) -> bool {
        let c = theme::of(ui);
        ui.ctx().request_repaint_after(Duration::from_millis(100));
        let count = reading.progress.load(Ordering::Relaxed);
        let text = if count == 0 { "Reading history…".to_string() } else { format!("Reading history… {count} commits so far") };
        let mut cancel = false;
        ui.horizontal(|ui| {
            ui.spinner();
            ui.label(RichText::new(text).color(c.muted));
            if ui.button("Cancel").clicked() {
                cancel = true;
            }
        });
        cancel
    }
}

impl Drop for InsightsWindow {
    /// Closing the window stops a read still running, so it does not finish for nobody.
    fn drop(&mut self) {
        if let Some(reading) = &self.reading {
            reading.cancel.store(true, Ordering::Relaxed);
        }
    }
}

impl ToolWindow for InsightsWindow {
    fn id(&self) -> String {
        "insights".to_string()
    }

    fn title(&self) -> String {
        "Insights".to_string()
    }

    fn default_size(&self) -> egui::Vec2 {
        egui::vec2(780.0, 640.0)
    }

    fn repository_changed(&mut self, _snapshot: &Snapshot) {
        self.needs_load = true;
    }

    fn ui(&mut self, ui: &mut Ui, cx: &mut Ctx) {
        if self.needs_load {
            self.start(ui.ctx(), cx.repo);
        }
        if let Some(result) = self.reading.as_mut().and_then(|reading| reading.task.get().cloned()) {
            self.reading = None;
            self.result = Some(result.map_err(|error| message(&error)));
        }

        self.controls(ui);
        ui.add_space(6.0);
        if let Some(reading) = &self.reading {
            if Self::reading_line(ui, reading) {
                if let Some(reading) = self.reading.take() {
                    reading.cancel.store(true, Ordering::Relaxed);
                }
                self.cancelled = true;
            }
            return;
        }
        if self.cancelled {
            widgets::empty_state(ui, icon::CHART_BAR, "Reading the history was cancelled.");
            ui.vertical_centered(|ui| {
                if ui.button("Read again").clicked() {
                    self.needs_load = true;
                }
            });
            return;
        }
        match &self.result {
            None => {}
            Some(Err(text)) => widgets::error(ui, text),
            Some(Ok(insights)) => {
                if insights.commits == 0 {
                    widgets::empty_state(ui, icon::CHART_BAR, "No commits in this period.");
                    return;
                }
                egui::ScrollArea::vertical().id_salt("insights").auto_shrink([false, false]).show(ui, |ui| {
                    show(ui, cx, insights);
                });
            }
        }
    }
}

/// Everything the window shows for a finished read.
fn show(ui: &mut Ui, cx: &mut Ctx, insights: &Insights) {
    if insights.truncated {
        widgets::callout(ui, "Showing the newest 20,000 commits.", false);
        ui.add_space(6.0);
    }
    summary(ui, insights);
    ui.add_space(14.0);
    chart(ui, insights);
    ui.add_space(14.0);
    authors(ui, &insights.authors);
    ui.add_space(14.0);
    files(ui, cx, &insights.files);
    ui.add_space(14.0);
    folders(ui, &insights.folders);
}

fn summary(ui: &mut Ui, insights: &Insights) {
    let span = match (insights.first_commit, insights.last_commit) {
        (Some(first), Some(last)) => format!("{} – {}", date_text(first), date_text(last)),
        _ => "None".to_string(),
    };
    ui.horizontal_wrapped(|ui| {
        stat(ui, "Commits", insights.commits.to_string());
        stat(ui, "Authors", insights.authors.len().to_string());
        stat(ui, "Active days", insights.active_days.to_string());
        stat(ui, "First to last commit", span);
    });
}

/// A small card with a caption above its value.
fn stat(ui: &mut Ui, caption: &str, value: String) {
    let c = theme::of(ui);
    egui::Frame::new().fill(c.card_bg).stroke(Stroke::new(1.0, c.border)).corner_radius(6.0).inner_margin(Margin::symmetric(10, 6)).show(
        ui,
        |ui| {
            ui.set_min_width(110.0);
            ui.label(RichText::new(caption).small().color(c.muted));
            ui.label(RichText::new(value).font(theme::strong(15.0)));
        },
    );
}

/// Commits over time as bars, one per bucket, which shrink to fit the width.
fn chart(ui: &mut Ui, insights: &Insights) {
    let c = theme::of(ui);
    widgets::section(ui, "Commits over time");
    ui.add_space(4.0);
    let width = ui.available_width().max(120.0);
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, CHART_HEIGHT + 22.0), Sense::hover());
    let plot = Rect::from_min_size(rect.min, Vec2::new(width, CHART_HEIGHT));
    let painter = ui.painter_at(rect);
    let series = &insights.series;
    let count = series.len().max(1);
    let slot = width / count as f32;
    let max = series.iter().map(|bucket| bucket.commits).max().unwrap_or(0).max(1);

    painter.line_segment([plot.left_bottom(), plot.right_bottom()], Stroke::new(1.0, c.border));
    for (index, bucket) in series.iter().enumerate() {
        if bucket.commits == 0 {
            continue;
        }
        let x = plot.left() + slot * (index as f32 + 0.5);
        let height = bucket.commits as f32 / max as f32 * (CHART_HEIGHT - 2.0);
        let half = (slot * 0.36).max(0.5);
        let bar = Rect::from_min_max(Pos2::new(x - half, plot.bottom() - height), Pos2::new(x + half, plot.bottom()));
        painter.rect_filled(bar, 1.0, c.accent);
    }
    let font = egui::TextStyle::Small.resolve(ui.style());
    painter.text(plot.left_top() + Vec2::new(2.0, 0.0), egui::Align2::LEFT_TOP, max.to_string(), font.clone(), c.muted);

    // Dates along the bottom, spread out and kept inside the chart's width.
    let labels = ((width / 110.0) as usize).clamp(2, 6).min(count);
    for label in 0..labels {
        let index = if labels == 1 { 0 } else { label * (count - 1) / (labels - 1) };
        let Some(bucket) = series.get(index) else { break };
        let galley = painter.layout_no_wrap(date_text(bucket.start), font.clone(), c.muted);
        let x = plot.left() + slot * (index as f32 + 0.5);
        let left = (x - galley.size().x / 2.0).clamp(rect.left(), (rect.right() - galley.size().x).max(rect.left()));
        painter.galley(Pos2::new(left, plot.bottom() + 4.0), galley, c.muted);
    }

    if let Some(pointer) = response.hover_pos() {
        let index = (((pointer.x - plot.left()) / slot).floor().max(0.0) as usize).min(count - 1);
        if let Some(bucket) = series.get(index) {
            response.on_hover_text(bucket_text(bucket, insights.granularity));
        }
    }
}

/// The tooltip for one bar, such as "Week of 3 Nov 2026: 14 commits".
fn bucket_text(bucket: &Bucket, granularity: Granularity) -> String {
    let commits = if bucket.commits == 1 { "1 commit".to_string() } else { format!("{} commits", bucket.commits) };
    match granularity {
        Granularity::Week => format!("Week of {}: {commits}", date_text(bucket.start)),
        Granularity::Day => format!("{}: {commits}", date_text(bucket.start)),
    }
}

fn authors(ui: &mut Ui, authors: &[AuthorStat]) {
    let c = theme::of(ui);
    widgets::section(ui, "Authors");
    ui.add_space(4.0);
    let max = authors.iter().map(|author| author.commits).max().unwrap_or(0).max(1);
    let available = ui.available_width();
    // Name takes what the other columns leave: the commit bar and count, lines, and last active date.
    let name_width = (available - (150.0 + 130.0 + 96.0 + 24.0)).max(90.0);
    ui.horizontal(|ui| {
        header(ui, name_width, "Author");
        header(ui, 150.0, "Commits");
        header(ui, 130.0, "Lines");
        header(ui, 96.0, "Last active");
    });
    for author in authors.iter().take(AUTHOR_LIMIT) {
        ui.horizontal(|ui| {
            left_cell(ui, [name_width, 20.0], egui::Label::new(RichText::new(&author.name).strong()).truncate())
                .on_hover_text(&author.email);
            ui.scope(|ui| {
                ui.set_min_width(150.0);
                ui.horizontal(|ui| {
                    let (track, _) = ui.allocate_exact_size(Vec2::new(56.0, 8.0), Sense::hover());
                    ui.painter().rect_filled(track, 2.0, c.subtle_bg);
                    let filled = Rect::from_min_size(track.min, Vec2::new(track.width() * author.commits as f32 / max as f32, 8.0));
                    ui.painter().rect_filled(filled, 2.0, c.accent);
                    ui.label(commits_text(author.commits));
                });
            });
            ui.scope(|ui| {
                ui.set_min_width(130.0);
                ui.horizontal(|ui| {
                    ui.label(RichText::new(format!("+{}", author.added)).color(c.added));
                    ui.label(RichText::new(format!("−{}", author.removed)).color(c.removed));
                });
            });
            ui.scope(|ui| {
                ui.set_min_width(96.0);
                ui.label(RichText::new(date_text(author.last)).small().color(c.muted));
            });
        });
    }
    if authors.len() > AUTHOR_LIMIT {
        ui.label(RichText::new(format!("and {} more", authors.len() - AUTHOR_LIMIT)).small().color(c.muted));
    }
}

fn files(ui: &mut Ui, cx: &mut Ctx, files: &[FileStat]) {
    let c = theme::of(ui);
    widgets::section(ui, "Most changed files");
    ui.add_space(4.0);
    if files.is_empty() {
        ui.label(RichText::new("No files changed in this period.").small().color(c.muted));
        return;
    }
    let path_width = (ui.available_width() - 190.0).max(120.0);
    for file in files {
        ui.horizontal(|ui| {
            let shown = truncate_from_left(ui, &file.path, path_width);
            let response = left_cell(ui, [path_width, 20.0], egui::Label::new(RichText::new(shown)).sense(Sense::click()));
            let response =
                response.on_hover_text(format!("{}\nOpen its history", file.path)).on_hover_cursor(egui::CursorIcon::PointingHand);
            if response.clicked() {
                cx.open(Box::new(FileHistoryWindow::new(file.path.clone())));
            }
            ui.scope(|ui| {
                ui.set_min_width(90.0);
                ui.label(commits_text(file.commits));
            });
            ui.scope(|ui| {
                ui.set_min_width(90.0);
                ui.label(RichText::new(format!("{} lines", file.lines)).color(c.muted));
            });
        });
    }
}

fn folders(ui: &mut Ui, folders: &[FolderStat]) {
    let c = theme::of(ui);
    widgets::section(ui, "Folders");
    ui.add_space(4.0);
    let max = folders.iter().map(|folder| folder.commits).max().unwrap_or(0).max(1);
    let name_width = (ui.available_width() - 190.0).max(120.0);
    for folder in folders.iter().take(FOLDER_LIMIT) {
        ui.horizontal(|ui| {
            left_cell(ui, [name_width, 20.0], egui::Label::new(RichText::new(&folder.name)).truncate());
            ui.scope(|ui| {
                ui.set_min_width(190.0);
                ui.horizontal(|ui| {
                    let (track, _) = ui.allocate_exact_size(Vec2::new(90.0, 8.0), Sense::hover());
                    ui.painter().rect_filled(track, 2.0, c.subtle_bg);
                    let filled = Rect::from_min_size(track.min, Vec2::new(track.width() * folder.commits as f32 / max as f32, 8.0));
                    ui.painter().rect_filled(filled, 2.0, c.accent);
                    ui.label(commits_text(folder.commits));
                });
            });
        });
    }
    if folders.len() > FOLDER_LIMIT {
        ui.label(RichText::new(format!("and {} more", folders.len() - FOLDER_LIMIT)).small().color(c.muted));
    }
}

/// A column heading of a fixed width.
fn header(ui: &mut Ui, width: f32, text: &str) {
    let c = theme::of(ui);
    left_cell(ui, [width, 16.0], egui::Label::new(RichText::new(text).small().color(c.muted)));
}

fn commits_text(count: usize) -> RichText {
    RichText::new(if count == 1 { "1 commit".to_string() } else { format!("{count} commits") })
}

/// The start of a path cut from the left so that its file name stays visible, with an ellipsis.
fn truncate_from_left(ui: &Ui, path: &str, max_width: f32) -> String {
    let font = egui::TextStyle::Body.resolve(ui.style());
    let color: Color32 = ui.visuals().text_color();
    let width = |text: &str| ui.painter().layout_no_wrap(text.to_string(), font.clone(), color).size().x;
    if width(path) <= max_width {
        return path.to_string();
    }
    let chars: Vec<char> = path.chars().collect();
    let tail = |start: usize| -> String { std::iter::once('…').chain(chars[start..].iter().copied()).collect() };
    // Narrower tails are shorter, so the shortest one that fits is found by bisection.
    let (mut low, mut high) = (1, chars.len());
    while low < high {
        let middle = (low + high) / 2;
        if width(&tail(middle)) <= max_width {
            high = middle;
        } else {
            low = middle + 1;
        }
    }
    tail(low)
}

/// A date as "3 Nov 2026", from a time in seconds since the Unix epoch, in UTC.
fn date_text(time: i64) -> String {
    let (year, month, day) = civil_date(time.div_euclid(86_400));
    format!("{day} {} {year}", MONTHS[(month - 1) as usize])
}

fn message(error: &GitError) -> String {
    match error {
        GitError::CommandFailed { message, .. } => message.clone(),
        other => other.to_string(),
    }
}

/// A fixed-width table cell whose contents start at its left edge (`add_sized` would centre
/// them), so names and paths line up down a column.
fn left_cell(ui: &mut Ui, size: [f32; 2], widget: impl egui::Widget) -> egui::Response {
    ui.allocate_ui_with_layout(Vec2::new(size[0], size[1]), egui::Layout::left_to_right(egui::Align::Center), |ui| {
        ui.set_min_width(size[0]);
        ui.add(widget)
    })
    .inner
}
