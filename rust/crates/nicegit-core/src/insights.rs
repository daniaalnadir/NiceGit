//! Repository insights: how much the history changed, when, and by whom. Everything comes from one
//! `git log` of the current branch or of every branch, read as it arrives. A file keeps its
//! history under its newest name when it was renamed.

use std::collections::{HashMap, HashSet};
use std::io::{BufRead, BufReader, Read};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::client::GitClient;
use crate::models::{GitError, Result};
use crate::runner;

/// The most commits one read covers. A longer history shows its newest commits and says so.
pub const COMMIT_LIMIT: usize = 20_000;
/// How many files the most-changed list holds.
pub const FILE_LIMIT: usize = 15;
/// The folder name for a file at the top of the repository.
pub const ROOT_FOLDER: &str = "(root)";

const DAY: i64 = 86_400;
/// Written before each commit's header. `%x1e` is a record separator, which a numstat line or a
/// name cannot start with, so it marks the start of a commit.
const LOG_FORMAT: &str = "--format=%x1e%H%x00%aN%x00%aE%x00%at";
const RECORD_START: u8 = 0x1e;
const OPERATION: &str = "git log";
const CANCELLED: &str = "The read was cancelled.";

/// How far back the history is read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InsightsPeriod {
    Days30,
    Days90,
    Year,
    AllTime,
}

impl InsightsPeriod {
    /// How many days back the period reaches, or `None` for all of history.
    pub fn days(self) -> Option<i64> {
        match self {
            InsightsPeriod::Days30 => Some(30),
            InsightsPeriod::Days90 => Some(90),
            InsightsPeriod::Year => Some(365),
            InsightsPeriod::AllTime => None,
        }
    }

    /// Thirty days is shown day by day; longer periods week by week.
    pub fn granularity(self) -> Granularity {
        match self {
            InsightsPeriod::Days30 => Granularity::Day,
            _ => Granularity::Week,
        }
    }
}

/// What the commits-over-time series counts: one day or one ISO week (starting Monday).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Granularity {
    Day,
    #[default]
    Week,
}

/// Which commits to read: a period, and the current branch or every branch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InsightsRange {
    pub period: InsightsPeriod,
    pub all_branches: bool,
}

/// One bucket of the series: the commits whose time falls in it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Bucket {
    /// Midnight UTC, in seconds since the Unix epoch, of the bucket's first day.
    pub start: i64,
    pub commits: usize,
}

/// One author, merged by their mailmapped email address.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthorStat {
    /// The name on the author's newest commit in the range.
    pub name: String,
    /// The mailmapped address, lower-cased.
    pub email: String,
    pub commits: usize,
    pub added: u64,
    pub removed: u64,
    /// Author times of the first and last commits, in seconds since the Unix epoch.
    pub first: i64,
    pub last: i64,
}

/// One file, followed through renames to its newest name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileStat {
    pub path: String,
    /// Commits that changed the file.
    pub commits: usize,
    /// Lines added plus lines removed. Binary changes count as zero lines.
    pub lines: u64,
}

/// One top-level folder, or [`ROOT_FOLDER`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FolderStat {
    pub name: String,
    /// Commits that changed at least one file in the folder.
    pub commits: usize,
}

/// The whole read: totals, the series, and the rankings.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Insights {
    pub commits: usize,
    pub active_days: usize,
    /// Author times of the oldest and newest commits, in seconds since the Unix epoch.
    pub first_commit: Option<i64>,
    pub last_commit: Option<i64>,
    /// True when the history held more commits than one read covers.
    pub truncated: bool,
    pub granularity: Granularity,
    /// Oldest first, with empty buckets filled in as zeros.
    pub series: Vec<Bucket>,
    /// Most commits first.
    pub authors: Vec<AuthorStat>,
    /// The most-changed files, at most [`FILE_LIMIT`], most commits first.
    pub files: Vec<FileStat>,
    /// Most commits first.
    pub folders: Vec<FolderStat>,
}

impl GitClient {
    /// The insights for `range` of `directory`'s history. `cancel` is checked between commits; once
    /// it is set the read stops and returns an error.
    pub fn insights(&self, directory: &Path, range: InsightsRange, cancel: &AtomicBool) -> Result<Insights> {
        self.insights_with_progress(directory, range, cancel, &AtomicUsize::new(0))
    }

    /// Like [`GitClient::insights`], and adds one to `progress` for each commit read, so a caller
    /// can show how far the read has got.
    pub fn insights_with_progress(
        &self,
        directory: &Path,
        range: InsightsRange,
        cancel: &AtomicBool,
        progress: &AtomicUsize,
    ) -> Result<Insights> {
        if cancel.load(Ordering::Relaxed) {
            return Err(cancelled());
        }
        let mut collector = Collector { granularity: range.period.granularity(), ..Collector::default() };
        // Before the first commit HEAD names nothing, and `git log HEAD` would fail.
        if range.all_branches || self.head(directory).is_some() {
            self.read_history(directory, range, cancel, progress, &mut collector)?;
        }
        Ok(collector.into_insights(range.period, now_seconds()))
    }

    fn read_history(
        &self,
        directory: &Path,
        range: InsightsRange,
        cancel: &AtomicBool,
        progress: &AtomicUsize,
        collector: &mut Collector,
    ) -> Result<()> {
        let revision = if range.all_branches { "--all" } else { "HEAD" };
        let limit = COMMIT_LIMIT.to_string();
        let since = range.period.days().map(|days| format!("--since={days} days ago"));
        let mut arguments = vec!["log", revision, "--no-merges", "--use-mailmap", LOG_FORMAT, "--numstat", "-z", "-n", limit.as_str()];
        if let Some(since) = since.as_deref() {
            arguments.push(since);
        }
        arguments.push("--");

        let mut child = runner::spawn_piped(&arguments, directory, &self.options())?;
        let Some(stdout) = child.stdout.take() else {
            let _ = child.kill();
            let _ = child.wait();
            return Err(GitError::failed(OPERATION, "Git gave no output."));
        };
        // Errors are read on their own thread, so a full error pipe cannot stop the output.
        let mut stderr = child.stderr.take();
        let errors = std::thread::spawn(move || {
            let mut bytes = Vec::new();
            if let Some(stream) = stderr.as_mut() {
                let _ = stream.read_to_end(&mut bytes);
            }
            bytes
        });

        let outcome = {
            let mut reader = BufReader::new(stdout);
            read_records(&mut reader, cancel, progress, collector)
        };
        if outcome.is_err() {
            let _ = child.kill();
        }
        let status = child.wait().map_err(|error| GitError::failed(OPERATION, error.to_string()))?;
        let errors = errors.join().unwrap_or_default();
        outcome?;
        if !status.success() {
            let message = String::from_utf8_lossy(&errors).trim().to_string();
            let message = if message.is_empty() { format!("Exited with status {}.", status.code().unwrap_or(-1)) } else { message };
            return Err(GitError::failed(OPERATION, message));
        }
        Ok(())
    }
}

/// Reads the NUL-separated records of `git log --numstat -z` into `collector`. A commit is its
/// header (the record start, then the hash, name, email, and time, each NUL-terminated) followed by
/// numstat lines. A rename's numstat line has an empty path, and its old and new paths follow as
/// two more NUL-terminated fields.
fn read_records(reader: &mut impl BufRead, cancel: &AtomicBool, progress: &AtomicUsize, collector: &mut Collector) -> Result<()> {
    while let Some(token) = next_token(reader)? {
        if cancel.load(Ordering::Relaxed) {
            return Err(cancelled());
        }
        // Git ends each numstat block with a newline before its first line.
        let record = strip_leading_newlines(&token);
        if record.first() == Some(&RECORD_START) {
            let (Some(name), Some(email), Some(time)) = (next_token(reader)?, next_token(reader)?, next_token(reader)?) else {
                break;
            };
            let time = String::from_utf8_lossy(&time);
            collector.start_commit(
                String::from_utf8_lossy(&name).into_owned(),
                String::from_utf8_lossy(&email).into_owned(),
                time.trim().parse().unwrap_or(0),
            );
            progress.fetch_add(1, Ordering::Relaxed);
        } else if collector.in_commit() {
            let line = String::from_utf8_lossy(record);
            let mut fields = line.splitn(3, '\t');
            let added = count(fields.next());
            let removed = count(fields.next());
            let path = fields.next().unwrap_or("");
            if path.is_empty() {
                let (Some(old), Some(new)) = (next_token(reader)?, next_token(reader)?) else {
                    break;
                };
                collector.rename(String::from_utf8_lossy(&old).into_owned(), String::from_utf8_lossy(&new).into_owned(), added, removed);
            } else {
                collector.change(path.to_string(), added, removed);
            }
        }
    }
    collector.finish_commit();
    Ok(())
}

/// The next NUL-terminated field, without its terminator, or `None` at the end of the output.
fn next_token(reader: &mut impl BufRead) -> Result<Option<Vec<u8>>> {
    let mut token = Vec::new();
    let read = reader.read_until(0, &mut token).map_err(|error| GitError::failed(OPERATION, error.to_string()))?;
    if read == 0 {
        return Ok(None);
    }
    if token.last() == Some(&0) {
        token.pop();
    }
    Ok(Some(token))
}

fn strip_leading_newlines(bytes: &[u8]) -> &[u8] {
    let start = bytes.iter().position(|&byte| byte != b'\n').unwrap_or(bytes.len());
    &bytes[start..]
}

/// A numstat count. Binary files show `-`, which counts as zero lines.
fn count(field: Option<&str>) -> u64 {
    field.and_then(|field| field.parse().ok()).unwrap_or(0)
}

fn cancelled() -> GitError {
    GitError::failed(OPERATION, CANCELLED)
}

fn now_seconds() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|duration| duration.as_secs() as i64).unwrap_or(0)
}

/// The day a series bucket starts on: the day itself, or the Monday of its week. Day 0 (1970-01-01)
/// was a Thursday, so `(day + 3) % 7` counts days from Monday.
fn bucket_key(day: i64, granularity: Granularity) -> i64 {
    match granularity {
        Granularity::Day => day,
        Granularity::Week => day - (day + 3).rem_euclid(7),
    }
}

/// The top-level folder of a path, or [`ROOT_FOLDER`] for a file at the top.
fn folder_of(path: &str) -> &str {
    path.split_once('/').map_or(ROOT_FOLDER, |(folder, _)| folder)
}

/// The proleptic Gregorian date (year, month, day) of a day counted from 1970-01-01.
pub fn civil_date(day: i64) -> (i64, u32, u32) {
    let shifted = day + 719_468;
    let era = shifted.div_euclid(146_097);
    let day_of_era = shifted.rem_euclid(146_097);
    let year_of_era = (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day_of_month = (day_of_year - (153 * month_index + 2) / 5 + 1) as u32;
    let month = (if month_index < 10 { month_index + 3 } else { month_index - 9 }) as u32;
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    (year, month, day_of_month)
}

/// Everything read so far, folded commit by commit. Commits arrive newest first.
#[derive(Default)]
struct Collector {
    granularity: Granularity,
    commits: usize,
    days: HashSet<i64>,
    first: Option<i64>,
    last: Option<i64>,
    /// Commits per bucket, keyed by [`bucket_key`].
    buckets: HashMap<i64, usize>,
    /// Authors keyed by lower-cased mailmapped email.
    authors: HashMap<String, AuthorStat>,
    /// Old path to the path it was renamed to, so older commits count toward the newer name.
    aliases: HashMap<String, String>,
    /// Commits and lines per file, keyed by its newest name.
    files: HashMap<String, (usize, u64)>,
    /// Commits per top-level folder.
    folders: HashMap<String, usize>,
    current: Option<PendingCommit>,
}

struct PendingCommit {
    name: String,
    email: String,
    time: i64,
    changes: Vec<Change>,
}

struct Change {
    /// The file's newest name, which keys its totals.
    file: String,
    /// The path as this commit has it, which places it in a folder.
    location: String,
    added: u64,
    removed: u64,
}

impl Collector {
    fn in_commit(&self) -> bool {
        self.current.is_some()
    }

    fn start_commit(&mut self, name: String, email: String, time: i64) {
        self.finish_commit();
        self.current = Some(PendingCommit { name, email, time, changes: Vec::new() });
    }

    fn change(&mut self, location: String, added: u64, removed: u64) {
        let file = self.resolve(&location);
        if let Some(commit) = self.current.as_mut() {
            commit.changes.push(Change { file, location, added, removed });
        }
    }

    fn rename(&mut self, old: String, new: String, added: u64, removed: u64) {
        let file = self.resolve(&new);
        // Commits older than this one name the file `old`, which is now `file`.
        if file != old {
            self.aliases.insert(old, file.clone());
        }
        if let Some(commit) = self.current.as_mut() {
            commit.changes.push(Change { file, location: new, added, removed });
        }
    }

    /// The newest name of `path`, following renames from newer commits.
    fn resolve(&self, path: &str) -> String {
        let mut current = path.to_string();
        // A chain cannot be longer than the renames recorded, so the bound only guards a cycle.
        for _ in 0..=self.aliases.len() {
            match self.aliases.get(&current) {
                Some(next) => current = next.clone(),
                None => break,
            }
        }
        current
    }

    fn finish_commit(&mut self) {
        let Some(commit) = self.current.take() else { return };
        let day = commit.time.div_euclid(DAY);
        self.commits += 1;
        self.days.insert(day);
        self.first = Some(self.first.map_or(commit.time, |first| first.min(commit.time)));
        self.last = Some(self.last.map_or(commit.time, |last| last.max(commit.time)));
        *self.buckets.entry(bucket_key(day, self.granularity)).or_default() += 1;

        let added: u64 = commit.changes.iter().map(|change| change.added).sum();
        let removed: u64 = commit.changes.iter().map(|change| change.removed).sum();
        let key = commit.email.to_lowercase();
        let author = self.authors.entry(key.clone()).or_insert_with(|| AuthorStat {
            name: commit.name.clone(),
            email: key,
            commits: 0,
            added: 0,
            removed: 0,
            first: commit.time,
            last: commit.time,
        });
        author.commits += 1;
        author.added += added;
        author.removed += removed;
        author.first = author.first.min(commit.time);
        author.last = author.last.max(commit.time);

        let mut touched_files = HashSet::new();
        let mut touched_folders = HashSet::new();
        for change in &commit.changes {
            let entry = self.files.entry(change.file.clone()).or_insert((0, 0));
            entry.1 += change.added + change.removed;
            if touched_files.insert(change.file.as_str()) {
                entry.0 += 1;
            }
            let folder = folder_of(&change.location);
            if touched_folders.insert(folder) {
                *self.folders.entry(folder.to_string()).or_default() += 1;
            }
        }
    }

    fn into_insights(self, period: InsightsPeriod, now: i64) -> Insights {
        let series = self.series(period, now);
        let mut authors: Vec<AuthorStat> = self.authors.into_values().collect();
        authors.sort_by(|a, b| {
            b.commits.cmp(&a.commits).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())).then_with(|| a.email.cmp(&b.email))
        });
        let mut files: Vec<FileStat> = self.files.into_iter().map(|(path, (commits, lines))| FileStat { path, commits, lines }).collect();
        files.sort_by(|a, b| b.commits.cmp(&a.commits).then(b.lines.cmp(&a.lines)).then_with(|| a.path.cmp(&b.path)));
        files.truncate(FILE_LIMIT);
        let mut folders: Vec<FolderStat> = self.folders.into_iter().map(|(name, commits)| FolderStat { name, commits }).collect();
        folders.sort_by(|a, b| b.commits.cmp(&a.commits).then_with(|| a.name.cmp(&b.name)));
        Insights {
            commits: self.commits,
            active_days: self.days.len(),
            first_commit: self.first,
            last_commit: self.last,
            truncated: self.commits >= COMMIT_LIMIT,
            granularity: self.granularity,
            series,
            authors,
            files,
            folders,
        }
    }

    /// Buckets from the period's start to its end, filled with zeros. A period runs from its
    /// start to today; all of history runs from the first commit to the last.
    fn series(&self, period: InsightsPeriod, now: i64) -> Vec<Bucket> {
        let mut start = self.first.map(|time| time.div_euclid(DAY));
        let mut end = self.last.map(|time| time.div_euclid(DAY));
        if let Some(days) = period.days() {
            let since = (now - days * DAY).div_euclid(DAY);
            let today = now.div_euclid(DAY);
            start = Some(start.map_or(since, |first| first.min(since)));
            end = Some(end.map_or(today, |last| last.max(today)));
        }
        let (Some(start), Some(end)) = (start, end) else { return Vec::new() };
        let step = match self.granularity {
            Granularity::Day => 1,
            Granularity::Week => 7,
        };
        let last_key = bucket_key(end, self.granularity);
        let mut key = bucket_key(start, self.granularity);
        let mut series = Vec::new();
        while key <= last_key {
            series.push(Bucket { start: key * DAY, commits: self.buckets.get(&key).copied().unwrap_or(0) });
            key += step;
        }
        series
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    /// 2023-11-14, a Tuesday.
    const BASE: i64 = 1_700_000_000;

    fn repo() -> (tempfile::TempDir, GitClient) {
        let dir = tempfile::tempdir().expect("temp dir");
        let client = GitClient::new();
        client.run(&["init", "-q"], dir.path()).expect("init");
        (dir, client)
    }

    /// Writes `files` as text, stages everything, and commits it as `author` at `when`.
    fn commit_as(client: &GitClient, dir: &Path, author: (&str, &str), when: i64, files: &[(&str, &str)]) {
        for (path, text) in files {
            let target = dir.join(path);
            fs::create_dir_all(target.parent().expect("a parent folder")).expect("folder");
            fs::write(&target, text).expect("write");
        }
        client.run(&["add", "-A"], dir).expect("stage");
        let identity = format!("{} <{}>", author.0, author.1);
        let date = format!("@{when} +0000");
        // `--since` filters on the committer date, so both dates are set to the same moment.
        let dates = [("GIT_COMMITTER_DATE", date.as_str())];
        let arguments = [
            "-c",
            "user.name=Committer",
            "-c",
            "user.email=committer@example.com",
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-q",
            "--allow-empty",
            "--author",
            &identity,
            "--date",
            &date,
            "-m",
            "change",
        ];
        runner::run(&arguments, dir, &runner::RunOptions { env: &dates, ..Default::default() }).expect("commit");
    }

    fn all_time(all_branches: bool) -> InsightsRange {
        InsightsRange { period: InsightsPeriod::AllTime, all_branches }
    }

    /// Five commits by two authors, with a mailmap that merges one author's second address:
    /// - Ann adds `a.txt` (2 lines).
    /// - Bob adds `src/lib.rs` (3 lines) and `README.md` (1 line), a week later.
    /// - Ann, under `ann@old.example.com`, changes one line of `src/lib.rs` and adds binary `bin.dat`, a week later.
    /// - Bob renames `a.txt` to `docs/a.md`, three weeks after the first commit (a week is empty before it).
    /// - Ann adds `notes.txt` (1 line), a day later.
    fn history() -> (tempfile::TempDir, GitClient) {
        let (dir, client) = repo();
        let path = dir.path().to_path_buf();
        let mailmap: PathBuf = path.join(".git").join("insights.mailmap");
        fs::write(&mailmap, "Ann Lee <ann@example.com> <ann@old.example.com>\n").expect("mailmap");
        let mailmap = mailmap.to_string_lossy().into_owned();
        client.run(&["config", "--local", "mailmap.file", &mailmap], &path).expect("mailmap file");

        commit_as(&client, &path, ("Ann", "ann@example.com"), BASE, &[("a.txt", "x\ny\n")]);
        commit_as(
            &client,
            &path,
            ("Bob", "bob@example.com"),
            BASE + DAY,
            &[("src/lib.rs", "one\ntwo\nthree\n"), ("README.md", "# Project\n")],
        );
        fs::write(path.join("bin.dat"), [0u8, 1, 2, 0, 255]).expect("binary");
        commit_as(&client, &path, ("Ann", "ann@old.example.com"), BASE + 8 * DAY, &[("src/lib.rs", "one\n2\nthree\n")]);
        fs::create_dir_all(path.join("docs")).expect("docs");
        fs::rename(path.join("a.txt"), path.join("docs").join("a.md")).expect("rename");
        commit_as(&client, &path, ("Bob", "bob@example.com"), BASE + 22 * DAY, &[]);
        commit_as(&client, &path, ("Ann Lee", "ann@example.com"), BASE + 23 * DAY, &[("notes.txt", "todo\n")]);
        (dir, client)
    }

    #[test]
    fn counts_authors_files_and_folders_from_real_history() {
        let (dir, client) = history();
        let cancel = AtomicBool::new(false);
        let progress = AtomicUsize::new(0);
        let insights = client.insights_with_progress(dir.path(), all_time(false), &cancel, &progress).expect("insights");

        assert_eq!(progress.load(Ordering::Relaxed), 5, "every commit is read");
        assert_eq!(insights.commits, 5);
        assert_eq!(insights.active_days, 5);
        assert_eq!(insights.first_commit, Some(BASE));
        assert_eq!(insights.last_commit, Some(BASE + 23 * DAY));
        assert!(!insights.truncated);
        assert_eq!(insights.granularity, Granularity::Week);

        // Weeks from the first commit's week to the last: 2, 1, then an empty week, then 2.
        let counts: Vec<usize> = insights.series.iter().map(|bucket| bucket.commits).collect();
        assert_eq!(counts, vec![2, 1, 0, 2], "the empty week is a zero");
        assert_eq!(insights.series[0].start, (BASE.div_euclid(DAY) - 1) * DAY, "the bucket starts on the Monday");
        assert_eq!(insights.series.iter().map(|bucket| bucket.commits).sum::<usize>(), insights.commits);

        let authors: Vec<(&str, &str, usize, u64, u64)> = insights
            .authors
            .iter()
            .map(|author| (author.name.as_str(), author.email.as_str(), author.commits, author.added, author.removed))
            .collect();
        assert_eq!(
            authors,
            vec![("Ann Lee", "ann@example.com", 3, 4, 1), ("Bob", "bob@example.com", 2, 4, 0)],
            "the old address merges into the mailmapped one and shows the newest name"
        );
        assert_eq!(insights.authors[0].first, BASE);
        assert_eq!(insights.authors[0].last, BASE + 23 * DAY);

        let files: Vec<(&str, usize, u64)> = insights.files.iter().map(|file| (file.path.as_str(), file.commits, file.lines)).collect();
        assert_eq!(
            files,
            vec![
                ("src/lib.rs", 2, 5),
                // The rename's commit and a.txt's first commit count under the new name.
                ("docs/a.md", 2, 2),
                ("README.md", 1, 1),
                ("notes.txt", 1, 1),
                ("bin.dat", 1, 0),
            ]
        );

        let folders: Vec<(&str, usize)> = insights.folders.iter().map(|folder| (folder.name.as_str(), folder.commits)).collect();
        assert_eq!(folders, vec![(ROOT_FOLDER, 4), ("src", 2), ("docs", 1)]);
    }

    #[test]
    fn the_period_filter_leaves_out_older_commits() {
        let (dir, client) = repo();
        let now = now_seconds();
        // Git's date limit stops the walk at the first commit older than it, so the old commit comes
        // first and the recent one is the head.
        commit_as(&client, dir.path(), ("Bob", "bob@example.com"), now - 100 * DAY, &[("old.txt", "old\n")]);
        commit_as(&client, dir.path(), ("Ann", "ann@example.com"), now - 5 * DAY, &[("recent.txt", "new\n")]);
        let cancel = AtomicBool::new(false);

        let thirty = InsightsRange { period: InsightsPeriod::Days30, all_branches: false };
        let insights = client.insights(dir.path(), thirty, &cancel).expect("30 days");
        assert_eq!(insights.commits, 1, "only the commit five days old is in 30 days");
        assert_eq!(insights.authors.len(), 1);
        assert_eq!(insights.granularity, Granularity::Day);
        assert_eq!(insights.series.iter().map(|bucket| bucket.commits).sum::<usize>(), 1);
        assert_eq!(insights.series.len(), 31, "one bucket per day from 30 days ago to today, both ends included");

        let year = InsightsRange { period: InsightsPeriod::Year, all_branches: false };
        assert_eq!(client.insights(dir.path(), year, &cancel).expect("one year").commits, 2);
        assert_eq!(client.insights(dir.path(), all_time(false), &cancel).expect("all time").commits, 2);
    }

    #[test]
    fn all_branches_reads_commits_that_only_other_branches_have() {
        let (dir, client) = history();
        let path = dir.path();
        client.run(&["switch", "-q", "-c", "side"], path).expect("new branch");
        commit_as(&client, path, ("Carl", "carl@example.com"), BASE + 30 * DAY, &[("side.txt", "s\n")]);
        client.run(&["switch", "-q", "-"], path).expect("back to the first branch");
        let cancel = AtomicBool::new(false);

        assert_eq!(client.insights(path, all_time(false), &cancel).expect("this branch").commits, 5);
        let every = client.insights(path, all_time(true), &cancel).expect("every branch");
        assert_eq!(every.commits, 6);
        assert!(every.authors.iter().any(|author| author.email == "carl@example.com" && author.commits == 1));
    }

    #[test]
    fn a_repository_without_commits_has_no_history() {
        let (dir, client) = repo();
        let cancel = AtomicBool::new(false);
        let insights = client.insights(dir.path(), all_time(false), &cancel).expect("empty repository");
        assert_eq!(insights, Insights::default());
        let year = InsightsRange { period: InsightsPeriod::Year, all_branches: false };
        let recent = client.insights(dir.path(), year, &cancel).expect("empty repository, one year");
        assert_eq!(recent.commits, 0);
        assert_eq!(recent.series.iter().map(|bucket| bucket.commits).sum::<usize>(), 0);
    }

    #[test]
    fn a_cancelled_read_returns_an_error() {
        let (dir, client) = history();
        let cancel = AtomicBool::new(true);
        assert!(client.insights(dir.path(), all_time(false), &cancel).is_err());
    }

    #[cfg(not(windows))]
    #[test]
    fn a_path_with_a_newline_is_listed_as_written() {
        let (dir, client) = repo();
        let cancel = AtomicBool::new(false);
        commit_as(&client, dir.path(), ("Ann", "ann@example.com"), BASE, &[("odd\nname.txt", "one\ntwo\n")]);
        let insights = client.insights(dir.path(), all_time(false), &cancel).expect("insights");
        let files: Vec<(&str, usize, u64)> = insights.files.iter().map(|file| (file.path.as_str(), file.commits, file.lines)).collect();
        assert_eq!(files, vec![("odd\nname.txt", 1, 2)]);
    }

    #[test]
    fn civil_dates_follow_the_calendar() {
        assert_eq!(civil_date(0), (1970, 1, 1));
        assert_eq!(civil_date(BASE.div_euclid(DAY)), (2023, 11, 14));
        assert_eq!(civil_date(-1), (1969, 12, 31));
        assert_eq!(civil_date(11_016), (2000, 2, 29), "leap day 2000");
    }

    #[test]
    fn weeks_start_on_monday() {
        // 2023-11-13 was a Monday, and 2023-11-19 the Sunday that ends its week.
        let monday = BASE.div_euclid(DAY) - 1;
        assert_eq!(bucket_key(monday, Granularity::Week), monday);
        assert_eq!(bucket_key(monday + 6, Granularity::Week), monday);
        assert_eq!(bucket_key(monday + 7, Granularity::Week), monday + 7);
        assert_eq!(bucket_key(0, Granularity::Week), -3, "1970-01-01 was a Thursday, so its week began on 1969-12-29");
        assert_eq!(bucket_key(42, Granularity::Day), 42);
    }
}
