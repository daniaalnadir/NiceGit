//! Translated interface text. English is the source: every string in the interface is written
//! in English and looked up in a catalog for the active language. A key that a catalog does not
//! have shows its English text, so a missing translation never leaves a blank control.
//!
//! Catalogs are tab-separated files in `locales/`, one `English<TAB>Translation` pair per line.
//! Lines starting with `#` are comments. Fields may use `\t`, `\n` and `\\`. The test
//! `every_catalog_matches_the_strings_in_the_source` checks each catalog against the source.

use std::borrow::Cow;
#[cfg(test)]
use std::cell::Cell;
use std::collections::HashMap;
use std::path::Path;
use std::sync::OnceLock;

/// A language the interface can be shown in, once the System choice has been resolved.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Locale {
    English,
    German,
    French,
    Spanish,
    PortugueseBrazil,
    Japanese,
    ChineseSimplified,
}

/// Every locale, in the order of the catalogs' indices.
pub const LOCALES: [Locale; 7] = [
    Locale::English,
    Locale::German,
    Locale::French,
    Locale::Spanish,
    Locale::PortugueseBrazil,
    Locale::Japanese,
    Locale::ChineseSimplified,
];

impl Locale {
    /// The tag the catalog file is named after, such as `de` or `pt-BR`.
    pub fn code(self) -> &'static str {
        match self {
            Locale::English => "en",
            Locale::German => "de",
            Locale::French => "fr",
            Locale::Spanish => "es",
            Locale::PortugueseBrazil => "pt-BR",
            Locale::Japanese => "ja",
            Locale::ChineseSimplified => "zh-Hans",
        }
    }

    /// The language's name in itself. Never translated.
    pub fn native_name(self) -> &'static str {
        match self {
            Locale::English => "English",
            Locale::German => "Deutsch",
            Locale::French => "Français",
            Locale::Spanish => "Español",
            Locale::PortugueseBrazil => "Português (Brasil)",
            Locale::Japanese => "日本語",
            Locale::ChineseSimplified => "简体中文",
        }
    }

    /// The locale a system language tag selects, such as `de-DE` or `zh-CN`. Traditional Chinese
    /// has no catalog yet, so it shows English.
    pub fn from_tag(tag: &str) -> Locale {
        let tag = tag.to_ascii_lowercase().replace('_', "-");
        if tag.starts_with("zh-hant") || tag.starts_with("zh-tw") || tag.starts_with("zh-hk") || tag.starts_with("zh-mo") {
            return Locale::English;
        }
        let language = tag.split('-').next().unwrap_or_default();
        match language {
            "de" => Locale::German,
            "fr" => Locale::French,
            "es" => Locale::Spanish,
            "pt" => Locale::PortugueseBrazil,
            "ja" => Locale::Japanese,
            "zh" => Locale::ChineseSimplified,
            _ => Locale::English,
        }
    }

    /// Whether the interface needs a system font for Chinese or Japanese characters.
    pub fn needs_cjk_font(self) -> bool {
        matches!(self, Locale::Japanese | Locale::ChineseSimplified)
    }

    fn index(self) -> u8 {
        LOCALES.iter().position(|&locale| locale == self).unwrap_or(0) as u8
    }

    fn from_index(index: u8) -> Locale {
        LOCALES.get(index as usize).copied().unwrap_or(Locale::English)
    }

    /// The catalog's files, as (name, text) pairs, or none for English, which needs none. Each
    /// language is a folder of files in `locales`, one per part of the interface; `build.rs`
    /// lists them.
    fn catalog_files(self) -> &'static [(&'static str, &'static str)] {
        match self {
            Locale::English => &[],
            Locale::German => catalogs::DE,
            Locale::French => catalogs::FR,
            Locale::Spanish => catalogs::ES,
            Locale::PortugueseBrazil => catalogs::PT_BR,
            Locale::Japanese => catalogs::JA,
            Locale::ChineseSimplified => catalogs::ZH_HANS,
        }
    }
}

/// The embedded catalog files of each language, written by `build.rs`.
mod catalogs {
    include!(concat!(env!("OUT_DIR"), "/catalogs.rs"));
}

/// Translations for one locale. Keys borrow from the embedded file unless an escape needed
/// decoding, so lookups never allocate.
type Catalog = HashMap<Cow<'static, str>, String>;

/// One lazily parsed catalog per non-English locale; English has none.
static CATALOGS: [OnceLock<Catalog>; 6] = [const { OnceLock::new() }; 6];

#[cfg(not(test))]
static LOCALE: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);

#[cfg(test)]
thread_local! {
    // Tests run in parallel threads, so each test thread has its own language. It starts as
    // English.
    static LOCALE: Cell<u8> = const { Cell::new(0) };
}

/// The language the interface is shown in now.
#[cfg(not(test))]
pub fn current() -> Locale {
    Locale::from_index(LOCALE.load(std::sync::atomic::Ordering::Relaxed))
}

#[cfg(test)]
pub fn current() -> Locale {
    LOCALE.with(|locale| Locale::from_index(locale.get()))
}

/// Sets the language the interface is shown in from now on.
#[cfg(not(test))]
pub fn set_locale(locale: Locale) {
    LOCALE.store(locale.index(), std::sync::atomic::Ordering::Relaxed);
}

#[cfg(test)]
pub fn set_locale(locale: Locale) {
    LOCALE.with(|current| current.set(locale.index()));
}

/// Sets the language for the current test thread until the guard is dropped.
#[cfg(test)]
pub fn with_language<R>(locale: Locale, body: impl FnOnce() -> R) -> R {
    struct Restore(Locale);
    impl Drop for Restore {
        fn drop(&mut self) {
            set_locale(self.0);
        }
    }
    let _restore = Restore(current());
    set_locale(locale);
    body()
}

/// The English text, in the active language when a translation exists for it. Returns quickly
/// for English, so it can be called for every label on every frame.
pub fn tr(english: &'static str) -> &'static str {
    let locale = current();
    if locale == Locale::English {
        return english;
    }
    catalog(locale).get(english).map_or(english, String::as_str)
}

fn catalog(locale: Locale) -> &'static Catalog {
    CATALOGS[(locale.index() - 1) as usize].get_or_init(|| {
        let mut catalog = Catalog::new();
        for (_, source) in locale.catalog_files() {
            catalog.extend(parse_catalog(source));
        }
        catalog
    })
}

/// Parses a catalog file. Comments and lines without a tab are skipped, and so is a line with
/// no translation, which then shows English.
pub fn parse_catalog(source: &'static str) -> Catalog {
    let mut catalog = Catalog::new();
    for line in source.lines() {
        let line = line.strip_suffix('\r').unwrap_or(line);
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((english, translation)) = line.split_once('\t') else { continue };
        let translation = unescape(translation);
        if !translation.is_empty() {
            // A key without escapes is borrowed from the source, so it is not copied.
            catalog.insert(unescape(english), translation.into_owned());
        }
    }
    catalog
}

/// Decodes the `\t`, `\n` and `\\` escapes of a catalog field. Other backslashes are kept.
fn unescape(field: &str) -> Cow<'_, str> {
    if !field.contains('\\') {
        return Cow::Borrowed(field);
    }
    let mut decoded = String::with_capacity(field.len());
    let mut chars = field.chars();
    while let Some(character) = chars.next() {
        if character != '\\' {
            decoded.push(character);
            continue;
        }
        match chars.clone().next() {
            Some('t') => decoded.push('\t'),
            Some('n') => decoded.push('\n'),
            Some('\\') => decoded.push('\\'),
            _ => {
                decoded.push('\\');
                continue;
            }
        }
        chars.next();
    }
    Cow::Owned(decoded)
}

/// Replaces each `{name}` in the template with its argument, in one pass. Names with no argument
/// are left as they are.
pub fn fill(template: &str, args: &[(&str, String)]) -> String {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(start) = rest.find('{') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        let Some(end) = after.find('}') else {
            // An unclosed brace is kept, with everything after it.
            rest = &rest[start..];
            break;
        };
        let name = &after[..end];
        match args.iter().find(|(argument, _)| *argument == name) {
            Some((_, value)) => out.push_str(value),
            None => {
                out.push('{');
                out.push_str(name);
                out.push('}');
            }
        }
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    out
}

/// The English template, translated and filled with named arguments:
/// `tr_fmt!("Pushed {branch}.", branch = name)`. The template must be a string literal, so the
/// completeness test can find it.
macro_rules! tr_fmt {
    ($template:literal $(, $name:ident = $value:expr)* $(,)?) => {
        $crate::i18n::fill($crate::i18n::tr($template), &[$((stringify!($name), ($value).to_string())),*] as &[(&str, String)])
    };
}
pub(crate) use tr_fmt;

/// Whether a count uses the singular form in the locale. French treats zero as singular.
/// Nothing in the interface counts things yet, so this is only used by the tests.
#[cfg_attr(not(test), allow(dead_code))]
fn is_singular(locale: Locale, count: usize) -> bool {
    match locale {
        Locale::French => count <= 1,
        Locale::Japanese | Locale::ChineseSimplified => false,
        _ => count == 1,
    }
}

/// Picks the singular or plural text for a count and fills in `{count}`. Both forms are catalog
/// keys, so a translator can give each its own wording.
#[cfg_attr(not(test), allow(dead_code))]
pub fn tr_count(one: &'static str, other: &'static str, count: usize) -> String {
    let key = if is_singular(current(), count) { one } else { other };
    fill(tr(key), &[("count", count.to_string())])
}

/// The system fonts, tried in order, that cover the characters of a locale.
fn cjk_font_candidates(locale: Locale) -> &'static [&'static str] {
    match locale {
        Locale::Japanese => &[
            "/System/Library/Fonts/ヒラギノ角ゴシック W3.ttc",
            "C:\\Windows\\Fonts\\YuGothM.ttc",
            "C:\\Windows\\Fonts\\meiryo.ttc",
            "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
            "/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc",
            "/usr/share/fonts/google-noto-cjk/NotoSansCJK-Regular.ttc",
        ],
        Locale::ChineseSimplified => &[
            "/System/Library/Fonts/PingFang.ttc",
            "/System/Library/Fonts/Hiragino Sans GB.ttc",
            "C:\\Windows\\Fonts\\msyh.ttc",
            "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
            "/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc",
            "/usr/share/fonts/google-noto-cjk/NotoSansCJK-Regular.ttc",
        ],
        _ => &[],
    }
}

/// Reads the first system font found for the locale's characters. Blocks on the file read, so
/// call it from a background thread.
pub fn load_cjk_font(locale: Locale) -> Option<Vec<u8>> {
    cjk_font_candidates(locale).iter().filter(|path| Path::new(path).is_file()).find_map(|path| std::fs::read(path).ok())
}

/// Adds a font for the locale's characters after the built-in fonts, so Latin text keeps the
/// Inter look and the extra font is used only for characters the built-in fonts lack.
pub fn install_cjk_font(ctx: &egui::Context, locale: Locale, bytes: Vec<u8>) {
    use egui::{FontFamily, FontPriority, InsertFontFamily};
    let families = [FontFamily::Proportional, FontFamily::Monospace, crate::theme::strong_family()]
        .into_iter()
        .map(|family| InsertFontFamily { family, priority: FontPriority::Lowest })
        .collect();
    ctx.add_font(egui::FontInsert::new(&format!("cjk-{}", locale.code()), egui::FontData::from_owned(bytes), families));
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;
    use std::path::PathBuf;

    #[test]
    fn catalog_lines_decode_escapes_and_skip_comments() {
        let catalog = parse_catalog("# A comment\nHello\tBonjour\r\nTab\\there\tA\\tB\\nC\\\\D\nNo tab here\nEmpty\t\n");
        assert_eq!(catalog.get("Hello").map(String::as_str), Some("Bonjour"));
        assert_eq!(catalog.get("Tab\there").map(String::as_str), Some("A\tB\nC\\D"));
        assert!(!catalog.contains_key("# A comment"));
        assert!(!catalog.contains_key("No tab here"));
        assert!(!catalog.contains_key("Empty"));
    }

    #[test]
    fn english_returns_the_same_text_without_a_lookup() {
        with_language(Locale::English, || {
            let text = "Check for new versions";
            assert!(std::ptr::eq(tr(text), text));
        });
    }

    #[test]
    fn a_missing_key_falls_back_to_english() {
        with_language(Locale::German, || {
            let text = "A string that no catalog has";
            assert!(std::ptr::eq(tr(text), text));
        });
    }

    #[test]
    fn a_translated_key_returns_its_translation() {
        with_language(Locale::German, || assert_eq!(tr("General"), "Allgemein"));
    }

    #[test]
    fn tr_fmt_replaces_each_named_placeholder() {
        assert_eq!(tr_fmt!("Pushed {branch} to {upstream}.", branch = "main", upstream = "origin/main"), "Pushed main to origin/main.");
        assert_eq!(tr_fmt!("No placeholders."), "No placeholders.");
    }

    #[test]
    fn tr_fmt_leaves_unknown_placeholders_and_unclosed_braces() {
        assert_eq!(fill("{a} and {b} {c", &[("a", "1".into())]), "1 and {b} {c");
        assert_eq!(fill("{a}{a}", &[("a", "x{a}".into())]), "x{a}x{a}");
    }

    #[test]
    fn tr_fmt_translates_the_template_before_filling_it() {
        with_language(Locale::German, || {
            assert_eq!(tr_fmt!("System ({language})", language = "Deutsch"), "System (Deutsch)");
        });
    }

    #[test]
    fn plurals_follow_each_languages_rule() {
        assert!(is_singular(Locale::English, 1) && !is_singular(Locale::English, 0) && !is_singular(Locale::English, 2));
        assert!(is_singular(Locale::German, 1) && !is_singular(Locale::German, 0));
        assert!(is_singular(Locale::Spanish, 1) && !is_singular(Locale::Spanish, 2));
        assert!(is_singular(Locale::PortugueseBrazil, 1) && !is_singular(Locale::PortugueseBrazil, 0));
        assert!(is_singular(Locale::French, 0) && is_singular(Locale::French, 1) && !is_singular(Locale::French, 2));
        assert!(!is_singular(Locale::Japanese, 1) && !is_singular(Locale::ChineseSimplified, 1));
    }

    #[test]
    fn tr_count_fills_the_count_in_the_chosen_form() {
        assert_eq!(tr_count("{count} commit", "{count} commits", 1), "1 commit");
        assert_eq!(tr_count("{count} commit", "{count} commits", 3), "3 commits");
        assert_eq!(tr_count("{count} commit", "{count} commits", 0), "0 commits");
    }

    #[test]
    fn system_tags_select_a_supported_locale() {
        assert_eq!(Locale::from_tag("de-DE"), Locale::German);
        assert_eq!(Locale::from_tag("pt_PT"), Locale::PortugueseBrazil);
        assert_eq!(Locale::from_tag("zh-CN"), Locale::ChineseSimplified);
        assert_eq!(Locale::from_tag("zh-Hant-TW"), Locale::English);
        assert_eq!(Locale::from_tag("nl-NL"), Locale::English);
    }

    /// Every `tr("...")`, `tr_fmt!("...", ...)` and `tr_count("...", "...")` key in the source.
    fn keys_in_source() -> BTreeSet<String> {
        let mut keys = BTreeSet::new();
        let mut files = Vec::new();
        collect_rust_files(&PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src"), &mut files);
        for file in files {
            // This module holds the test's own examples, which are not interface text.
            if file.file_name().is_some_and(|name| name == "i18n.rs") {
                continue;
            }
            let text = std::fs::read_to_string(&file).unwrap();
            for (calls, count) in [("tr(", 1), ("tr_fmt!(", 1), ("tr_count(", 2)] {
                for start in call_starts(&text, calls) {
                    for literal in string_literals_after(&text[start..], count) {
                        keys.insert(literal);
                    }
                }
            }
        }
        keys
    }

    fn collect_rust_files(dir: &Path, files: &mut Vec<PathBuf>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                collect_rust_files(&path, files);
            } else if path.extension().is_some_and(|extension| extension == "rs") {
                files.push(path);
            }
        }
    }

    /// The byte offsets just after each call of `name` that is not part of a longer identifier.
    fn call_starts(text: &str, name: &str) -> Vec<usize> {
        text.match_indices(name)
            .filter(|(at, _)| text[..*at].chars().next_back().is_none_or(|before| !(before.is_alphanumeric() || before == '_')))
            .map(|(at, _)| at + name.len())
            .collect()
    }

    /// The first `count` string literals in the call's arguments, decoded. Stops at the call's
    /// first non-literal argument.
    fn string_literals_after(call: &str, count: usize) -> Vec<String> {
        let mut literals = Vec::new();
        let mut rest = call.trim_start();
        while literals.len() < count {
            let Some(body) = rest.strip_prefix('"') else { break };
            let (literal, after) = read_literal(body);
            literals.push(literal);
            rest = after.trim_start().trim_start_matches(',').trim_start();
        }
        literals
    }

    /// Decodes a string literal's body, given the text after its opening quote, and returns it
    /// with the text after its closing quote.
    fn read_literal(body: &str) -> (String, &str) {
        let mut decoded = String::new();
        let mut chars = body.char_indices();
        while let Some((index, character)) = chars.next() {
            match character {
                '"' => return (decoded, &body[index + 1..]),
                '\\' => {
                    let (_, escaped) = chars.next().unwrap();
                    decoded.push(match escaped {
                        'n' => '\n',
                        't' => '\t',
                        other => other,
                    });
                }
                other => decoded.push(other),
            }
        }
        (decoded, "")
    }

    /// The `{name}` placeholders of a template, sorted and without duplicates.
    fn placeholders(text: &str) -> BTreeSet<String> {
        let mut names = BTreeSet::new();
        let mut rest = text;
        while let Some(start) = rest.find('{') {
            let after = &rest[start + 1..];
            let Some(end) = after.find('}') else { break };
            names.insert(after[..end].to_string());
            rest = &after[end + 1..];
        }
        names
    }

    fn catalog_entries(source: &'static str) -> Vec<(String, String)> {
        source
            .lines()
            .map(|line| line.strip_suffix('\r').unwrap_or(line))
            .filter(|line| !line.is_empty() && !line.starts_with('#'))
            .filter_map(|line| line.split_once('\t'))
            .map(|(english, translation)| (unescape(english).into_owned(), unescape(translation).into_owned()))
            .collect()
    }

    #[test]
    fn every_catalog_matches_the_strings_in_the_source() {
        let keys = keys_in_source();
        // The guard against a scan that finds nothing: the Settings window alone has 15 keys.
        assert!(keys.len() >= 15, "the scan found only {} keys", keys.len());
        // Every problem is listed, one per line, so a catalog can be fixed in one pass.
        let mut problems = Vec::new();
        for locale in LOCALES.into_iter().filter(|locale| *locale != Locale::English) {
            let name = locale.code();
            // A key belongs to one file only, so two parts of the interface never disagree.
            let mut seen: HashMap<String, &str> = HashMap::new();
            let mut entries = Vec::new();
            for (file, source) in locale.catalog_files() {
                for (key, value) in catalog_entries(source) {
                    if let Some(other) = seen.insert(key.clone(), file) {
                        problems.push(format!("duplicate\t{name}\t{file}\t{key:?}\talso in {other}.tsv"));
                    }
                    entries.push((key, value, *file));
                }
            }
            let translations: HashMap<&str, &str> = entries.iter().map(|(key, value, _)| (key.as_str(), value.as_str())).collect();
            for key in &keys {
                match translations.get(key.as_str()) {
                    None => problems.push(format!("missing\t{name}\t-\t{key:?}")),
                    Some(translation) if translation.trim().is_empty() => problems.push(format!("empty\t{name}\t-\t{key:?}")),
                    Some(translation) if placeholders(key) != placeholders(translation) => {
                        problems.push(format!("placeholders\t{name}\t-\t{key:?}\t{translation:?}"))
                    }
                    Some(_) => {}
                }
            }
            // common.tsv holds everyday words ready for any part of the interface, used or not.
            for (key, _, file) in entries.iter().filter(|(_, _, file)| *file != "common") {
                if !keys.contains(key) {
                    problems.push(format!("unused\t{name}\t{file}\t{key:?}"));
                }
            }
        }
        assert!(problems.is_empty(), "{} catalog problems:\n{}", problems.len(), problems.join("\n"));
    }

    #[test]
    fn the_scan_finds_plain_and_formatted_calls() {
        let keys = keys_in_source();
        assert!(keys.contains("General"));
        assert!(keys.contains("System ({language})"));
    }

    #[test]
    fn string_literals_handle_escaped_quotes() {
        assert_eq!(string_literals_after("\"Say \\\"hi\\\"\")", 1), vec!["Say \"hi\""]);
        assert_eq!(string_literals_after("\"{count} commit\", \"{count} commits\", n)", 2), vec!["{count} commit", "{count} commits"]);
    }
}
