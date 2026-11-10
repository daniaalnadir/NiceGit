//! Syntax colours for the code lines of a diff, from the language of each file.
//!
//! A hunk's code is read as two streams, its old side and its new side, in order, so a string or
//! comment that runs across several lines keeps its colour within the hunk. Each hunk starts from
//! a fresh parser state, since the lines between hunks are not in the diff.
//!
//! Colours are computed when a hunk is first drawn and cached by file path, the hunk's text and
//! the theme. Highlighting costs about a tenth of a millisecond a line, so a hunk longer than
//! [`SYNC_HUNK_LINES`], or a file longer than [`BACKGROUND_LINES`], is coloured on a background
//! thread, and its lines stay plain until that finishes.

use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::ops::Range;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock, Mutex, MutexGuard};

use egui::Color32;
use nicegit_core::diff::{DiffLine, DiffLineKind};
use syntect::easy::HighlightLines;
use syntect::highlighting::{Style, Theme, ThemeSet};
use syntect::parsing::{SyntaxReference, SyntaxSet};

/// A coloured run of a line's text: its byte range and its text colour.
pub type Span = (Range<usize>, Color32);

/// The spans of each line of one hunk, in the hunk's order.
pub type HunkSpans = Vec<Vec<Span>>;

/// Diffs with more lines than this stay plain.
pub const MAX_LINES: usize = 10_000;
/// A file whose diff text is larger than this many bytes stays plain.
pub const MAX_FILE_BYTES: usize = 1_000_000;
/// A file with more lines than this is coloured on a background thread.
pub const BACKGROUND_LINES: usize = 2_000;
/// A hunk with more lines than this is coloured on a background thread, so the first draw of a
/// long hunk does not stall the window (about 20 ms of work at this size).
pub const SYNC_HUNK_LINES: usize = 200;
/// How many hunks' colours are kept; the cache starts again once it is full.
const CACHE_LIMIT: usize = 256;

static ENABLED: AtomicBool = AtomicBool::new(true);

/// Turns syntax colours on or off, as Settings does. Off, every line is drawn in plain text.
pub fn set_enabled(enabled: bool) {
    ENABLED.store(enabled, Ordering::Relaxed);
}

pub fn enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

static SYNTAXES: LazyLock<SyntaxSet> = LazyLock::new(SyntaxSet::load_defaults_newlines);
static THEMES: LazyLock<ThemeSet> = LazyLock::new(ThemeSet::load_defaults);

/// The theme for the appearance: InspiredGitHub on a light background, base16-ocean on a dark one.
fn theme(dark: bool) -> &'static Theme {
    let name = if dark { "base16-ocean.dark" } else { "InspiredGitHub" };
    &THEMES.themes[name]
}

/// The language of a file, from its extension. Without a known extension, the file's first line
/// decides, as with a shebang such as `#!/usr/bin/env python3`. `None` means plain text.
pub fn language(path: Option<&str>, first_line: Option<&str>) -> Option<&'static SyntaxReference> {
    let by_extension = path.and_then(|path| {
        // Git quotes a path with unusual characters, so a closing quote is not part of the name.
        let name = path.rsplit('/').next()?.trim_end_matches('"');
        let (stem, extension) = name.rsplit_once('.')?;
        if stem.is_empty() || extension.is_empty() {
            return None;
        }
        SYNTAXES.find_syntax_by_extension(extension).or_else(|| SYNTAXES.find_syntax_by_extension(&extension.to_ascii_lowercase()))
    });
    by_extension
        .or_else(|| first_line.and_then(|line| SYNTAXES.find_syntax_by_first_line(line)))
        .filter(|syntax| syntax.name != "Plain Text")
}

/// The code of a diff line, without its one-byte sign (' ', '+' or '-').
fn code(text: &str) -> &str {
    text.get(1..).unwrap_or("")
}

/// Colours each line of one stream of code, reading them in order. Each span covers the line's
/// text in bytes, without the line break, and the spans together cover the whole line.
fn highlight_stream(texts: &[&str], syntax: &SyntaxReference, dark: bool) -> Vec<Vec<Span>> {
    let mut highlighter = HighlightLines::new(syntax, theme(dark));
    texts
        .iter()
        .map(|text| {
            let line = format!("{text}\n");
            match highlighter.highlight_line(&line, &SYNTAXES) {
                Ok(tokens) => spans_of(&tokens, text.len()),
                // The parser gave up on this line; it stays plain.
                Err(_) => Vec::new(),
            }
        })
        .collect()
}

/// Turns highlighted tokens into spans over the first `len` bytes, merging neighbours of one colour.
fn spans_of(tokens: &[(Style, &str)], len: usize) -> Vec<Span> {
    let mut spans: Vec<Span> = Vec::new();
    let mut offset = 0;
    for (style, token) in tokens {
        let end = (offset + token.len()).min(len);
        if offset < end {
            let foreground = style.foreground;
            let color = Color32::from_rgb(foreground.r, foreground.g, foreground.b);
            match spans.last_mut() {
                Some((range, last)) if *last == color && range.end == offset => range.end = end,
                _ => spans.push((offset..end, color)),
            }
        }
        offset += token.len();
    }
    spans
}

/// The colours of one hunk's lines, aligned with `lines`. Context lines take the new side's
/// colours; lines that are not code (headers, notes) get none. Each span is offset past the sign.
pub fn highlight_hunk(lines: &[DiffLine], syntax: &SyntaxReference, dark: bool) -> HunkSpans {
    let mut result: HunkSpans = vec![Vec::new(); lines.len()];
    for take_new in [false, true] {
        let indices: Vec<usize> = (0..lines.len())
            .filter(|&index| match lines[index].kind {
                DiffLineKind::Context => true,
                DiffLineKind::Deletion => !take_new,
                DiffLineKind::Addition => take_new,
                _ => false,
            })
            .collect();
        let texts: Vec<&str> = indices.iter().map(|&index| code(&lines[index].text)).collect();
        let streams = highlight_stream(&texts, syntax, dark);
        for (&index, spans) in indices.iter().zip(streams) {
            // Context is in both streams; only the new side's colours are kept for it.
            if lines[index].kind == DiffLineKind::Context && !take_new {
                continue;
            }
            // The sign before the code is one byte.
            result[index] = spans.into_iter().map(|(range, color)| (range.start + 1..range.end + 1, color)).collect();
        }
    }
    result
}

/// What a hunk's colours depend on: its file, its lines and the theme.
fn key(path: Option<&str>, lines: &[DiffLine], dark: bool) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    path.hash(&mut hasher);
    dark.hash(&mut hasher);
    for line in lines {
        std::mem::discriminant(&line.kind).hash(&mut hasher);
        line.text.hash(&mut hasher);
    }
    hasher.finish()
}

#[derive(Default)]
struct Cache {
    ready: HashMap<u64, Arc<HunkSpans>>,
    /// Hunks being coloured on a background thread.
    pending: HashSet<u64>,
}

static CACHE: LazyLock<Mutex<Cache>> = LazyLock::new(Mutex::default);

fn cache() -> MutexGuard<'static, Cache> {
    CACHE.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn ready(key: u64) -> Option<Arc<HunkSpans>> {
    cache().ready.get(&key).cloned()
}

/// Marks a hunk as being coloured, unless it is already ready or being coloured. Returns whether
/// the caller should start the work.
fn begin(key: u64) -> bool {
    let mut cache = cache();
    if cache.ready.contains_key(&key) {
        return false;
    }
    cache.pending.insert(key)
}

fn finish(key: u64, spans: Arc<HunkSpans>) {
    let mut cache = cache();
    cache.pending.remove(&key);
    if cache.ready.len() >= CACHE_LIMIT {
        cache.ready.clear();
    }
    cache.ready.insert(key, spans);
}

/// One hunk of a diff, and how it is coloured.
struct HunkPlan {
    /// The hunk's lines, from its `@@` header through its last line.
    range: Range<usize>,
    syntax: &'static SyntaxReference,
    /// The cache key for the light theme and for the dark theme.
    keys: [u64; 2],
    /// Whether the hunk is long enough to colour on a background thread.
    background: bool,
}

/// Which hunks of a diff are coloured, and by what language. Working this out is cheap; the
/// colours themselves are computed when a line is first drawn.
#[derive(Default)]
pub struct Plan {
    hunks: Vec<HunkPlan>,
    /// The hunk each line belongs to, when it is in a coloured hunk.
    hunk_of: Vec<Option<usize>>,
}

impl Plan {
    pub fn new(lines: &[DiffLine]) -> Self {
        let mut plan = Plan { hunks: Vec::new(), hunk_of: vec![None; lines.len()] };
        if lines.len() > MAX_LINES {
            return plan;
        }
        // A file starts at each `diff --git` line; a patch without one is a single file.
        let mut starts: Vec<usize> = vec![0];
        for (index, line) in lines.iter().enumerate().skip(1) {
            if line.kind == DiffLineKind::Metadata && line.text.starts_with("diff --git ") {
                starts.push(index);
            }
        }
        for (position, &start) in starts.iter().enumerate() {
            let end = starts.get(position + 1).copied().unwrap_or(lines.len());
            plan.add_file(lines, start..end);
        }
        plan
    }

    fn add_file(&mut self, lines: &[DiffLine], file: Range<usize>) {
        let body = &lines[file.clone()];
        let bytes: usize = body.iter().map(|line| line.text.len()).sum();
        if bytes > MAX_FILE_BYTES {
            return;
        }
        let path = path_of(body);
        // The first line of the file, when this diff shows it, decides the language of a file
        // without an extension.
        let first = body
            .iter()
            .find(|line| {
                matches!(line.kind, DiffLineKind::Context | DiffLineKind::Addition | DiffLineKind::Deletion)
                    && (line.old_number == Some(1) || line.new_number == Some(1))
            })
            .map(|line| code(&line.text));
        let Some(syntax) = language(path, first) else { return };
        let background = body.len() > BACKGROUND_LINES;
        let hunk_starts: Vec<usize> = file.clone().filter(|&index| lines[index].kind == DiffLineKind::Hunk).collect();
        for (position, &start) in hunk_starts.iter().enumerate() {
            let end = hunk_starts.get(position + 1).copied().unwrap_or(file.end);
            let hunk = start..end;
            let keys = [key(path, &lines[hunk.clone()], false), key(path, &lines[hunk.clone()], true)];
            let index = self.hunks.len();
            for slot in self.hunk_of[hunk.clone()].iter_mut() {
                *slot = Some(index);
            }
            let background = background || hunk.len() > SYNC_HUNK_LINES;
            self.hunks.push(HunkPlan { range: hunk, syntax, keys, background });
        }
    }

    /// How many hunks are coloured at all, for tests.
    #[cfg(test)]
    pub fn coloured_hunks(&self) -> usize {
        self.hunks.len()
    }

    /// The colours of a line: its hunk's spans and the line's position among them. `None` while
    /// they are not ready (a background colouring is then started), or when the line is plain.
    pub fn line_spans(&self, ctx: &egui::Context, lines: &[DiffLine], line: usize, dark: bool) -> Option<(Arc<HunkSpans>, usize)> {
        if !enabled() {
            return None;
        }
        let hunk = &self.hunks[self.hunk_of.get(line).copied().flatten()?];
        let key = hunk.keys[usize::from(dark)];
        let spans = match ready(key) {
            Some(spans) => spans,
            None if hunk.background => {
                if begin(key) {
                    let owned = lines[hunk.range.clone()].to_vec();
                    let (syntax, ctx) = (hunk.syntax, ctx.clone());
                    std::thread::spawn(move || {
                        finish(key, Arc::new(highlight_hunk(&owned, syntax, dark)));
                        ctx.request_repaint();
                    });
                }
                return None;
            }
            None => {
                let spans = Arc::new(highlight_hunk(&lines[hunk.range.clone()], hunk.syntax, dark));
                finish(key, spans.clone());
                spans
            }
        };
        Some((spans, line - hunk.range.start))
    }
}

/// The file's path, from its `+++` header, or from its `---` header when the file was deleted.
/// Only the headers before the first hunk count, since a line of code can look like one.
fn path_of(body: &[DiffLine]) -> Option<&str> {
    let header = |prefix: &str| {
        body.iter()
            .take_while(|line| line.kind != DiffLineKind::Hunk)
            .find_map(|line| line.text.strip_prefix(prefix))
            .filter(|path| *path != "/dev/null")
    };
    header("+++ ").or_else(|| header("--- "))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(text: &str, kind: DiffLineKind, old: Option<usize>, new: Option<usize>) -> DiffLine {
        DiffLine { text: text.into(), kind, old_number: old, new_number: new }
    }

    #[test]
    fn languages_come_from_extensions_then_shebangs() {
        let name = |path: Option<&str>, first: Option<&str>| language(path, first).map(|syntax| syntax.name.clone());
        assert_eq!(name(Some("src/main.rs"), None).as_deref(), Some("Rust"));
        assert_eq!(name(Some("tool.py"), None).as_deref(), Some("Python"));
        assert_eq!(name(Some("NOTES.RS"), None).as_deref(), Some("Rust"), "extensions match regardless of case");
        assert_eq!(name(Some("\"quoted name.rs\""), None).as_deref(), Some("Rust"), "a quoted Git path ends at its quote");
        assert_eq!(name(Some("run"), Some("#!/usr/bin/env python3")).as_deref(), Some("Python"));
        assert!(name(Some("run.unknownext"), Some("#!/bin/sh")).is_some(), "an unknown extension falls back to the shebang");
        assert_eq!(name(Some("notes.unknownext"), Some("just text")), None);
        assert_eq!(name(Some(".rs"), None), None, "a name that is only an extension is not a language");
        assert_eq!(name(None, None), None);
    }

    #[test]
    fn colour_spans_line_up_with_the_original_text() {
        let rust = language(Some("lib.rs"), None).expect("Rust is built in");
        let texts = ["fn main() {", "    let text = \"héllo\"; // ü", "}"];
        for dark in [false, true] {
            let streams = highlight_stream(&texts, rust, dark);
            for (text, spans) in texts.iter().zip(&streams) {
                // Spans are in order, cover the text exactly, and start and end on character boundaries.
                let mut position = 0;
                let mut joined = String::new();
                for (range, _) in spans {
                    assert_eq!(range.start, position, "{text:?} has a gap or overlap");
                    assert!(text.is_char_boundary(range.start) && text.is_char_boundary(range.end));
                    joined.push_str(&text[range.clone()]);
                    position = range.end;
                }
                assert_eq!(joined, *text, "concatenated spans equal the line");
            }
            assert!(streams[1].len() > 2, "a line with a string and a comment has several colours");
        }
    }

    #[test]
    fn hunk_colours_follow_each_side_of_the_change() {
        let rust = language(Some("lib.rs"), None).expect("Rust is built in");
        let lines = vec![
            line("@@ -1,2 +1,2 @@", DiffLineKind::Hunk, None, None),
            line(" fn a() {", DiffLineKind::Context, Some(1), Some(1)),
            line("-    let x = 1;", DiffLineKind::Deletion, Some(2), None),
            line("+    let x = \"one\";", DiffLineKind::Addition, None, Some(2)),
            line(" }", DiffLineKind::Context, Some(3), Some(3)),
            line("\\ No newline at end of file", DiffLineKind::Metadata, None, None),
        ];
        let spans = highlight_hunk(&lines, rust, false);
        assert!(spans[0].is_empty(), "a hunk header is not code");
        assert!(spans[5].is_empty(), "a note is not code");
        for index in [1, 2, 3, 4] {
            let text = &lines[index].text;
            let joined: String = spans[index].iter().map(|(range, _)| &text[range.clone()]).collect();
            assert_eq!(joined, &text[1..], "line {index} keeps its code, after its sign");
        }
        let added = &spans[3];
        let colours: HashSet<[u8; 3]> = added.iter().map(|(_, c)| c.to_array()[..3].try_into().unwrap()).collect();
        assert!(colours.len() > 1, "a string literal is a different colour from the code around it");
    }

    #[test]
    fn plan_colours_each_file_of_a_diff_by_its_own_language() {
        let lines = vec![
            line("diff --git a/a.rs b/a.rs", DiffLineKind::Metadata, None, None),
            line("--- a/a.rs", DiffLineKind::Metadata, None, None),
            line("+++ b/a.rs", DiffLineKind::Metadata, None, None),
            line("@@ -1 +1 @@", DiffLineKind::Hunk, None, None),
            line("-let a = 1;", DiffLineKind::Deletion, Some(1), None),
            line("+let a = 2;", DiffLineKind::Addition, None, Some(1)),
            line("diff --git a/notes.txt b/notes.txt", DiffLineKind::Metadata, None, None),
            line("+++ b/notes.txt", DiffLineKind::Metadata, None, None),
            line("@@ -0,0 +1 @@", DiffLineKind::Hunk, None, None),
            line("+plain words", DiffLineKind::Addition, None, Some(1)),
        ];
        let plan = Plan::new(&lines);
        assert_eq!(plan.hunks.len(), 1, "the plain-text file has no coloured hunk");
        assert_eq!(plan.hunks[0].syntax.name, "Rust");
        assert_eq!(plan.hunk_of[4], Some(0));
        assert_eq!(plan.hunk_of[1], None, "headers are outside every hunk");
        assert_eq!(plan.hunk_of[9], None, "a plain-text file has no colours");
    }

    #[test]
    fn a_deleted_file_is_named_by_its_old_path() {
        let body = vec![
            line("diff --git a/gone.py b/gone.py", DiffLineKind::Metadata, None, None),
            line("--- a/gone.py", DiffLineKind::Metadata, None, None),
            line("+++ /dev/null", DiffLineKind::Metadata, None, None),
            line("@@ -1 +0,0 @@", DiffLineKind::Hunk, None, None),
            line("+++ b/inside_the_code.rs", DiffLineKind::Deletion, Some(1), None),
        ];
        assert_eq!(path_of(&body), Some("a/gone.py"));
    }

    #[test]
    fn long_diffs_and_files_stay_plain() {
        let many: Vec<DiffLine> = (0..MAX_LINES + 1).map(|_| line("+x", DiffLineKind::Addition, None, Some(1))).collect();
        let mut lines = vec![
            line("diff --git a/a.rs b/a.rs", DiffLineKind::Metadata, None, None),
            line("+++ b/a.rs", DiffLineKind::Metadata, None, None),
            line("@@ -0,0 +1 @@", DiffLineKind::Hunk, None, None),
        ];
        lines.extend(many);
        assert!(Plan::new(&lines).hunks.is_empty());
    }

    #[test]
    fn long_hunks_are_coloured_in_the_background_until_ready() {
        let count = SYNC_HUNK_LINES + 50;
        let mut lines = vec![
            line("diff --git a/background.rs b/background.rs", DiffLineKind::Metadata, None, None),
            line("+++ b/background.rs", DiffLineKind::Metadata, None, None),
            line(&format!("@@ -0,0 +1,{count} @@"), DiffLineKind::Hunk, None, None),
        ];
        for index in 0..count {
            lines.push(line(&format!("+let long_{index} = \"{index}\";"), DiffLineKind::Addition, None, Some(index + 1)));
        }
        let ctx = egui::Context::default();
        let plan = Plan::new(&lines);
        assert!(plan.line_spans(&ctx, &lines, 3, false).is_none(), "the hunk is plain until its colours are ready");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        let found = loop {
            if let Some(found) = plan.line_spans(&ctx, &lines, 3, false) {
                break found;
            }
            assert!(std::time::Instant::now() < deadline, "the background colouring finishes");
            std::thread::sleep(std::time::Duration::from_millis(10));
        };
        // The hunk starts at its header, so the first added line is the hunk's second line.
        let (spans, index) = found;
        assert_eq!(index, 1);
        assert!(!spans[1].is_empty(), "the first added line is coloured once ready");
    }
}
