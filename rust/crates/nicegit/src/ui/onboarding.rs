//! The first-run welcome and tour. Sprig, NiceGit's mascot, introduces the app, confirms the name
//! and email Git puts on commits, and walks through staging, committing and branching in a practice
//! repository. Every screen can be skipped; the tour can be taken again from the command palette or
//! Settings.

use std::path::{Path, PathBuf};

use egui::{Align2, Color32, Id, Key, LayerId, Order, Pos2, Rect, RichText, Stroke, StrokeKind, Ui};
use nicegit_core::client::GitClient;

use crate::app::NiceGitApp;
use crate::i18n::{tr, tr_fmt};
use crate::theme;
use crate::tools::widgets;

/// The number of steps the tour has before its closing card.
const TOUR_STEPS: usize = 6;
/// The width of a coach-mark bubble.
const BUBBLE_WIDTH: f32 = 320.0;
/// The height a coach-mark bubble is placed with, which is enough for its longest text.
const BUBBLE_HEIGHT: f32 = 230.0;

/// Where the onboarding is up to.
pub enum Onboarding {
    /// The welcome page, with Sprig waving.
    Intro,
    /// The page that confirms the commit name and email, with the fields as typed so far.
    Identity { name: String, email: String, error: Option<String>, after: AfterIdentity },
    /// The coach marks over the window. `start_head` is the checkout's HEAD when the commit step
    /// began, so a new commit moves the tour on.
    Tour { step: TourStep, start_head: Option<String> },
}

impl Onboarding {
    /// The welcome page, shown at first run.
    pub fn intro() -> Self {
        Onboarding::Intro
    }
}

/// The onboarding a new window starts with: the welcome page at first run only, meaning the tour
/// has not been finished or skipped and no repository was named on the command line.
pub fn at_start(onboarding_done: bool, opening_nothing: bool) -> Option<Onboarding> {
    (!onboarding_done && opening_nothing).then(Onboarding::intro)
}

/// What the person chose on the identity page, so the choice runs once the details are confirmed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AfterIdentity {
    /// Start the tour in the practice repository.
    Tour,
    /// Choose an existing repository to open.
    Open,
    /// Clone a repository.
    Clone,
    /// Finish the onboarding and show the welcome screen.
    Nothing,
}

/// The steps of the tour, in order. `Done` is the closing card.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TourStep {
    History,
    Sidebar,
    Changes,
    Commit,
    Toolbar,
    Palette,
    Done,
}

impl TourStep {
    const ORDER: [TourStep; 7] =
        [TourStep::History, TourStep::Sidebar, TourStep::Changes, TourStep::Commit, TourStep::Toolbar, TourStep::Palette, TourStep::Done];

    fn index(self) -> usize {
        Self::ORDER.iter().position(|step| *step == self).unwrap_or(0)
    }

    /// The step's number in the counter, from 1.
    fn number(self) -> usize {
        self.index() + 1
    }

    fn next(self) -> Self {
        Self::ORDER[(self.index() + 1).min(Self::ORDER.len() - 1)]
    }

    fn previous(self) -> Self {
        Self::ORDER[self.index().saturating_sub(1)]
    }
}

/// Where the coach marks point. Each is recorded while the window is laid out, and cleared at its
/// start, so a step without a target shows its bubble at the centre.
#[derive(Clone, Copy, Debug, Default)]
pub struct TourTargets {
    pub history: Option<Rect>,
    pub sidebar: Option<Rect>,
    pub changes: Option<Rect>,
    pub commit_box: Option<Rect>,
    pub toolbar: Option<Rect>,
}

/// Sprig's pictures, decoded on first use.
#[derive(Default)]
pub struct SprigImages {
    loaded: Option<Pictures>,
}

/// The decoded pictures of Sprig, as textures. Each is absent only if it failed to decode.
#[derive(Clone, Default)]
pub struct Pictures {
    wave: Option<egui::TextureHandle>,
    point: Option<egui::TextureHandle>,
    cheer: Option<egui::TextureHandle>,
}

#[derive(Clone, Copy)]
enum Pose {
    Wave,
    Point,
    Cheer,
}

impl SprigImages {
    fn get(&mut self, ctx: &egui::Context) -> Pictures {
        self.loaded
            .get_or_insert_with(|| Pictures {
                wave: decode(ctx, "sprig-wave", include_bytes!("../../../../assets/mascot/sprig-wave.png")),
                point: decode(ctx, "sprig-point", include_bytes!("../../../../assets/mascot/sprig-point.png")),
                cheer: decode(ctx, "sprig-cheer", include_bytes!("../../../../assets/mascot/sprig-cheer.png")),
            })
            .clone()
    }
}

fn decode(ctx: &egui::Context, name: &str, bytes: &[u8]) -> Option<egui::TextureHandle> {
    let image = image::load_from_memory(bytes).ok()?.into_rgba8();
    let size = [image.width() as usize, image.height() as usize];
    let color = egui::ColorImage::from_rgba_unmultiplied(size, image.as_raw());
    Some(ctx.load_texture(name, color, egui::TextureOptions::LINEAR))
}

impl Pictures {
    fn of(&self, pose: Pose) -> Option<&egui::TextureHandle> {
        match pose {
            Pose::Wave => self.wave.as_ref(),
            Pose::Point => self.point.as_ref(),
            Pose::Cheer => self.cheer.as_ref(),
        }
    }
}

/// What a screen asks for. Applied after the screen is drawn, so the screen never changes the app
/// while it is being drawn.
enum Move {
    Stay,
    Intro,
    Identity(AfterIdentity),
    Save { name: String, email: String, after: AfterIdentity },
    Follow { after: AfterIdentity, name: String, email: String },
    Step(TourStep),
    Finish,
}

impl NiceGitApp {
    /// Draws the onboarding over the window, if it is running. Called after the window is laid out
    /// so it paints on top.
    pub fn onboarding_ui(&mut self, ctx: &egui::Context) {
        let Some(mut state) = self.onboarding.take() else {
            return;
        };
        let pictures = self.sprig.get(ctx);
        let moved = match &mut state {
            Onboarding::Intro => intro_page(ctx, &pictures),
            Onboarding::Identity { name, email, error, after } => identity_page(ctx, &pictures, name, email, error, *after),
            Onboarding::Tour { step, start_head } => self.tour_page(ctx, &pictures, step, start_head),
        };
        self.apply_onboarding_move(state, moved);
    }

    /// Ends the onboarding for good: the welcome screen shows and the tour is not offered again
    /// unasked.
    pub fn finish_onboarding(&mut self) {
        self.onboarding = None;
        self.settings.onboarding_done = true;
    }

    fn apply_onboarding_move(&mut self, state: Onboarding, moved: Move) {
        // Anything that opens or creates a repository waits for the current action, as other
        // actions do; the page stays as it was so the person can try again.
        let needs_action = match &moved {
            Move::Save { after, .. } | Move::Follow { after, .. } => *after != AfterIdentity::Nothing,
            _ => false,
        };
        if needs_action && self.busy.is_some() {
            self.notify(tr("Wait for the current action to finish, then try again."), true);
            self.onboarding = Some(state);
            return;
        }
        match moved {
            Move::Stay => self.onboarding = Some(state),
            Move::Intro => self.onboarding = Some(Onboarding::Intro),
            Move::Identity(after) => self.onboarding = Some(self.identity_form(after)),
            Move::Save { name, email, after } => {
                let directory = std::env::temp_dir();
                match GitClient::new().set_global_identity(&name, &email, self.identity_file(), &directory) {
                    Ok(()) => self.follow_onboarding(after, &name, &email),
                    Err(error) => {
                        self.onboarding = Some(Onboarding::Identity { name, email, error: Some(error.to_string()), after });
                    }
                }
            }
            Move::Follow { after, name, email } => self.follow_onboarding(after, &name, &email),
            Move::Step(step) => self.onboarding = Some(Onboarding::Tour { step, start_head: None }),
            Move::Finish => self.finish_onboarding(),
        }
    }

    /// Runs what the person chose once the identity is settled.
    fn follow_onboarding(&mut self, after: AfterIdentity, name: &str, email: &str) {
        match after {
            AfterIdentity::Tour => self.start_practice_tour(name, email),
            AfterIdentity::Open => {
                self.finish_onboarding();
                self.choose_folder();
            }
            AfterIdentity::Clone => {
                self.finish_onboarding();
                self.dialog = Some(crate::ui::dialogs::Dialog::clone_repository());
            }
            AfterIdentity::Nothing => self.finish_onboarding(),
        }
    }

    /// Creates the practice repository and opens it at the first step of the tour.
    fn start_practice_tour(&mut self, name: &str, email: &str) {
        let parent = self.practice_parent();
        match GitClient::new().create_practice_repository(&parent, name, email) {
            Ok(path) => {
                self.onboarding = Some(Onboarding::Tour { step: TourStep::History, start_head: None });
                self.open(path);
            }
            Err(error) => {
                self.finish_onboarding();
                self.notify(tr_fmt!("The practice repository could not be made: {error}", error = error), true);
            }
        }
    }

    /// The identity page, with the name and email Git has now.
    fn identity_form(&self, after: AfterIdentity) -> Onboarding {
        let (name, email) = GitClient::new().global_identity(self.identity_file(), &std::env::temp_dir());
        Onboarding::Identity { name, email, error: None, after }
    }

    /// The folder the practice repository is made in: the person's Documents folder when it exists,
    /// or else the temporary folder.
    #[cfg(not(test))]
    fn practice_parent(&self) -> PathBuf {
        let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(PathBuf::from);
        home.map(|home| home.join("Documents")).filter(|documents| documents.is_dir()).unwrap_or_else(std::env::temp_dir)
    }

    /// The test's folder for the practice repository, or else the temporary folder, so a test never
    /// writes into the person's folders.
    #[cfg(test)]
    fn practice_parent(&self) -> PathBuf {
        self.practice_parent.clone().unwrap_or_else(std::env::temp_dir)
    }

    /// The file the identity is read from and saved to: the test's file in tests, otherwise the
    /// person's own Git settings.
    #[cfg(test)]
    fn identity_file(&self) -> Option<&Path> {
        self.identity_file.as_deref()
    }

    #[cfg(not(test))]
    fn identity_file(&self) -> Option<&Path> {
        None
    }

    /// The tour's coach mark for a step, if the step points at part of the window.
    fn tour_target(&self, step: TourStep) -> Option<Rect> {
        match step {
            TourStep::History => self.tour_targets.history,
            TourStep::Sidebar => self.tour_targets.sidebar,
            TourStep::Changes => self.tour_targets.changes,
            TourStep::Commit => self.tour_targets.commit_box,
            TourStep::Toolbar => self.tour_targets.toolbar,
            TourStep::Palette | TourStep::Done => None,
        }
    }

    /// One frame of the tour: moves on when the person has done the step's action, then draws the
    /// step's coach mark.
    fn tour_page(&self, ctx: &egui::Context, pictures: &Pictures, step: &mut TourStep, start_head: &mut Option<String>) -> Move {
        if ctx.input(|input| input.key_pressed(Key::Escape)) {
            return Move::Finish;
        }
        if let Some(snapshot) = self.snapshot() {
            match *step {
                TourStep::Changes => {
                    if snapshot.status.iter().any(|entry| entry.is_staged()) {
                        return Move::Step(TourStep::Commit);
                    }
                }
                TourStep::Commit => match (start_head.as_ref(), snapshot.head_hash.as_ref()) {
                    (None, _) => *start_head = snapshot.head_hash.clone(),
                    (Some(start), Some(head)) if start != head => return Move::Step(TourStep::Toolbar),
                    _ => {}
                },
                _ => {}
            }
        }
        let screen = ctx.content_rect();
        if *step == TourStep::Done {
            dim(ctx, screen, None);
            return done_card(ctx, pictures);
        }
        if self.snapshot().is_none() {
            loading_bubble(ctx, screen);
            return Move::Stay;
        }
        let target = self.tour_target(*step);
        dim(ctx, screen, target);
        coach_mark(ctx, pictures, *step, target, screen)
    }
}

/// The id of one of the onboarding's areas: the same every frame, so the area keeps its place.
fn area_id(ctx: &egui::Context, name: &str) -> Id {
    ctx.viewport_id().root_ui_id().with_salt(egui::IdSalt::new(name))
}

/// Dims the window except for the target, which is outlined. Paints only, so clicks reach the
/// window under it. With no target, the whole window is dimmed.
fn dim(ctx: &egui::Context, screen: Rect, hole: Option<Rect>) {
    // Under the bubble, which is a foreground area, and over the window's panels.
    let painter = ctx.layer_painter(LayerId::new(Order::Middle, area_id(ctx, "tour-dim")));
    let shade = Color32::from_black_alpha(140);
    let accent = theme::colors(ctx.global_style().visuals.dark_mode).accent;
    match hole {
        None => {
            painter.rect_filled(screen, 0.0, shade);
        }
        Some(hole) => {
            let hole = hole.expand(6.0).intersect(screen);
            let top = Rect::from_min_max(screen.min, Pos2::new(screen.max.x, hole.min.y));
            let bottom = Rect::from_min_max(Pos2::new(screen.min.x, hole.max.y), screen.max);
            let left = Rect::from_min_max(Pos2::new(screen.min.x, hole.min.y), Pos2::new(hole.min.x, hole.max.y));
            let right = Rect::from_min_max(Pos2::new(hole.max.x, hole.min.y), Pos2::new(screen.max.x, hole.max.y));
            for part in [top, bottom, left, right] {
                painter.rect_filled(part, 0.0, shade);
            }
            painter.rect_stroke(hole, 8.0, Stroke::new(2.0, accent), StrokeKind::Outside);
        }
    }
}

/// Where a bubble goes: beside its target, to the right if there is room, else to the left, else
/// below; kept inside the window. Centred when there is no target.
fn bubble_position(screen: Rect, target: Option<Rect>, width: f32, height: f32) -> Pos2 {
    let edge = 8.0;
    let margin = 16.0;
    let Some(target) = target else {
        return Pos2::new(screen.center().x - width / 2.0, screen.center().y - height / 2.0);
    };
    let top_limit = (screen.bottom() - height - edge).max(screen.top() + edge);
    let top = target.top().clamp(screen.top() + edge, top_limit);
    let right = target.right() + margin;
    if right + width <= screen.right() - edge {
        return Pos2::new(right, top);
    }
    let left = target.left() - margin - width;
    if left >= screen.left() + edge {
        return Pos2::new(left, top);
    }
    let x_limit = (screen.right() - width - edge).max(screen.left() + edge);
    let x = (target.center().x - width / 2.0).clamp(screen.left() + edge, x_limit);
    let y = (target.bottom() + margin).clamp(screen.top() + edge, top_limit);
    Pos2::new(x, y)
}

/// The text of a tour step: its title and its body.
fn step_copy(step: TourStep) -> (&'static str, &'static str) {
    match step {
        TourStep::History => (
            tr("Your history"),
            tr("Each dot is a commit, newest at the top. The lines show where branches split off and join back."),
        ),
        TourStep::Sidebar => (
            tr("Branches and more"),
            tr("Your branches, remotes, tags and stashes are listed here. Double-click a branch to switch to it."),
        ),
        TourStep::Changes => (
            tr("Stage a change"),
            tr("I've changed hello.txt for you. Click it to see the change, then press Stage, or Stage all."),
        ),
        TourStep::Commit => (tr("Commit it"), tr("Write a short message saying what you changed, then press Commit.")),
        TourStep::Toolbar => (
            tr("Share your work"),
            tr("Pull brings in other people's commits, Push sends yours, and Branch starts a new line of work. Undo takes back most actions."),
        ),
        TourStep::Palette => {
            let text = if cfg!(target_os = "macos") {
                tr("Press Shift-Cmd-P to search every command.")
            } else {
                tr("Press Shift-Ctrl-P to search every command.")
            };
            (tr("Find anything"), text)
        }
        TourStep::Done => (tr("You're ready"), tr("That's the basics. The practice repository stays open for you to play in.")),
    }
}

/// The bubble for a step: Sprig pointing, the step's title and text, its counter, and its buttons.
fn coach_mark(ctx: &egui::Context, pictures: &Pictures, step: TourStep, target: Option<Rect>, screen: Rect) -> Move {
    let mut moved = Move::Stay;
    let position = bubble_position(screen, target, BUBBLE_WIDTH, BUBBLE_HEIGHT);
    let (title, text) = step_copy(step);
    egui::Area::new(area_id(ctx, "tour-bubble")).order(Order::Foreground).fixed_pos(position).show(ctx, |ui| {
        egui::Frame::popup(ui.style()).show(ui, |ui| {
            ui.set_width(BUBBLE_WIDTH - 24.0);
            let c = theme::of(ui);
            ui.horizontal_top(|ui| {
                sprig(ui, pictures, Pose::Point, 64.0);
                ui.add_space(8.0);
                ui.vertical(|ui| {
                    ui.set_width(BUBBLE_WIDTH - 24.0 - 64.0 - 16.0);
                    ui.label(RichText::new(title).size(16.0).strong());
                    ui.add_space(4.0);
                    ui.add(egui::Label::new(RichText::new(text)).wrap());
                    ui.add_space(6.0);
                    let counter = tr_fmt!("Step {number} of {total}", number = step.number(), total = TOUR_STEPS);
                    ui.label(RichText::new(counter).small().color(c.muted));
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if step != TourStep::History && ui.button(tr("Back")).clicked() {
                            moved = Move::Step(step.previous());
                        }
                        if widgets::primary_button(ui, tr("Next"), true).clicked() {
                            moved = Move::Step(step.next());
                        }
                        if ui.link(tr("Skip tour")).clicked() {
                            moved = Move::Finish;
                        }
                    });
                });
            });
        });
    });
    moved
}

/// The closing card: Sprig cheering, and what to do next.
fn done_card(ctx: &egui::Context, pictures: &Pictures) -> Move {
    let mut moved = Move::Stay;
    let (title, text) = step_copy(TourStep::Done);
    egui::Area::new(area_id(ctx, "tour-done")).order(Order::Foreground).anchor(Align2::CENTER_CENTER, egui::vec2(0.0, 0.0)).show(
        ctx,
        |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.set_width(400.0);
                ui.vertical_centered(|ui| {
                    sprig(ui, pictures, Pose::Cheer, 160.0);
                    ui.label(RichText::new(title).size(26.0).strong());
                    ui.add_space(4.0);
                    ui.add(egui::Label::new(RichText::new(text)).wrap());
                    ui.add_space(14.0);
                    if big_button(ui, tr("Open my repository…"), true).clicked() {
                        moved = Move::Follow { after: AfterIdentity::Open, name: String::new(), email: String::new() };
                    }
                    if big_button(ui, tr("Clone a repository…"), false).clicked() {
                        moved = Move::Follow { after: AfterIdentity::Clone, name: String::new(), email: String::new() };
                    }
                    ui.add_space(4.0);
                    if ui.link(tr("Finish")).clicked() {
                        moved = Move::Finish;
                    }
                });
            });
        },
    );
    moved
}

/// The small note shown while the practice repository is still being read.
fn loading_bubble(ctx: &egui::Context, screen: Rect) {
    egui::Area::new(area_id(ctx, "tour-loading")).order(Order::Foreground).fixed_pos(bubble_position(screen, None, 300.0, 40.0)).show(
        ctx,
        |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                widgets::loading(ui, tr("Opening the practice repository…"));
            });
        },
    );
}

/// Draws Sprig at `size` wide, keeping the picture's shape. Nothing is drawn if it failed to load.
fn sprig(ui: &mut Ui, pictures: &Pictures, pose: Pose, size: f32) {
    if let Some(texture) = pictures.of(pose) {
        let shape = texture.size_vec2();
        let height = if shape.x > 0.0 { size * shape.y / shape.x } else { size };
        ui.add(egui::Image::new((texture.id(), egui::vec2(size, height))).alt_text("Sprig"));
    }
}

/// A large button for the onboarding pages; the primary one is filled with the accent colour.
fn big_button(ui: &mut Ui, text: &str, primary: bool) -> egui::Response {
    let c = theme::of(ui);
    let label = RichText::new(text).size(15.0);
    let button = if primary { egui::Button::new(label.color(c.accent_text).strong()).fill(c.accent) } else { egui::Button::new(label) };
    ui.add(button.min_size(egui::vec2(if primary { 240.0 } else { 180.0 }, 40.0)))
}

/// A full-window page, drawn over everything else so nothing behind it can be clicked.
fn page(ctx: &egui::Context, pictures: &Pictures, pose: Pose, content: impl FnOnce(&mut Ui) -> Move) -> Move {
    let screen = ctx.content_rect();
    let mut moved = Move::Stay;
    egui::Area::new(area_id(ctx, "onboarding-page")).order(Order::Foreground).fixed_pos(screen.min).show(ctx, |ui| {
        ui.set_width(screen.width());
        ui.set_height(screen.height());
        ui.painter().rect_filled(screen, 0.0, ui.visuals().window_fill);
        egui::ScrollArea::vertical().id_salt("onboarding-page-scroll").auto_shrink([false, false]).show(ui, |ui| {
            moved = page_layout(ui, pictures, pose, content);
        });
    });
    moved
}

/// The text beside Sprig on a wide window, or the text under Sprig on a narrow one.
fn page_layout(ui: &mut Ui, pictures: &Pictures, pose: Pose, content: impl FnOnce(&mut Ui) -> Move) -> Move {
    let mut moved = Move::Stay;
    let wide = ui.available_width() >= 760.0;
    let width = (ui.available_width() - 48.0).clamp(200.0, 760.0);
    let margin = ((ui.available_width() - width) / 2.0).max(0.0);
    // Roughly centred: the pages are about 360 points tall on a wide window.
    let tall = if wide { 360.0 } else { 560.0 };
    ui.add_space(((ui.available_height() - tall) / 2.0).max(32.0));
    ui.horizontal_top(|ui| {
        ui.add_space(margin);
        ui.vertical(|ui| {
            ui.set_width(width);
            if wide {
                ui.horizontal_top(|ui| {
                    ui.vertical(|ui| {
                        ui.set_width(width - 300.0);
                        moved = content(ui);
                    });
                    ui.add_space(24.0);
                    sprig(ui, pictures, pose, 260.0);
                });
            } else {
                ui.vertical_centered(|ui| sprig(ui, pictures, pose, 160.0));
                moved = content(ui);
            }
        });
    });
    ui.add_space(32.0);
    moved
}

/// The welcome: Sprig waving, the tour, the buttons for people who know Git already, and skipping.
fn intro_page(ctx: &egui::Context, pictures: &Pictures) -> Move {
    page(ctx, pictures, Pose::Wave, |ui| {
        let c = theme::of(ui);
        let mut moved = Move::Stay;
        ui.label(RichText::new(tr("Hi, I'm Sprig")).size(26.0).strong());
        ui.add_space(6.0);
        ui.label(RichText::new(tr("I'll show you around NiceGit in about a minute, in a practice repository you can't break.")).size(15.0));
        ui.add_space(18.0);
        if big_button(ui, tr("Start the tour"), true).clicked() {
            moved = Move::Identity(AfterIdentity::Tour);
        }
        ui.add_space(18.0);
        ui.label(RichText::new(tr("or")).color(c.muted));
        ui.add_space(8.0);
        ui.label(RichText::new(tr("Already know your way around?")).size(15.0).strong());
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            if big_button(ui, tr("Open repository…"), false).clicked() {
                moved = Move::Identity(AfterIdentity::Open);
            }
            if big_button(ui, tr("Clone repository…"), false).clicked() {
                moved = Move::Identity(AfterIdentity::Clone);
            }
        });
        ui.add_space(18.0);
        if ui.link(tr("Skip for now")).clicked() {
            moved = Move::Identity(AfterIdentity::Nothing);
        }
        moved
    })
}

/// The page that confirms the commit name and email. Prefilled from the person's Git settings.
fn identity_page(
    ctx: &egui::Context,
    pictures: &Pictures,
    name: &mut String,
    email: &mut String,
    error: &mut Option<String>,
    after: AfterIdentity,
) -> Move {
    page(ctx, pictures, Pose::Point, |ui| {
        let c = theme::of(ui);
        let mut moved = Move::Stay;
        if ui.link(tr("Back")).clicked() {
            moved = Move::Intro;
        }
        ui.add_space(4.0);
        ui.label(RichText::new(tr("Confirm your Git details")).size(26.0).strong());
        ui.add_space(6.0);
        ui.label(
            RichText::new(tr(
                "Git puts your name and email on every commit you make. They're saved in your own Git settings, so other Git tools use them too.",
            ))
            .size(15.0),
        );
        if name.trim().is_empty() && email.trim().is_empty() {
            ui.add_space(4.0);
            ui.label(RichText::new(tr("Git doesn't know who you are yet.")).color(c.muted));
        }
        ui.add_space(16.0);
        let name_label = ui.label(tr("Name"));
        ui.add(egui::TextEdit::singleline(name).desired_width(360.0)).labelled_by(name_label.id);
        ui.add_space(8.0);
        let email_label = ui.label(tr("Email"));
        let email_edit = ui.add(egui::TextEdit::singleline(email).desired_width(360.0)).labelled_by(email_label.id);
        if let Some(message) = error.as_deref() {
            ui.add_space(6.0);
            ui.label(RichText::new(message).color(c.danger));
        }
        ui.add_space(14.0);
        // Enter in the email field confirms, like the button.
        let enter = email_edit.lost_focus() && ui.input(|input| input.key_pressed(Key::Enter));
        let save = big_button(ui, tr("Use these for my commits"), true).clicked() || enter;
        if save {
            if name.trim().is_empty() || !email.trim().contains('@') {
                *error = Some(tr("Enter your name and an email address.").to_string());
            } else {
                *error = None;
                moved = Move::Save { name: name.clone(), email: email.clone(), after };
            }
        }
        ui.add_space(8.0);
        if ui.link(tr("Skip for now")).clicked() {
            moved = Move::Follow { after, name: name.clone(), email: email.clone() };
        }
        moved
    })
}
