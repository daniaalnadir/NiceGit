use std::collections::{HashMap, HashSet};

use crate::models::Commit;

/// One line drawn through a graph row. Lines start at the row's top edge, or at its node when
/// `starts_at_node`, and end at its bottom edge, or at its node when `ends_at_node`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GraphSegment {
    pub from_lane: usize,
    pub to_lane: usize,
    pub starts_at_node: bool,
    pub ends_at_node: bool,
    pub color: usize,
    pub line: usize,
    pub from_line: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GraphRow {
    pub lane: usize,
    pub lane_count: usize,
    pub segments: Vec<GraphSegment>,
    pub color: usize,
    pub line: usize,
}

pub const WORKING_TREE_HASH: &str = "WORKING_TREE";

#[derive(Clone, Debug, PartialEq, Eq)]
struct Line {
    hash: String,
    id: usize,
    color: usize,
}

/// Lays out `commits` with an extra first row for uncommitted work, joined to HEAD.
pub fn layout_with_working_tree(commits: &[Commit], head_hash: Option<&str>, color_count: usize) -> Vec<GraphRow> {
    let working_tree = Commit {
        hash: WORKING_TREE_HASH.to_string(),
        short_hash: String::new(),
        parents: head_hash.map(|hash| vec![hash.to_string()]).unwrap_or_default(),
        refs: Vec::new(),
        subject: "Working tree".to_string(),
        author_name: String::new(),
        author_email: String::new(),
        relative_date: String::new(),
        commit_time: None,
    };
    let mut all = Vec::with_capacity(commits.len() + 1);
    all.push(working_tree);
    all.extend_from_slice(commits);
    layout(&all, Some(WORKING_TREE_HASH), color_count)
}

/// Assigns each commit (in topological order, newest first) a lane, and lists the line
/// segments crossing its row. The pinned commit's first-parent line stays in lane 0 with
/// colour 0; every other line keeps its lane and colour from tip to end.
pub fn layout(commits: &[Commit], pinned: Option<&str>, color_count: usize) -> Vec<GraphRow> {
    let palette = color_count.max(1);
    let mut positions: HashMap<&str, usize> = HashMap::with_capacity(commits.len());
    for (offset, commit) in commits.iter().enumerate() {
        positions.entry(commit.hash.as_str()).or_insert(offset);
    }
    let by_hash = |hash: &str| positions.get(hash).map(|&index| &commits[index]);

    let mut mainline: HashSet<String> = HashSet::new();
    let mut cursor = pinned.and_then(by_hash);
    while let Some(commit) = cursor {
        let Some(&position) = positions.get(commit.hash.as_str()) else { break };
        if !mainline.insert(commit.hash.clone()) {
            break;
        }
        cursor = commit.parents.first().and_then(|parent| positions.get(parent.as_str()).filter(|&&p| p > position).map(|&p| &commits[p]));
    }
    if mainline.len() < 2 {
        mainline.clear();
    }

    const MAIN_LINE_ID: usize = 0;
    let mut next_line_id = 1usize;
    let mut next_color = 0usize;
    let mut main_pending = !mainline.is_empty();
    let mut lanes: Vec<Option<Line>> = Vec::new();

    fn lane_index(lanes: &[Option<Line>], hash: &str) -> Option<usize> {
        lanes.iter().position(|line| line.as_ref().is_some_and(|line| line.hash == hash))
    }

    fn free_lane(lanes: &mut Vec<Option<Line>>, main_pending: bool) -> usize {
        if main_pending && lanes.is_empty() {
            lanes.push(None);
        }
        if let Some(free) = (0..lanes.len()).find(|&i| lanes[i].is_none() && !(main_pending && i == 0)) {
            return free;
        }
        lanes.push(None);
        lanes.len() - 1
    }

    let reserved = if mainline.is_empty() { 0 } else { 1 };
    let mut new_line = |lanes: &[Option<Line>], hash: &str, index: usize| -> Line {
        let choices = palette.saturating_sub(reserved);
        let mut color = reserved;
        if choices > 0 {
            let active: HashSet<usize> = lanes.iter().flatten().map(|line| line.color).collect();
            let neighbours: HashSet<usize> = [index.checked_sub(1), Some(index + 1)]
                .into_iter()
                .flatten()
                .filter_map(|i| lanes.get(i).and_then(|line| line.as_ref()).map(|line| line.color))
                .collect();
            let candidates: Vec<usize> = (0..choices).map(|offset| reserved + (next_color + offset) % choices).collect();
            color = candidates
                .iter()
                .copied()
                .find(|c| !active.contains(c))
                .or_else(|| candidates.iter().copied().find(|c| !neighbours.contains(c)))
                .unwrap_or(candidates[0]);
            next_color = (color - reserved + 1) % choices;
        }
        let line = Line { hash: hash.to_string(), id: next_line_id, color };
        next_line_id += 1;
        line
    };

    commits
        .iter()
        .map(|commit| {
            let incoming = lanes.clone();
            let is_main = mainline.contains(&commit.hash);
            let (lane, node) = if is_main {
                if let Some(existing) = lane_index(&lanes, &commit.hash) {
                    if existing != 0 {
                        lanes[existing] = None;
                    }
                }
                if lanes.is_empty() {
                    lanes.push(None);
                }
                main_pending = false;
                (0, Line { hash: commit.hash.clone(), id: MAIN_LINE_ID, color: 0 })
            } else if let Some(existing) = lane_index(&lanes, &commit.hash) {
                (existing, lanes[existing].clone().expect("lane holds a line"))
            } else {
                let lane = free_lane(&mut lanes, main_pending);
                let node = new_line(&lanes, &commit.hash, lane);
                (lane, node)
            };
            lanes[lane] = Some(node.clone());
            let width_before = lanes.len();
            lanes[lane] = None;
            for (index, parent) in commit.parents.iter().enumerate() {
                if index == 0 && is_main {
                    if let Some(existing) = lane_index(&lanes, parent) {
                        lanes[existing] = None;
                    }
                    lanes[0] = Some(Line { hash: parent.clone(), id: MAIN_LINE_ID, color: 0 });
                } else if lane_index(&lanes, parent).is_none() {
                    if index == 0 {
                        lanes[lane] = Some(Line { hash: parent.clone(), id: node.id, color: node.color });
                    } else {
                        let destination = free_lane(&mut lanes, main_pending);
                        let line = new_line(&lanes, parent, destination);
                        lanes[destination] = Some(line);
                    }
                }
            }
            while matches!(lanes.last(), Some(None)) {
                lanes.pop();
            }

            let mut segments = Vec::new();
            for (index, line) in incoming.iter().enumerate() {
                let Some(line) = line else { continue };
                if line.hash == commit.hash {
                    segments.push(GraphSegment {
                        from_lane: index,
                        to_lane: lane,
                        starts_at_node: false,
                        ends_at_node: true,
                        color: line.color,
                        line: line.id,
                        from_line: line.id,
                    });
                } else if let Some(destination) = lane_index(&lanes, &line.hash) {
                    let target = lanes[destination].as_ref().expect("lane holds a line");
                    segments.push(GraphSegment {
                        from_lane: index,
                        to_lane: destination,
                        starts_at_node: false,
                        ends_at_node: false,
                        color: target.color,
                        line: target.id,
                        from_line: line.id,
                    });
                }
            }
            for parent in &commit.parents {
                if let Some(destination) = lane_index(&lanes, parent) {
                    let target = lanes[destination].as_ref().expect("lane holds a line");
                    segments.push(GraphSegment {
                        from_lane: lane,
                        to_lane: destination,
                        starts_at_node: true,
                        ends_at_node: false,
                        color: target.color,
                        line: target.id,
                        from_line: node.id,
                    });
                }
            }
            GraphRow { lane, lane_count: width_before.max(lanes.len()), segments, color: node.color, line: node.id }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn commit(hash: &str, parents: &[&str]) -> Commit {
        Commit {
            hash: hash.into(),
            short_hash: hash.into(),
            parents: parents.iter().map(|p| p.to_string()).collect(),
            refs: vec![],
            subject: hash.into(),
            author_name: String::new(),
            author_email: String::new(),
            relative_date: String::new(),
            commit_time: None,
        }
    }

    #[test]
    fn linear_history_stays_in_one_lane() {
        let rows = layout(&[commit("c", &["b"]), commit("b", &["a"]), commit("a", &[])], None, 8);
        assert!(rows.iter().all(|row| row.lane == 0 && row.lane_count == 1));
        assert_eq!(rows[0].segments.len(), 1);
        assert!(rows[2].segments.iter().all(|segment| segment.ends_at_node));
    }

    #[test]
    fn merge_opens_and_closes_a_second_lane() {
        // m merges f into b; f branches from a.
        let commits = [commit("m", &["b", "f"]), commit("f", &["a"]), commit("b", &["a"]), commit("a", &[])];
        let rows = layout(&commits, Some("m"), 8);
        assert_eq!(rows[0].lane, 0);
        assert_eq!(rows[1].lane, 1);
        assert_eq!(rows[2].lane, 0);
        assert_eq!(rows[3].lane, 0);
        assert_eq!(rows[0].lane_count, 2);
        // The pinned first-parent line keeps colour 0; the side line gets another.
        assert_eq!(rows[0].color, 0);
        assert_ne!(rows[1].color, 0);
        // The checkout line claims a, so the side line bends into lane 0 at b's row.
        assert!(rows[2].segments.iter().any(|s| s.from_lane == 1 && s.to_lane == 0 && !s.ends_at_node));
        assert_eq!(rows[3].lane_count, 1);
    }

    #[test]
    fn checkout_line_is_pinned_to_lane_zero_below_a_newer_branch() {
        // "other" is newer than HEAD but HEAD's line must stay leftmost.
        let commits = [commit("other", &["a"]), commit("head", &["a"]), commit("a", &[])];
        let rows = layout_with_working_tree(&commits, Some("head"), 8);
        assert_eq!(rows[0].lane, 0); // working tree
        assert_eq!(rows[1].lane, 1); // other
        assert_eq!(rows[2].lane, 0); // head
        assert_eq!(rows[3].lane, 0);
        assert_eq!(rows[2].color, 0);
        assert_ne!(rows[1].color, 0);
    }

    #[test]
    fn lanes_stay_stationary_when_another_line_ends() {
        // Three parallel branches; when the middle one ends, the right one keeps its lane.
        let commits =
            [commit("x", &["xa"]), commit("y", &["ya"]), commit("z", &["za"]), commit("ya", &[]), commit("za", &[]), commit("xa", &[])];
        let rows = layout(&commits, None, 8);
        assert_eq!(rows[2].lane, 2);
        assert_eq!(rows[4].lane, 2);
    }
}
