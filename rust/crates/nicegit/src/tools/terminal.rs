#![allow(dead_code)]
//! Embedded terminal.
//!
//! A [`TerminalSession`] runs one shell on a pseudo-terminal and parses its output with
//! `alacritty_terminal`; [`TerminalSession::ui`] draws the grid with egui and turns keyboard
//! and mouse input into bytes for the shell.
//!
//! Shells outlive the window. [`TerminalWindow`] is only a view: the tabs of each repository
//! live in a process-wide registry, so closing the window (or its last running tab) keeps the
//! shells, and opening the terminal again for the same repository reattaches to them. Closing
//! a tab other than the last one ends its shell.

use std::borrow::Cow;
use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};
use std::thread::JoinHandle;

use alacritty_terminal::event::{Event, EventListener, Notify, OnResize, WindowSize};
use alacritty_terminal::event_loop::{EventLoop, EventLoopSender, Msg, Notifier, State};
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Column, Line, Point, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::cell::{Cell, Flags};
use alacritty_terminal::term::color::Colors;
use alacritty_terminal::term::{Config, RenderableContent, Term, TermMode};
use alacritty_terminal::tty::{self, Shell};
use alacritty_terminal::vte::ansi::{Color, CursorShape, NamedColor, Rgb};
use egui::text::LayoutJob;
use egui::{Color32, EventFilter, FontId, Key, Modifiers, Pos2, Rect, RichText, Sense, Stroke, TextFormat, TextStyle};
use egui_phosphor::regular as icon;

use crate::theme;
use crate::tools::widgets;
use crate::tools::{Ctx, ToolWindow};

/// The grid size in cells, as alacritty's `Dimensions` expects it.
#[derive(Clone, Copy)]
struct GridSize {
    cols: usize,
    lines: usize,
}

impl Dimensions for GridSize {
    fn total_lines(&self) -> usize {
        self.lines
    }

    fn screen_lines(&self) -> usize {
        self.lines
    }

    fn columns(&self) -> usize {
        self.cols
    }
}

/// State shared between the UI thread, the PTY thread, and the event listener.
#[derive(Default)]
struct Shared {
    title: Mutex<Option<String>>,
    exited: AtomicBool,
    exit_code: Mutex<Option<i32>>,
    /// The colours answers to colour queries use; refreshed every frame.
    palette: Mutex<Palette>,
    /// The size answers to size queries use.
    size: Mutex<Option<WindowSize>>,
    /// Writes replies (cursor reports, colour queries) back to the shell.
    sender: OnceLock<EventLoopSender>,
}

/// Receives events from the terminal and the PTY thread.
#[derive(Clone)]
struct Listener {
    ctx: egui::Context,
    shared: Arc<Shared>,
}

impl Listener {
    fn reply(&self, bytes: Vec<u8>) {
        if let Some(sender) = self.shared.sender.get() {
            let _ = sender.send(Msg::Input(Cow::Owned(bytes)));
        }
    }
}

impl EventListener for Listener {
    fn send_event(&self, event: Event) {
        match event {
            Event::Title(title) => {
                *lock(&self.shared.title) = Some(title);
                self.ctx.request_repaint();
            }
            Event::ResetTitle => {
                *lock(&self.shared.title) = None;
                self.ctx.request_repaint();
            }
            Event::ChildExit(status) => {
                *lock(&self.shared.exit_code) = status.code();
                self.shared.exited.store(true, Ordering::Release);
                self.ctx.request_repaint();
            }
            Event::PtyWrite(text) => self.reply(text.into_bytes()),
            Event::ClipboardStore(_, text) => self.ctx.copy_text(text),
            Event::ColorRequest(index, format) => {
                let rgb = lock(&self.shared.palette).rgb_at(index);
                self.reply(format(rgb).into_bytes());
            }
            Event::TextAreaSizeRequest(format) => {
                let size = (*lock(&self.shared.size)).unwrap_or(WindowSize { num_cols: 80, num_lines: 24, cell_width: 8, cell_height: 16 });
                self.reply(format(size).into_bytes());
            }
            Event::Wakeup | Event::MouseCursorDirty | Event::CursorBlinkingChange | Event::Bell => self.ctx.request_repaint(),
            // Reading the system clipboard for OSC 52 is not supported; the request is ignored.
            Event::ClipboardLoad(..) | Event::Exit => {}
        }
    }
}

/// One shell on a pseudo-terminal, with its emulated screen.
pub struct TerminalSession {
    cwd: PathBuf,
    term: Arc<FairMutex<Term<Listener>>>,
    shared: Arc<Shared>,
    notifier: Notifier,
    thread: Option<JoinHandle<(EventLoop<tty::Pty, Listener>, State)>>,
    /// The size last applied to the terminal and the PTY.
    size: WindowSize,
    scroll_remainder: f32,
    restart_error: Option<String>,
}

impl TerminalSession {
    /// Starts the user's shell as a login shell in `cwd`.
    pub fn new(ctx: &egui::Context, cwd: &Path) -> io::Result<Self> {
        let shared = Arc::new(Shared::default());
        let listener = Listener { ctx: ctx.clone(), shared: Arc::clone(&shared) };
        let size = WindowSize { num_cols: 80, num_lines: 24, cell_width: 8, cell_height: 16 };
        *lock(&shared.size) = Some(size);

        let pty = spawn_pty(cwd, size)?;
        let term = Arc::new(FairMutex::new(Term::new(Config::default(), &grid_size(size), listener.clone())));
        // Drains the shell's output after it exits, so its last lines stay visible.
        let event_loop = EventLoop::new(Arc::clone(&term), listener, pty, true, false)?;
        let sender = event_loop.channel();
        let _ = shared.sender.set(sender.clone());
        let thread = event_loop.spawn();

        Ok(Self {
            cwd: cwd.to_path_buf(),
            term,
            shared,
            notifier: Notifier(sender),
            thread: Some(thread),
            size,
            scroll_remainder: 0.0,
            restart_error: None,
        })
    }

    pub fn cwd(&self) -> &Path {
        &self.cwd
    }

    /// The window title the shell set, or "Shell", with " (exited)" once the shell has ended.
    pub fn title(&self) -> String {
        let title = lock(&self.shared.title).clone().unwrap_or_else(|| "Shell".to_string());
        if self.is_exited() { format!("{title} (exited)") } else { title }
    }

    /// Whether the shell has ended.
    pub fn is_exited(&self) -> bool {
        self.shared.exited.load(Ordering::Acquire) || self.thread.as_ref().is_some_and(|thread| thread.is_finished())
    }

    /// The shell's exit status code, once it has ended.
    pub fn exit_code(&self) -> Option<i32> {
        *lock(&self.shared.exit_code)
    }

    /// The whole screen and scrollback as plain text.
    pub fn screen_text(&self) -> String {
        let term = self.term.lock();
        term.bounds_to_string(Point::new(term.topmost_line(), Column(0)), Point::new(term.bottommost_line(), term.last_column()))
    }

    /// Draws the terminal into the rest of `ui`, handling its keyboard and mouse input.
    pub fn ui(&mut self, ui: &mut egui::Ui) {
        let exited = self.is_exited();
        let mut restart = false;
        if exited || self.restart_error.is_some() {
            ui.horizontal(|ui| {
                if exited {
                    let status = match self.exit_code() {
                        Some(code) => format!("[Process exited with status {code}]"),
                        None => "[Process exited]".to_string(),
                    };
                    ui.label(RichText::new(status).color(theme::of(ui).muted));
                    restart = ui.button("Restart").clicked();
                }
                if let Some(error) = &self.restart_error {
                    widgets::error(ui, error);
                }
            });
        }
        if restart {
            match TerminalSession::new(ui.ctx(), &self.cwd) {
                Ok(session) => *self = session,
                Err(err) => self.restart_error = Some(format!("Could not start a shell: {err}")),
            }
        }

        let font = TextStyle::Monospace.resolve(ui.style());
        let (cw, ch) = ui.ctx().fonts_mut(|fonts| (fonts.glyph_width(&font, 'M'), fonts.row_height(&font)));
        let metrics = Metrics { cw: cw.max(1.0), ch: ch.max(1.0), font };

        let available = ui.available_size();
        let (rect, response) = ui.allocate_exact_size(egui::vec2(available.x.max(1.0), available.y.max(1.0)), Sense::click_and_drag());
        if response.clicked() || response.drag_started() {
            response.request_focus();
        }
        let focused = response.has_focus();
        if focused {
            // Keep Tab, arrows, and Escape for the shell instead of egui's focus navigation.
            let filter = EventFilter { tab: true, horizontal_arrows: true, vertical_arrows: true, escape: true };
            ui.memory_mut(|memory| memory.set_focus_lock_filter(response.id, filter));
        }

        let size = WindowSize {
            num_cols: u16::try_from((rect.width() / metrics.cw) as usize).unwrap_or(u16::MAX).max(2),
            num_lines: u16::try_from((rect.height() / metrics.ch) as usize).unwrap_or(u16::MAX).max(1),
            cell_width: metrics.cw.round().clamp(1.0, f32::from(u16::MAX)) as u16,
            cell_height: metrics.ch.round().clamp(1.0, f32::from(u16::MAX)) as u16,
        };
        let term_handle = Arc::clone(&self.term);
        let mut term = term_handle.lock();
        if !same_size(size, self.size) {
            self.apply_size(&mut term, size);
        }

        let palette = Palette::from_visuals(ui.visuals());
        *lock(&self.shared.palette) = palette;

        if focused {
            self.keyboard(ui, &mut term);
        }
        self.mouse(ui, &response, rect, &metrics, &mut term);

        let content = term.renderable_content();
        paint(ui, rect, &metrics, &palette, focused, usize::from(self.size.num_lines), content);
    }

    fn apply_size(&mut self, term: &mut Term<Listener>, size: WindowSize) {
        term.resize(grid_size(size));
        self.notifier.on_resize(size);
        self.size = size;
        *lock(&self.shared.size) = Some(size);
    }

    fn keyboard(&self, ui: &egui::Ui, term: &mut Term<Listener>) {
        let modifiers = ui.input(|i| i.modifiers);
        let events = ui.input(|i| i.events.clone());
        let app_cursor = term.mode().contains(TermMode::APP_CURSOR);
        let mac = cfg!(target_os = "macos");
        for event in events {
            match event {
                egui::Event::Text(text) => {
                    // Ctrl and Cmd combinations arrive as key events. AltGr sets Ctrl and Alt
                    // together and types characters, so it is not a control chord.
                    if (modifiers.ctrl && !modifiers.alt) || modifiers.mac_cmd || text.chars().any(|c| c.is_control()) {
                        continue;
                    }
                    type_bytes(&self.notifier, term, text.into_bytes());
                }
                egui::Event::Paste(text) => {
                    let bytes = paste_bytes(&text, term.mode().contains(TermMode::BRACKETED_PASTE));
                    type_bytes(&self.notifier, term, bytes);
                }
                egui::Event::Copy => {
                    // Cmd+C on macOS and Ctrl+Shift+C elsewhere copy; plain Ctrl+C interrupts.
                    if mac || modifiers.shift {
                        if let Some(text) = term.selection_to_string().filter(|text| !text.is_empty()) {
                            ui.ctx().copy_text(text);
                        }
                    } else {
                        type_bytes(&self.notifier, term, vec![0x03]);
                    }
                }
                egui::Event::Cut if !mac => type_bytes(&self.notifier, term, vec![0x18]),
                egui::Event::Key { key, pressed: true, modifiers: key_modifiers, .. } => {
                    if key_modifiers.mac_cmd {
                        continue;
                    }
                    if key_modifiers.shift && matches!(key, Key::PageUp | Key::PageDown) {
                        let scroll = if key == Key::PageUp { Scroll::PageUp } else { Scroll::PageDown };
                        term.scroll_display(scroll);
                        continue;
                    }
                    if let Some(bytes) = key_bytes(key, key_modifiers, app_cursor) {
                        type_bytes(&self.notifier, term, bytes);
                    }
                }
                _ => {}
            }
        }
    }

    fn mouse(&mut self, ui: &egui::Ui, response: &egui::Response, rect: Rect, metrics: &Metrics, term: &mut Term<Listener>) {
        let offset = term.grid().display_offset();
        let size = self.size;
        let point_at = |pos: Pos2| point_at(pos, rect, metrics, size, offset);

        if response.clicked() {
            term.selection = None;
        }
        if response.drag_started() {
            if let Some(origin) = ui.input(|i| i.pointer.press_origin()) {
                let (point, side) = point_at(origin);
                term.selection = Some(Selection::new(SelectionType::Simple, point, side));
            }
        } else if response.dragged() {
            if let (Some(pos), Some(selection)) = (response.interact_pointer_pos(), term.selection.as_mut()) {
                let (point, side) = point_at(pos);
                selection.update(point, side);
            }
        }
        if let Some(pos) = response.interact_pointer_pos() {
            if response.double_clicked() {
                let (point, side) = point_at(pos);
                term.selection = Some(Selection::new(SelectionType::Semantic, point, side));
            } else if response.triple_clicked() {
                let (point, side) = point_at(pos);
                term.selection = Some(Selection::new(SelectionType::Lines, point, side));
            }
        }

        if response.hovered() {
            let delta = ui.input(|i| i.smooth_scroll_delta.y);
            self.scroll_remainder += delta / metrics.ch;
            let lines = self.scroll_remainder.trunc() as i32;
            if lines != 0 {
                self.scroll_remainder -= lines as f32;
                let mode = *term.mode();
                if mode.contains(TermMode::ALT_SCREEN)
                    && mode.contains(TermMode::ALTERNATE_SCROLL)
                    && !mode.intersects(TermMode::MOUSE_MODE)
                {
                    // Full-screen programs such as pagers scroll by arrow keys.
                    let key = csi_final(if lines > 0 { 'A' } else { 'B' }, Modifiers::NONE, mode.contains(TermMode::APP_CURSOR));
                    self.notifier.notify(key.repeat(lines.unsigned_abs() as usize).into_bytes());
                } else {
                    term.scroll_display(Scroll::Delta(lines));
                }
            }
        }
    }
}

impl Drop for TerminalSession {
    fn drop(&mut self) {
        // Stops the PTY thread, which hangs up the shell. Dropping the thread's result can wait
        // for the shell to exit, so it happens on a separate thread instead of the UI thread.
        let _ = self.notifier.0.send(Msg::Shutdown);
        if let Some(thread) = self.thread.take() {
            std::thread::spawn(move || drop(thread));
        }
    }
}

/// The text cell size, in points.
struct Metrics {
    cw: f32,
    ch: f32,
    font: FontId,
}

fn grid_size(size: WindowSize) -> GridSize {
    GridSize { cols: usize::from(size.num_cols), lines: usize::from(size.num_lines) }
}

/// Whether two window sizes match. `WindowSize` does not implement `PartialEq`.
fn same_size(a: WindowSize, b: WindowSize) -> bool {
    (a.num_cols, a.num_lines, a.cell_width, a.cell_height) == (b.num_cols, b.num_lines, b.cell_width, b.cell_height)
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Sends bytes to the shell, first returning the view to the bottom of the scrollback.
fn type_bytes(notifier: &Notifier, term: &mut Term<Listener>, bytes: Vec<u8>) {
    if term.grid().display_offset() != 0 {
        term.scroll_display(Scroll::Bottom);
    }
    notifier.notify(bytes);
}

fn paste_bytes(text: &str, bracketed: bool) -> Vec<u8> {
    if bracketed {
        // Escape characters in the pasted text could end the paste early.
        let mut bytes = b"\x1b[200~".to_vec();
        bytes.extend_from_slice(text.replace('\x1b', "").as_bytes());
        bytes.extend_from_slice(b"\x1b[201~");
        bytes
    } else {
        text.replace('\n', "\r").into_bytes()
    }
}

/// The xterm modifier parameter: 1 plus shift (1), alt (2), and ctrl (4).
fn modifier_param(modifiers: Modifiers) -> u8 {
    1 + u8::from(modifiers.shift) + 2 * u8::from(modifiers.alt) + 4 * u8::from(modifiers.ctrl)
}

/// Cursor keys: `ESC [ X`, `ESC O X` in application cursor mode, or `ESC [ 1 ; m X` with modifiers.
fn csi_final(final_char: char, modifiers: Modifiers, app_cursor: bool) -> String {
    let param = modifier_param(modifiers);
    if param > 1 {
        format!("\x1b[1;{param}{final_char}")
    } else if app_cursor {
        format!("\x1bO{final_char}")
    } else {
        format!("\x1b[{final_char}")
    }
}

/// Keys such as Insert, Delete, PageUp, and the function keys from F5: `ESC [ n ~`.
fn tilde(number: u8, modifiers: Modifiers) -> String {
    let param = modifier_param(modifiers);
    if param > 1 { format!("\x1b[{number};{param}~") } else { format!("\x1b[{number}~") }
}

/// The bytes a key press sends to the shell, or None when the key is typed as text instead.
fn key_bytes(key: Key, modifiers: Modifiers, app_cursor: bool) -> Option<Vec<u8>> {
    // AltGr sets Ctrl and Alt together; the characters it types arrive as text.
    let ctrl = modifiers.ctrl && !modifiers.alt;
    let text = match key {
        Key::ArrowUp => csi_final('A', modifiers, app_cursor),
        Key::ArrowDown => csi_final('B', modifiers, app_cursor),
        Key::ArrowRight => csi_final('C', modifiers, app_cursor),
        Key::ArrowLeft => csi_final('D', modifiers, app_cursor),
        Key::Home => csi_final('H', modifiers, app_cursor),
        Key::End => csi_final('F', modifiers, app_cursor),
        Key::Insert => tilde(2, modifiers),
        Key::Delete => tilde(3, modifiers),
        Key::PageUp => tilde(5, modifiers),
        Key::PageDown => tilde(6, modifiers),
        Key::F1 => csi_final('P', modifiers, true),
        Key::F2 => csi_final('Q', modifiers, true),
        Key::F3 => csi_final('R', modifiers, true),
        Key::F4 => csi_final('S', modifiers, true),
        Key::F5 => tilde(15, modifiers),
        Key::F6 => tilde(17, modifiers),
        Key::F7 => tilde(18, modifiers),
        Key::F8 => tilde(19, modifiers),
        Key::F9 => tilde(20, modifiers),
        Key::F10 => tilde(21, modifiers),
        Key::F11 => tilde(23, modifiers),
        Key::F12 => tilde(24, modifiers),
        Key::Enter => "\r".to_string(),
        Key::Tab if modifiers.shift => "\x1b[Z".to_string(),
        Key::Tab => "\t".to_string(),
        Key::Escape => "\x1b".to_string(),
        Key::Backspace if ctrl => "\x08".to_string(),
        Key::Backspace if modifiers.alt => "\x1b\x7f".to_string(),
        Key::Backspace => "\x7f".to_string(),
        Key::Space if ctrl => "\0".to_string(),
        Key::OpenBracket if ctrl => "\x1b".to_string(),
        Key::Backslash if ctrl => "\x1c".to_string(),
        Key::CloseBracket if ctrl => "\x1d".to_string(),
        _ if ctrl => ctrl_letter(key)?,
        _ => return None,
    };
    Some(text.into_bytes())
}

/// The control code for Ctrl+letter, for example Ctrl+C is 0x03.
fn ctrl_letter(key: Key) -> Option<String> {
    let name = key.name().as_bytes();
    (name.len() == 1 && name[0].is_ascii_uppercase()).then(|| char::from(name[0] - b'A' + 1).to_string())
}

/// The colours of the terminal: the default foreground and background, and the 16 named colours.
#[derive(Clone, Copy)]
struct Palette {
    fg: Color32,
    bg: Color32,
    selection: Color32,
    ansi: [Color32; 16],
}

const fn hex(value: u32) -> Color32 {
    Color32::from_rgb((value >> 16) as u8, (value >> 8) as u8, value as u8)
}

const DARK_ANSI: [Color32; 16] = [
    hex(0x3b4252),
    hex(0xf07a84),
    hex(0x8fd18a),
    hex(0xf0cf7a),
    hex(0x7aa8ff),
    hex(0xcf9bf0),
    hex(0x6fd6d6),
    hex(0xd8dee9),
    hex(0x6b7385),
    hex(0xff9aa3),
    hex(0xa8e8a6),
    hex(0xffe69a),
    hex(0x9bbfff),
    hex(0xe0b6ff),
    hex(0x9be9e9),
    hex(0xf5f7fa),
];

/// Darker tones, so the named colours stay readable on a light background.
const LIGHT_ANSI: [Color32; 16] = [
    hex(0x2e3440),
    hex(0xc4313f),
    hex(0x17803f),
    hex(0x8a6200),
    hex(0x2a63c9),
    hex(0x8238b5),
    hex(0x0f7c86),
    hex(0x6b7280),
    hex(0x4b5563),
    hex(0xdc3c4e),
    hex(0x1f9a55),
    hex(0xa86f00),
    hex(0x3b78e0),
    hex(0x9c4fd0),
    hex(0x14909b),
    hex(0x111827),
];

impl Default for Palette {
    fn default() -> Self {
        Self { fg: hex(0xe4e7ee), bg: hex(0x101217), selection: hex(0x1f4f3a), ansi: DARK_ANSI }
    }
}

impl Palette {
    fn from_visuals(visuals: &egui::Visuals) -> Self {
        Self {
            fg: visuals.text_color(),
            bg: visuals.extreme_bg_color,
            selection: visuals.selection.bg_fill,
            ansi: if visuals.dark_mode { DARK_ANSI } else { LIGHT_ANSI },
        }
    }

    /// The colour of palette slot `index`: 0 to 255 are the indexed colours, and the slots above
    /// them are alacritty's named colours (foreground 256, background 257, cursor 258, dim
    /// colours 259 to 266, bright foreground 267, dim foreground 268).
    fn slot(&self, index: usize) -> Color32 {
        match index {
            0..=15 => self.ansi[index],
            16..=255 => xterm_256(index),
            256 | 258 | 267 => self.fg,
            257 => self.bg,
            259..=266 => blend(self.ansi[index - 259], self.bg, 0.35),
            268 => blend(self.fg, self.bg, 0.35),
            _ => self.fg,
        }
    }

    fn rgb_at(&self, index: usize) -> Rgb {
        let color = self.slot(index);
        Rgb { r: color.r(), g: color.g(), b: color.b() }
    }
}

/// The xterm 256-colour cube and greyscale ramp.
fn xterm_256(index: usize) -> Color32 {
    if index < 232 {
        let cube = index - 16;
        let level = |v: usize| -> u8 { if v == 0 { 0 } else { (55 + 40 * v) as u8 } };
        Color32::from_rgb(level(cube / 36), level((cube / 6) % 6), level(cube % 6))
    } else {
        let grey = (8 + 10 * (index - 232)) as u8;
        Color32::from_rgb(grey, grey, grey)
    }
}

/// `a` moved `t` of the way towards `b`.
fn blend(a: Color32, b: Color32, t: f32) -> Color32 {
    let mix = |x: u8, y: u8| (f32::from(x) + (f32::from(y) - f32::from(x)) * t).round() as u8;
    Color32::from_rgb(mix(a.r(), b.r()), mix(a.g(), b.g()), mix(a.b(), b.b()))
}

fn rgb32(rgb: Rgb) -> Color32 {
    Color32::from_rgb(rgb.r, rgb.g, rgb.b)
}

/// The colour of a cell colour, honouring colours the shell set with escape sequences.
fn resolve(color: Color, colors: &Colors, palette: &Palette, bold: bool) -> Color32 {
    let index = match color {
        Color::Spec(rgb) => return rgb32(rgb),
        Color::Indexed(index) => usize::from(index),
        Color::Named(named) => named as usize,
    };
    match colors[index] {
        Some(rgb) => rgb32(rgb),
        // Bold text uses the bright variant of the basic colours, as the font has no bold face.
        None if bold && index < 8 => palette.ansi[index + 8],
        None => palette.slot(index),
    }
}

/// The text and background colours of a cell, with selection, inverse video, and cursor applied.
fn cell_colors(cell: &Cell, colors: &Colors, palette: &Palette, selected: bool, cursor: bool) -> (Color32, Option<Color32>) {
    let bold = cell.flags.contains(Flags::BOLD);
    let mut fg = resolve(cell.fg, colors, palette, bold);
    let mut bg = if cell.bg == Color::Named(NamedColor::Background) { None } else { Some(resolve(cell.bg, colors, palette, false)) };
    if cell.flags.contains(Flags::DIM) {
        fg = blend(fg, bg.unwrap_or(palette.bg), 0.5);
    }
    if cell.flags.contains(Flags::INVERSE) {
        let text = bg.unwrap_or(palette.bg);
        bg = Some(fg);
        fg = text;
    }
    if cell.flags.contains(Flags::HIDDEN) {
        fg = bg.unwrap_or(palette.bg);
    }
    if selected {
        bg = Some(palette.selection);
    }
    if cursor {
        let text = bg.unwrap_or(palette.bg);
        bg = Some(fg);
        fg = text;
    }
    (fg, bg)
}

/// The viewport row of a grid line, if it is on screen.
fn viewport_line(line: Line, display_offset: usize, lines: usize) -> Option<usize> {
    let row = line.0 + display_offset as i32;
    (row >= 0 && (row as usize) < lines).then_some(row as usize)
}

/// The grid point and side under a pointer position.
fn point_at(pos: Pos2, rect: Rect, metrics: &Metrics, size: WindowSize, display_offset: usize) -> (Point, Side) {
    let x = ((pos.x - rect.min.x) / metrics.cw).max(0.0);
    let y = ((pos.y - rect.min.y) / metrics.ch).max(0.0);
    let col = (x as usize).min(usize::from(size.num_cols).saturating_sub(1));
    let row = (y as usize).min(usize::from(size.num_lines).saturating_sub(1));
    let side = if x.fract() < 0.5 { Side::Left } else { Side::Right };
    (Point::new(Line(row as i32 - display_offset as i32), Column(col)), side)
}

/// Text sharing one style, drawn starting at `col` on `line`.
struct Run {
    line: usize,
    col: usize,
    next_col: usize,
    text: String,
    look: Look,
}

#[derive(Clone, Copy, PartialEq)]
struct Look {
    fg: Color32,
    italic: bool,
    underline: bool,
    strike: bool,
}

fn flush(painter: &egui::Painter, metrics: &Metrics, origin: Pos2, run: &mut Option<Run>) {
    let Some(run) = run.take() else { return };
    let decorated = run.look.underline || run.look.strike;
    if !decorated && run.text.trim().is_empty() {
        return;
    }
    let line_stroke = Stroke::new(1.0, run.look.fg);
    let job = LayoutJob::single_section(
        run.text,
        TextFormat {
            font_id: metrics.font.clone(),
            color: run.look.fg,
            italics: run.look.italic,
            underline: if run.look.underline { line_stroke } else { Stroke::NONE },
            strikethrough: if run.look.strike { line_stroke } else { Stroke::NONE },
            ..Default::default()
        },
    );
    let galley = painter.layout_job(job);
    let pos = origin + egui::vec2(run.col as f32 * metrics.cw, run.line as f32 * metrics.ch);
    painter.galley(pos, galley, run.look.fg);
}

/// Draws the visible grid, then the cursor.
fn paint(ui: &egui::Ui, rect: Rect, metrics: &Metrics, palette: &Palette, focused: bool, lines: usize, content: RenderableContent<'_>) {
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, palette.bg);

    let RenderableContent { display_iter, selection, cursor, display_offset, colors, .. } = content;
    let origin = rect.min;
    let cell_rect = |line: usize, col: usize, span: usize| {
        Rect::from_min_size(
            origin + egui::vec2(col as f32 * metrics.cw, line as f32 * metrics.ch),
            egui::vec2(span as f32 * metrics.cw, metrics.ch),
        )
    };
    let cursor_at = if cursor.shape == CursorShape::Hidden {
        None
    } else {
        viewport_line(cursor.point.line, display_offset, lines).map(|line| (line, cursor.point.column.0))
    };

    let mut run: Option<Run> = None;
    for indexed in display_iter {
        let Some(line) = viewport_line(indexed.point.line, display_offset, lines) else { continue };
        let col = indexed.point.column.0;
        let cell = indexed.cell;
        let wide = cell.flags.contains(Flags::WIDE_CHAR);
        let selected = selection.as_ref().is_some_and(|range| range.contains(indexed.point));
        let on_cursor = focused && cursor.shape == CursorShape::Block && cursor_at == Some((line, col));

        let (fg, bg) = cell_colors(cell, colors, palette, selected, on_cursor);
        if let Some(bg) = bg {
            painter.rect_filled(cell_rect(line, col, if wide { 2 } else { 1 }), 0.0, bg);
        }
        // The second cell of a wide character is part of the character drawn in the first.
        if cell.flags.contains(Flags::WIDE_CHAR_SPACER) {
            flush(&painter, metrics, origin, &mut run);
            continue;
        }

        let decorated = cell.flags.intersects(Flags::ALL_UNDERLINES | Flags::STRIKEOUT);
        let blank = cell.c == ' ' && cell.zerowidth().is_none() && !decorated;
        if blank || cell.flags.contains(Flags::HIDDEN) {
            flush(&painter, metrics, origin, &mut run);
            continue;
        }

        let look = Look {
            fg,
            italic: cell.flags.contains(Flags::ITALIC),
            underline: cell.flags.intersects(Flags::ALL_UNDERLINES),
            strike: cell.flags.contains(Flags::STRIKEOUT),
        };
        let mut text = String::new();
        text.push(cell.c);
        for mark in cell.zerowidth().into_iter().flatten() {
            text.push(*mark);
        }

        let joins = !wide && run.as_ref().is_some_and(|r| r.line == line && r.next_col == col && r.look == look);
        if joins {
            if let Some(r) = run.as_mut() {
                r.text.push_str(&text);
                r.next_col += 1;
            }
        } else {
            flush(&painter, metrics, origin, &mut run);
            let span = if wide { 2 } else { 1 };
            run = Some(Run { line, col, next_col: col + span, text, look });
        }
        if wide {
            flush(&painter, metrics, origin, &mut run);
        }
    }
    flush(&painter, metrics, origin, &mut run);

    if let Some((line, col)) = cursor_at {
        let cursor_rect = cell_rect(line, col, 1);
        let color = palette.fg;
        match cursor.shape {
            CursorShape::Block if !focused => {
                painter.rect_stroke(cursor_rect, 0.0, Stroke::new(1.0, color), egui::StrokeKind::Inside);
            }
            CursorShape::HollowBlock => {
                painter.rect_stroke(cursor_rect, 0.0, Stroke::new(1.0, color), egui::StrokeKind::Inside);
            }
            CursorShape::Underline => {
                let bar = Rect::from_min_max(Pos2::new(cursor_rect.min.x, cursor_rect.max.y - 2.0), cursor_rect.max);
                painter.rect_filled(bar, 0.0, color);
            }
            CursorShape::Beam => {
                let bar = Rect::from_min_size(cursor_rect.min, egui::vec2(2.0, cursor_rect.height()));
                painter.rect_filled(bar, 0.0, color);
            }
            _ => {}
        }
    }
}

/// Starts a shell on a new pseudo-terminal, trying each candidate in turn.
fn spawn_pty(cwd: &Path, size: WindowSize) -> io::Result<tty::Pty> {
    let env = HashMap::from([("TERM".to_string(), "xterm-256color".to_string()), ("COLORTERM".to_string(), "truecolor".to_string())]);
    let mut error = None;
    for (program, args) in shell_candidates() {
        // `..Default::default()` is only needed on Windows, where `Options` has `escape_args`.
        #[allow(clippy::needless_update)]
        let options = tty::Options {
            shell: Some(Shell::new(program, args)),
            working_directory: Some(cwd.to_path_buf()),
            drain_on_exit: true,
            env: env.clone(),
            ..Default::default()
        };
        match tty::new(&options, size, 0) {
            Ok(pty) => return Ok(pty),
            Err(err) => error = Some(err),
        }
    }
    Err(error.unwrap_or_else(|| io::Error::other("no shell could be started")))
}

/// The shells to try, with their arguments.
#[cfg(unix)]
fn shell_candidates() -> Vec<(String, Vec<String>)> {
    let fallback = if cfg!(target_os = "macos") { "/bin/zsh" } else { "/bin/bash" };
    let mut programs = Vec::new();
    if let Ok(shell) = std::env::var("SHELL") {
        if !shell.is_empty() {
            programs.push(shell);
        }
    }
    if !programs.iter().any(|program| program == fallback) {
        programs.push(fallback.to_string());
    }
    // `-l` starts a login shell, so the user's profile is read as in a Terminal window.
    programs.into_iter().map(|program| (program, vec!["-l".to_string()])).collect()
}

#[cfg(windows)]
fn shell_candidates() -> Vec<(String, Vec<String>)> {
    vec![("powershell.exe".to_string(), vec!["-NoLogo".to_string()]), ("cmd.exe".to_string(), Vec::new())]
}

/// The shell sessions of one repository, shared by every terminal window opened for it.
struct Group {
    cwd: PathBuf,
    tabs: Vec<TerminalSession>,
    active: usize,
    error: Option<String>,
}

static REGISTRY: Mutex<Vec<Group>> = Mutex::new(Vec::new());

fn registry() -> MutexGuard<'static, Vec<Group>> {
    lock(&REGISTRY)
}

fn group_for<'a>(groups: &'a mut Vec<Group>, cwd: &Path) -> &'a mut Group {
    let index = match groups.iter().position(|group| group.cwd.as_path() == cwd) {
        Some(index) => index,
        None => {
            groups.push(Group { cwd: cwd.to_path_buf(), tabs: Vec::new(), active: 0, error: None });
            groups.len() - 1
        }
    };
    &mut groups[index]
}

fn add_tab(group: &mut Group, ctx: &egui::Context) {
    match TerminalSession::new(ctx, &group.cwd) {
        Ok(session) => {
            group.tabs.push(session);
            group.active = group.tabs.len() - 1;
            group.error = None;
        }
        Err(err) => group.error = Some(format!("Could not start a shell: {err}")),
    }
}

enum TabAction {
    Select(usize),
    Close(usize),
    Add,
}

/// The terminal window. It shows the shells of the active repository.
pub struct TerminalWindow {
    cwd: PathBuf,
    closing: bool,
}

impl TerminalWindow {
    /// Opens the terminal for `cwd`, reattaching to its shells if they are still running.
    pub fn new(ctx: &egui::Context, cwd: PathBuf) -> Self {
        let mut registry = registry();
        let group = group_for(&mut registry, &cwd);
        if group.tabs.is_empty() {
            add_tab(group, ctx);
        }
        Self { cwd, closing: false }
    }

    pub fn cwd(&self) -> &Path {
        &self.cwd
    }

    /// Closes a tab. Closing a tab other than the last one ends its shell. The last tab is
    /// hidden with its shell still running, unless the shell has already exited.
    fn close_tab(&mut self, group: &mut Group, index: usize) {
        if group.tabs.len() > 1 {
            group.tabs.remove(index);
            if index < group.active {
                group.active -= 1;
            }
        } else {
            if group.tabs.first().is_some_and(TerminalSession::is_exited) {
                group.tabs.clear();
            }
            self.closing = true;
        }
        group.active = group.active.min(group.tabs.len().saturating_sub(1));
    }
}

impl ToolWindow for TerminalWindow {
    fn id(&self) -> String {
        "terminal".to_string()
    }

    fn title(&self) -> String {
        "Terminal".to_string()
    }

    fn default_size(&self) -> egui::Vec2 {
        egui::vec2(820.0, 420.0)
    }

    fn ui(&mut self, ui: &mut egui::Ui, cx: &mut Ctx<'_>) {
        // The terminal follows the active repository.
        if self.cwd.as_path() != cx.repo {
            self.cwd = cx.repo.to_path_buf();
        }
        let ctx = ui.ctx().clone();
        let mut registry = registry();
        let group = group_for(&mut registry, &self.cwd);
        if group.tabs.is_empty() {
            add_tab(group, &ctx);
        }

        let mut actions = Vec::new();
        ui.horizontal(|ui| {
            for (index, session) in group.tabs.iter().enumerate() {
                if ui.selectable_label(index == group.active, session.title()).clicked() {
                    actions.push(TabAction::Select(index));
                }
                if widgets::icon_button(ui, icon::X, "Close tab", true).clicked() {
                    actions.push(TabAction::Close(index));
                }
            }
            if widgets::icon_button(ui, icon::PLUS, "New shell tab", true).clicked() {
                actions.push(TabAction::Add);
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(RichText::new(group.cwd.display().to_string()).small().color(theme::of(ui).muted));
            });
        });
        for action in actions {
            match action {
                TabAction::Select(index) => group.active = index,
                TabAction::Add => add_tab(group, &ctx),
                TabAction::Close(index) if index < group.tabs.len() => self.close_tab(group, index),
                TabAction::Close(_) => {}
            }
        }

        if let Some(error) = &group.error {
            widgets::error(ui, error);
        }
        match group.tabs.get_mut(group.active) {
            Some(session) => session.ui(ui),
            None => widgets::empty_state(ui, icon::TERMINAL, "No shell open"),
        }
    }

    fn wants_close(&self) -> bool {
        self.closing
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    impl TerminalSession {
        fn input(&self, bytes: Vec<u8>) {
            self.notifier.notify(bytes);
        }
    }

    #[test]
    fn shell_runs_typed_commands_and_shows_their_output() {
        let ctx = egui::Context::default();
        let session = TerminalSession::new(&ctx, &std::env::temp_dir()).expect("start a shell");
        // The typed line contains "$((6*7))", so only the output can contain "nicegit-42".
        session.input(b"echo nicegit-$((6*7))\r".to_vec());

        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let text = session.screen_text();
            if text.contains("nicegit-42") {
                return;
            }
            assert!(Instant::now() < deadline, "the output never appeared; the screen was:\n{text}");
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// Runs real egui frames: a click focuses the terminal, typed text reaches the shell, the
    /// output is painted, and an exited shell shows its banner.
    #[test]
    fn ui_frames_take_keyboard_input_and_show_exit() {
        let ctx = egui::Context::default();
        let mut session = TerminalSession::new(&ctx, &std::env::temp_dir()).expect("start a shell");
        let screen = Rect::from_min_size(Pos2::ZERO, egui::vec2(800.0, 400.0));
        let pos = Pos2::new(100.0, 100.0);
        let press = |pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
        let key = |key| egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE };
        let frame = |session: &mut TerminalSession, events: Vec<egui::Event>| {
            let input = egui::RawInput { screen_rect: Some(screen), events, ..Default::default() };
            let mut output = ctx.run_ui(input, |ui| session.ui(ui));
            // No renderer uploads the font atlas here, so discard the texture changes.
            output.textures_delta.clear();
        };

        frame(&mut session, vec![]);
        frame(&mut session, vec![egui::Event::PointerMoved(pos), press(true)]);
        frame(&mut session, vec![press(false)]);
        frame(&mut session, vec![]);
        frame(&mut session, vec![egui::Event::Text("echo ui-$((5*5))".to_string()), key(Key::Enter)]);

        let deadline = Instant::now() + Duration::from_secs(5);
        while !session.screen_text().contains("ui-25") {
            assert!(Instant::now() < deadline, "typed input never reached the shell:\n{}", session.screen_text());
            frame(&mut session, vec![]);
            std::thread::sleep(Duration::from_millis(50));
        }

        frame(&mut session, vec![egui::Event::Text("exit".to_string()), key(Key::Enter)]);
        let deadline = Instant::now() + Duration::from_secs(5);
        while !session.is_exited() {
            assert!(Instant::now() < deadline, "the shell did not exit");
            frame(&mut session, vec![]);
            std::thread::sleep(Duration::from_millis(50));
        }
        frame(&mut session, vec![]);
        assert!(session.title().ends_with("(exited)"));
    }

    /// Hiding the last running tab and reopening the window reattaches to the same shell.
    #[test]
    fn reopened_window_reattaches_to_the_running_shell() {
        let ctx = egui::Context::default();
        let dir = std::env::temp_dir().join("nicegit-terminal-registry-test");
        std::fs::create_dir_all(&dir).expect("create the test directory");

        let mut window = TerminalWindow::new(&ctx, dir.clone());
        {
            let mut registry = registry();
            group_for(&mut registry, &dir).tabs[0].input(b"echo attached-$((6*7))\r".to_vec());
        }
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let text = group_for(&mut registry(), &dir).tabs[0].screen_text();
            if text.contains("attached-42") {
                break;
            }
            assert!(Instant::now() < deadline, "the output never appeared:\n{text}");
            std::thread::sleep(Duration::from_millis(50));
        }

        // The shell is running, so closing its tab hides the window but keeps the session.
        {
            let mut registry = registry();
            window.close_tab(group_for(&mut registry, &dir), 0);
        }
        assert!(window.wants_close());
        drop(window);

        let window = TerminalWindow::new(&ctx, dir.clone());
        let mut registry = registry();
        let group = group_for(&mut registry, &dir);
        assert_eq!(group.tabs.len(), 1);
        assert!(group.tabs[0].screen_text().contains("attached-42"));
        drop(registry);
        drop(window);
    }
}
