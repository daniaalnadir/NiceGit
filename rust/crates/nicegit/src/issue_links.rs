//! Links in commit and stash messages to issues: `#123` and `!123` to the repository's host, `AB#123`
//! to Azure DevOps work items, and the custom rules Settings holds, such as `JIRA-7` to a tracker.
//! This module is pure logic: it finds the links and builds their addresses, and draws and opens
//! nothing.

use std::cmp::Reverse;
use std::sync::{Arc, Mutex, OnceLock};

use nicegit_core::hosting::{encode_path, Host, HostedRepository};
use nicegit_core::Snapshot;
use regex::{Captures, Regex, RegexBuilder};
use serde::{Deserialize, Serialize};

use crate::git_ext::hosted_repository_for_remote;
use crate::settings::Settings;

/// The most a custom pattern may compile to, so one huge pattern cannot take the app's memory.
const PATTERN_SIZE_LIMIT: usize = 1 << 20;

/// A rule the person added in Settings: text that matches `pattern` links to `url`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct IssueLinkRule {
    /// A regular expression, as the `regex` crate reads it.
    pub pattern: String,
    /// The address to open. `$0` is the whole match and `$1` to `$9` are its groups. Each captured
    /// text is percent-encoded, so it cannot change the address's scheme or host.
    pub url: String,
    /// The rule applies only when a remote address of the repository contains this text, ignoring
    /// case. Empty means every repository.
    pub remote_contains: String,
}

/// A part of a message: text, or a link when `url` is set.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Segment {
    pub text: String,
    pub url: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Piece {
    Text(String),
    Group(usize),
}

/// A rule whose pattern compiled and whose address template is valid.
#[derive(Clone, Debug)]
struct CompiledRule {
    regex: Regex,
    template: Vec<Piece>,
    /// Lower-cased and trimmed; empty for every repository.
    remote_contains: String,
}

/// The custom rules, compiled. A rule that does not compile is left out; Settings never saves one.
#[derive(Clone, Debug, Default)]
pub struct CompiledRules {
    rules: Vec<CompiledRule>,
}

/// The rules as they were when last compiled, with the compiled form. Rebuilt only when they change.
static COMPILED: Mutex<Option<(Vec<IssueLinkRule>, Arc<CompiledRules>)>> = Mutex::new(None);

/// Checks one rule, returning the problem to show the person when it is not valid.
pub fn check_rule(rule: &IssueLinkRule) -> Result<(), String> {
    compile_rule(rule).map(|_| ())
}

fn compile_rule(rule: &IssueLinkRule) -> Result<CompiledRule, String> {
    if rule.pattern.is_empty() {
        return Err("Enter a pattern to match, such as \\b[A-Z]+-\\d+\\b.".to_string());
    }
    let regex = RegexBuilder::new(&rule.pattern).size_limit(PATTERN_SIZE_LIMIT).build().map_err(|error| {
        // The regex crate prints the pattern with a caret before its message; the message is last.
        let detail = error.to_string();
        let last = detail.lines().rev().find(|line| !line.trim().is_empty()).unwrap_or_default().trim();
        format!("The pattern is not a valid regular expression: {}", last.strip_prefix("error: ").unwrap_or(last))
    })?;
    let url = rule.url.trim();
    if url.is_empty() {
        return Err("Enter the address to open, such as https://example.atlassian.net/browse/$1.".to_string());
    }
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return Err("The address must start with https:// or http://.".to_string());
    }
    let template = parse_template(url);
    let captures = regex.captures_len();
    for piece in &template {
        if let Piece::Group(number) = piece {
            if *number >= captures {
                let groups = captures - 1;
                let plural = if groups == 1 { "" } else { "s" };
                return Err(format!("The address uses ${number}, but the pattern has {groups} group{plural}. Use $0 for the whole match."));
            }
        }
    }
    Ok(CompiledRule { regex, template, remote_contains: rule.remote_contains.trim().to_lowercase() })
}

/// Splits an address template into its text and its `$0` to `$9` groups. A `$` that is not followed
/// by a digit is text.
fn parse_template(url: &str) -> Vec<Piece> {
    let mut pieces = Vec::new();
    let mut text = String::new();
    let mut chars = url.chars().peekable();
    while let Some(c) = chars.next() {
        let group = if c == '$' { chars.peek().and_then(|next| next.to_digit(10)) } else { None };
        match group {
            Some(number) => {
                chars.next();
                if !text.is_empty() {
                    pieces.push(Piece::Text(std::mem::take(&mut text)));
                }
                pieces.push(Piece::Group(number as usize));
            }
            None => text.push(c),
        }
    }
    if !text.is_empty() {
        pieces.push(Piece::Text(text));
    }
    pieces
}

impl CompiledRules {
    /// Compiles the rules, leaving out any that are not valid.
    pub fn compile(rules: &[IssueLinkRule]) -> Self {
        Self { rules: rules.iter().filter_map(|rule| compile_rule(rule).ok()).collect() }
    }

    /// The rules that apply to a repository with these remote addresses.
    pub fn for_remotes(&self, addresses: &[String]) -> Self {
        let lowered: Vec<String> = addresses.iter().map(|address| address.to_lowercase()).collect();
        let rules = self
            .rules
            .iter()
            .filter(|rule| rule.remote_contains.is_empty() || lowered.iter().any(|address| address.contains(&rule.remote_contains)))
            .cloned()
            .collect();
        Self { rules }
    }
}

/// The compiled form of `rules`. The same rules give the same compiled value, built once.
pub fn compiled(rules: &[IssueLinkRule]) -> Arc<CompiledRules> {
    let mut cache = COMPILED.lock().unwrap_or_else(|poison| poison.into_inner());
    if let Some((key, value)) = cache.as_ref() {
        if key.as_slice() == rules {
            return value.clone();
        }
    }
    let value = Arc::new(CompiledRules::compile(rules));
    *cache = Some((rules.to_vec(), value.clone()));
    value
}

/// The links one repository's messages get: its custom rules and the issue links of its host.
#[derive(Clone, Debug, Default)]
pub struct IssueLinks {
    rules: CompiledRules,
    host: Option<HostedRepository>,
}

impl IssueLinks {
    /// The links for the open repository, from Settings and its remotes. The host comes from the
    /// `origin` remote, or the first remote when there is no origin.
    pub fn new(settings: &Settings, snapshot: &Snapshot) -> Self {
        let addresses: Vec<String> = snapshot.remote_fetch_addresses.values().flatten().cloned().collect();
        let rules = compiled(&settings.issue_links).for_remotes(&addresses);
        let host = if settings.link_host_issues {
            link_remote(snapshot).and_then(|remote| hosted_repository_for_remote(snapshot, remote, settings.listed_hosts()))
        } else {
            None
        };
        Self { rules, host }
    }

    /// The text split into its links and the text between them.
    pub fn segments(&self, text: &str) -> Vec<Segment> {
        segments(text, &self.rules, self.host.as_ref())
    }
}

/// The remote whose host the host links name: `origin`, or the first remote without one.
fn link_remote(snapshot: &Snapshot) -> Option<&str> {
    if snapshot.remotes.iter().any(|remote| remote == "origin") {
        Some("origin")
    } else {
        snapshot.remotes.first().map(String::as_str)
    }
}

/// A match found in the text, with the address it links to.
struct Found {
    start: usize,
    end: usize,
    url: String,
}

/// Splits `text` into segments, where each link is a segment with an address. Matches are taken
/// from the earliest start, then the longest, so no link lies inside another. Where a custom rule
/// and a host link start and end alike, the custom rule wins, as it comes first.
pub fn segments(text: &str, rules: &CompiledRules, host: Option<&HostedRepository>) -> Vec<Segment> {
    let mut found = Vec::new();
    for rule in &rules.rules {
        for captures in rule.regex.captures_iter(text) {
            let Some(whole) = captures.get(0) else { continue };
            if whole.is_empty() {
                continue;
            }
            found.push(Found { start: whole.start(), end: whole.end(), url: expand(&rule.template, &captures) });
        }
    }
    if let Some(repository) = host {
        found.extend(host_links(text, repository));
    }
    // A stable sort, so equal spans keep the custom rules first.
    found.sort_by_key(|link| (link.start, Reverse(link.end)));

    let mut segments = Vec::new();
    // The end of the last link taken; text before the next link starts here.
    let mut last = 0;
    for link in found {
        if link.start < last {
            continue;
        }
        push_text(&mut segments, &text[last..link.start]);
        segments.push(Segment { text: text[link.start..link.end].to_string(), url: Some(link.url) });
        last = link.end;
    }
    push_text(&mut segments, &text[last..]);
    segments
}

fn push_text(segments: &mut Vec<Segment>, text: &str) {
    if !text.is_empty() {
        segments.push(Segment { text: text.to_string(), url: None });
    }
}

/// The address a template gives for one match, with each captured text percent-encoded.
fn expand(template: &[Piece], captures: &Captures) -> String {
    let mut url = String::new();
    for piece in template {
        match piece {
            Piece::Text(text) => url.push_str(text),
            Piece::Group(number) => {
                if let Some(group) = captures.get(*number) {
                    url.push_str(&encode_capture(group.as_str()));
                }
            }
        }
    }
    url
}

/// Percent-encodes everything outside the unreserved characters of RFC 3986.
fn encode_capture(text: &str) -> String {
    let mut encoded = String::new();
    for byte in text.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => encoded.push(byte as char),
            _ => encoded.push_str(&format!("%{byte:02X}")),
        }
    }
    encoded
}

/// `#123` not preceded by a word character and not followed by one.
fn hash_number() -> &'static Regex {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    PATTERN.get_or_init(|| Regex::new(r"\B#(\d+)\b").expect("a fixed pattern"))
}

/// `!123` not preceded by a word character and not followed by one.
fn bang_number() -> &'static Regex {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    PATTERN.get_or_init(|| Regex::new(r"\B!(\d+)\b").expect("a fixed pattern"))
}

/// `AB#123` as a whole word.
fn azure_work_item() -> &'static Regex {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    PATTERN.get_or_init(|| Regex::new(r"\bAB#(\d+)\b").expect("a fixed pattern"))
}

/// The links the repository's host gives: issues for `#123`, merge requests for GitLab's `!123`,
/// and Azure DevOps work items for `AB#123`. Azure DevOps has no `#123` link here.
fn host_links(text: &str, repository: &HostedRepository) -> Vec<Found> {
    let home = repository.home_url();
    match repository.kind {
        Host::GitHub | Host::Bitbucket | Host::Gitea => numbered(hash_number(), text, |number| format!("{home}/issues/{number}")),
        Host::GitLab => {
            let mut links = numbered(hash_number(), text, |number| format!("{home}/-/issues/{number}"));
            links.extend(numbered(bang_number(), text, |number| format!("{home}/-/merge_requests/{number}")));
            links
        }
        Host::AzureDevOps => {
            let project = encode_path(&repository.owner_path);
            numbered(azure_work_item(), text, |number| format!("https://dev.azure.com/{project}/_workitems/edit/{number}"))
        }
    }
}

/// The matches of a pattern with one number group, each linked by `url` for that number.
fn numbered(regex: &Regex, text: &str, url: impl Fn(&str) -> String) -> Vec<Found> {
    regex
        .captures_iter(text)
        .filter_map(|captures| {
            let whole = captures.get(0)?;
            let number = captures.get(1)?.as_str();
            Some(Found { start: whole.start(), end: whole.end(), url: url(number) })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::Arc;

    use nicegit_core::hosting::{HostedRepository, ListedHosts};
    use nicegit_core::Snapshot;

    use super::{check_rule, compiled, segments, CompiledRules, IssueLinkRule, IssueLinks, Segment};
    use crate::settings::Settings;

    /// The links in `segments`, as (text, address) pairs.
    fn links(segments: &[Segment]) -> Vec<(String, String)> {
        segments.iter().filter_map(|segment| Some((segment.text.clone(), segment.url.clone()?))).collect()
    }

    fn pairs(list: &[(&str, &str)]) -> Vec<(String, String)> {
        list.iter().map(|(text, url)| (text.to_string(), url.to_string())).collect()
    }

    fn rule(pattern: &str, url: &str) -> IssueLinkRule {
        IssueLinkRule { pattern: pattern.to_string(), url: url.to_string(), remote_contains: String::new() }
    }

    fn jira() -> IssueLinkRule {
        rule(r"\b([A-Z][A-Z0-9]+-\d+)\b", "https://example.atlassian.net/browse/$1")
    }

    /// The segments of `text` with the rules and a repository at `host_address`, if it is supported.
    fn with_rules(list: &[IssueLinkRule], text: &str, host_address: Option<&str>) -> Vec<Segment> {
        let host = host_address.and_then(|address| HostedRepository::parse(address, ListedHosts::default()).ok());
        segments(text, &CompiledRules::compile(list), host.as_ref())
    }

    #[test]
    fn github_remotes_link_hash_numbers_to_issues_in_both_address_forms() {
        for address in ["https://github.com/acme/widgets.git", "git@github.com:acme/widgets.git"] {
            let found = with_rules(&[], "Fix crash (#42)", Some(address));
            assert_eq!(links(&found), pairs(&[("#42", "https://github.com/acme/widgets/issues/42")]), "{address}");
            assert_eq!(found.iter().map(|s| s.text.as_str()).collect::<String>(), "Fix crash (#42)");
        }
    }

    #[test]
    fn gitlab_remotes_link_hash_to_issues_and_bang_to_merge_requests() {
        let found = with_rules(&[], "Closes #7 and !8, see #9.", Some("https://gitlab.com/group/sub/proj.git"));
        assert_eq!(
            links(&found),
            pairs(&[
                ("#7", "https://gitlab.com/group/sub/proj/-/issues/7"),
                ("!8", "https://gitlab.com/group/sub/proj/-/merge_requests/8"),
                ("#9", "https://gitlab.com/group/sub/proj/-/issues/9"),
            ])
        );
    }

    #[test]
    fn bitbucket_remotes_link_hash_numbers_to_issues_but_not_bang_numbers() {
        let found = with_rules(&[], "Fix #3 !4", Some("git@bitbucket.org:team/tool.git"));
        assert_eq!(links(&found), pairs(&[("#3", "https://bitbucket.org/team/tool/issues/3")]));
    }

    #[test]
    fn azure_remotes_link_only_ab_work_items() {
        let azure = "https://dev.azure.com/acme/Web/_git/shop";
        let found = with_rules(&[], "Done AB#9 and #10", Some(azure));
        assert_eq!(links(&found), pairs(&[("AB#9", "https://dev.azure.com/acme/Web/_workitems/edit/9")]));
    }

    #[test]
    fn a_hash_inside_a_word_is_not_linked() {
        let github = Some("https://github.com/acme/widgets");
        for text in ["abc#12", "#12abc", "x#1 and y#2", "C#1"] {
            assert!(links(&with_rules(&[], text, github)).is_empty(), "{text} is not linked");
        }
        assert_eq!(links(&with_rules(&[], "(#12),#13.", github)).len(), 2, "punctuation around a number is fine");
    }

    #[test]
    fn a_host_without_a_known_service_or_no_remote_gives_no_host_links() {
        assert!(links(&with_rules(&[], "Fix #1", Some("https://example.com/acme/widgets"))).is_empty());
        assert!(links(&with_rules(&[], "Fix #1", None)).is_empty());
    }

    #[test]
    fn a_jira_style_rule_links_the_key_to_the_template() {
        let found = with_rules(&[jira()], "See JIRA-7 and not jira-8", None);
        assert_eq!(links(&found), pairs(&[("JIRA-7", "https://example.atlassian.net/browse/JIRA-7")]));
    }

    #[test]
    fn the_earliest_match_wins_then_the_longest() {
        // The rule starts at "Fix", before the host's "#42", so the host link inside it is dropped.
        let found = with_rules(&[rule(r"Fix #\d+", "https://tracker.example/$0")], "Fix #42", Some("https://github.com/acme/w"));
        assert_eq!(links(&found), pairs(&[("Fix #42", "https://tracker.example/Fix%20%2342")]));
        // A rule that starts later does not beat the host link that starts first.
        let found = with_rules(&[rule(r"\d+", "https://tracker.example/x")], "Fix #42", Some("https://github.com/acme/w"));
        assert_eq!(links(&found), pairs(&[("#42", "https://github.com/acme/w/issues/42")]));
    }

    #[test]
    fn a_custom_rule_wins_a_tie_with_a_host_link() {
        let found = with_rules(&[rule(r"#(\d+)", "https://tracker.example/$1")], "Fix #42", Some("https://github.com/acme/w"));
        assert_eq!(links(&found), pairs(&[("#42", "https://tracker.example/42")]));
    }

    #[test]
    fn rules_that_do_not_compile_or_have_a_bad_address_are_rejected() {
        assert!(check_rule(&rule("(abc", "https://example.com/$1")).unwrap_err().contains("not a valid regular expression"));
        assert!(check_rule(&rule("", "https://example.com/")).unwrap_err().contains("pattern"));
        assert!(check_rule(&rule("[A-Z]+", "")).unwrap_err().contains("address"));
        assert!(check_rule(&rule("[A-Z]+", "ftp://example.com/$0")).unwrap_err().contains("https://"));
        assert!(check_rule(&rule("[A-Z]+", "javascript:alert($0)")).is_err());
        let missing = check_rule(&rule(r"([A-Z]+)-(\d+)", "https://example.com/$3")).unwrap_err();
        assert!(missing.contains("$3") && missing.contains("2 groups"), "{missing}");
        assert!(check_rule(&rule(r"([A-Z]+)", "https://example.com/$0/$1")).is_ok(), "$0 and $1 both exist");
        assert!(check_rule(&rule(r"[A-Z]+", "https://example.com/$1")).is_err(), "no group to take");
        assert!(check_rule(&rule(r"[A-Z]+", "http://example.com/$0")).is_ok(), "plain http is allowed");
    }

    #[test]
    fn a_rule_that_exceeds_the_size_limit_is_rejected() {
        let huge = format!("(?:{}){{500}}", "[a-z]{100}".repeat(50));
        assert!(check_rule(&rule(&huge, "https://example.com/$0")).is_err());
    }

    #[test]
    fn captured_text_is_percent_encoded_so_it_cannot_change_the_address() {
        let found = with_rules(&[rule(r"\[([^\]]+)\]", "https://tracker.example/t/$1")], "[x y/../https://evil.example]", None);
        assert_eq!(
            links(&found),
            pairs(&[("[x y/../https://evil.example]", "https://tracker.example/t/x%20y%2F..%2Fhttps%3A%2F%2Fevil.example")])
        );
        let found = with_rules(&[rule(r"(\w+)", "https://tracker.example/$1")], "a\u{e9}", None);
        assert_eq!(links(&found)[0].1, "https://tracker.example/a%C3%A9");
    }

    #[test]
    fn a_rule_with_a_remote_filter_applies_only_to_matching_remotes() {
        let filtered = IssueLinkRule { remote_contains: "ACME".to_string(), ..jira() };
        let rules = compiled(std::slice::from_ref(&filtered));
        let acme = rules.for_remotes(&["https://github.com/acme/widgets.git".to_string()]);
        let other = rules.for_remotes(&["https://github.com/other/widgets.git".to_string()]);
        assert_eq!(links(&segments("JIRA-7", &acme, None)).len(), 1);
        assert!(links(&segments("JIRA-7", &other, None)).is_empty());
        let everywhere = compiled(&[jira()]).for_remotes(&[]);
        assert_eq!(links(&segments("JIRA-7", &everywhere, None)).len(), 1, "no filter means every repository");
    }

    #[test]
    fn turning_host_links_off_keeps_the_custom_rules() {
        let mut settings = Settings { issue_links: vec![jira()], ..Settings::default() };
        let snapshot = Snapshot {
            remotes: vec!["origin".to_string()],
            remote_fetch_addresses: BTreeMap::from([("origin".to_string(), vec!["https://github.com/acme/widgets.git".to_string()])]),
            ..Snapshot::default()
        };
        let on = IssueLinks::new(&settings, &snapshot);
        assert_eq!(links(&on.segments("JIRA-7 (#42)")).len(), 2, "the host link and the rule");
        settings.link_host_issues = false;
        let off = IssueLinks::new(&settings, &snapshot);
        assert_eq!(links(&off.segments("JIRA-7 (#42)")), pairs(&[("JIRA-7", "https://example.atlassian.net/browse/JIRA-7")]));
    }

    #[test]
    fn the_host_comes_from_origin_or_else_the_first_remote() {
        let mut snapshot = Snapshot {
            remotes: vec!["mirror".to_string(), "origin".to_string()],
            remote_fetch_addresses: BTreeMap::from([
                ("mirror".to_string(), vec!["https://bitbucket.org/team/tool.git".to_string()]),
                ("origin".to_string(), vec!["https://github.com/acme/widgets.git".to_string()]),
            ]),
            ..Snapshot::default()
        };
        let settings = Settings::default();
        assert_eq!(links(&IssueLinks::new(&settings, &snapshot).segments("#1"))[0].1, "https://github.com/acme/widgets/issues/1");
        snapshot.remotes = vec!["mirror".to_string()];
        assert_eq!(links(&IssueLinks::new(&settings, &snapshot).segments("#1"))[0].1, "https://bitbucket.org/team/tool/issues/1");
    }

    #[test]
    fn compiled_rules_are_built_once_for_the_same_rules() {
        let list = vec![jira()];
        let first = compiled(&list);
        assert!(Arc::ptr_eq(&first, &compiled(&list)), "the same rules reuse the compiled form");
        let changed = vec![rule(r"\bX-\d+\b", "https://example.com/$0")];
        assert!(!Arc::ptr_eq(&first, &compiled(&changed)), "changed rules are compiled again");
    }

    #[test]
    fn the_segments_always_add_up_to_the_original_text() {
        let text = "Fix JIRA-7 (#42), !3 and AB#4\nSee JIRA-8.";
        let found = with_rules(&[jira()], text, Some("https://github.com/acme/widgets"));
        assert_eq!(found.iter().map(|s| s.text.as_str()).collect::<String>(), text);
        assert!(with_rules(&[], "", None).is_empty());
    }
}
