use std::collections::{BTreeMap, HashMap, HashSet};

use egui::epaint::{CubicBezierShape, PathStroke};
use egui::{Color32, Painter, Pos2, Rect, Stroke};
use nicegit_core::graph::{GraphRow, GraphSegment};
use nicegit_core::Stash;

use crate::theme::graph_color as color;

pub const LANE_WIDTH: f32 = 20.0;
const LEFT_PADDING: f32 = 8.0;
pub const NODE_RADIUS: f32 = 11.0;
/// The radius of a stash's hollow node, smaller than a commit's.
const STASH_RADIUS: f32 = 6.0;

/// How a row's node is drawn.
pub enum Node<'a> {
    /// A commit, with its author's initials.
    Commit { initials: &'a str, merge: bool },
    /// Uncommitted work: a dashed ring.
    WorkingTree,
    /// A stash: a small hollow node with a dashed ring.
    Stash,
}

/// One row of the history list: a row of the layout, or a stash drawn above its base commit.
#[derive(Clone, Debug, PartialEq)]
pub enum Entry {
    /// A row of the layout, by its index.
    Commit(usize),
    Stash(StashRow),
}

/// A stash's own row in the graph.
#[derive(Clone, Debug, PartialEq)]
pub struct StashRow {
    /// The stash's index in the snapshot's stashes.
    pub stash: usize,
    /// The lane of the stash's node: the first lane right of the base commit's lane that no line
    /// passes through this row. Other lanes never move for it.
    pub lane: usize,
    /// The lane the dashed line from the node reaches at the row's bottom edge: the next stash's
    /// node when another stash shares the base, otherwise the base commit's lane.
    pub toward: usize,
    /// The colour of the base commit's lane.
    pub color: usize,
    /// The lines that continue through this row, one per lane, drawn straight down.
    pub passing: Vec<GraphSegment>,
}

pub fn width_for(lane_count: usize) -> f32 {
    LEFT_PADDING * 2.0 + LANE_WIDTH * lane_count.saturating_sub(1) as f32 + NODE_RADIUS * 2.0
}

pub fn lane_x(rect: Rect, lane: usize) -> f32 {
    rect.left() + LEFT_PADDING + NODE_RADIUS + lane as f32 * LANE_WIDTH
}

/// Lines the history list in order, with each stash drawn as a row directly above its base
/// commit's row. A stash whose base is not among `rows` is left out. Stashes sharing a base keep
/// the order given, so stash@{0} comes first.
///
/// `hash_at` names the commit of each row, or gives `None` for the working tree row. The rows
/// themselves are not changed, so the lanes of the commits stay where the layout put them.
pub fn with_stashes<'a>(rows: &[GraphRow], hash_at: impl Fn(usize) -> Option<&'a str>, stashes: &[Stash]) -> Vec<Entry> {
    // Without stashes, which is the default, the rows are listed as they are.
    if stashes.is_empty() {
        return (0..rows.len()).map(Entry::Commit).collect();
    }
    let mut row_of: HashMap<&str, usize> = HashMap::with_capacity(rows.len());
    for index in 0..rows.len() {
        if let Some(hash) = hash_at(index) {
            row_of.entry(hash).or_insert(index);
        }
    }
    let mut above: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for (index, stash) in stashes.iter().enumerate() {
        if let Some(&row) = stash.base.as_deref().and_then(|base| row_of.get(base)) {
            above.entry(row).or_default().push(index);
        }
    }
    let mut entries = Vec::with_capacity(rows.len() + stashes.len());
    for row in 0..rows.len() {
        if let Some(group) = above.get(&row) {
            let passing = crossing_lines(rows, row);
            let taken: HashSet<usize> = passing.iter().map(|segment| segment.to_lane).collect();
            let lane = (rows[row].lane + 1..).find(|lane| !taken.contains(lane)).unwrap_or(rows[row].lane + 1);
            for (position, &stash) in group.iter().enumerate() {
                let toward = if position + 1 < group.len() { lane } else { rows[row].lane };
                entries.push(Entry::Stash(StashRow { stash, lane, toward, color: rows[row].color, passing: passing.clone() }));
            }
        }
        entries.push(Entry::Commit(row));
    }
    entries
}

/// The lines that cross from the row above `row` into it, one per lane, as straight segments.
/// They are the lines leaving the row above at its bottom edge.
fn crossing_lines(rows: &[GraphRow], row: usize) -> Vec<GraphSegment> {
    let Some(above) = row.checked_sub(1).and_then(|index| rows.get(index)) else { return Vec::new() };
    let mut lanes: BTreeMap<usize, GraphSegment> = BTreeMap::new();
    for segment in above.segments.iter().filter(|segment| !segment.ends_at_node) {
        lanes.entry(segment.to_lane).or_insert(GraphSegment {
            from_lane: segment.to_lane,
            to_lane: segment.to_lane,
            starts_at_node: false,
            ends_at_node: false,
            color: segment.color,
            line: segment.line,
            from_line: segment.line,
        });
    }
    lanes.into_values().collect()
}

/// The lane count the stash nodes need, or zero when no stash row is drawn. The graph is widened
/// to this when it is larger than the commits need.
pub fn stash_lanes(entries: &[Entry]) -> usize {
    entries
        .iter()
        .filter_map(|entry| match entry {
            Entry::Stash(row) => Some(row.lane + 1),
            Entry::Commit(_) => None,
        })
        .max()
        .unwrap_or(0)
}

/// Paints a stash's row: the lines passing through it, a dashed line from its hollow node down to
/// the lane it leads to, and the node itself.
pub fn paint_stash_row(painter: &Painter, rect: Rect, row: &StashRow, background: Color32) {
    let from = Pos2::new(lane_x(rect, row.lane), rect.center().y);
    let to = Pos2::new(lane_x(rect, row.toward), rect.bottom());
    dashed_line(painter, from, to, Stroke::new(1.5, color(row.color)));
    let as_row = GraphRow { lane: row.lane, lane_count: row.lane + 1, segments: row.passing.clone(), color: row.color, line: 0 };
    paint_row(painter, rect, &as_row, Node::Stash, background);
}

/// A line drawn as short dashes from `from` to `to`.
fn dashed_line(painter: &Painter, from: Pos2, to: Pos2, stroke: Stroke) {
    const DASH: f32 = 3.0;
    const GAP: f32 = 2.5;
    let length = (to - from).length();
    if length <= 0.0 {
        return;
    }
    let direction = (to - from) / length;
    let mut along = 0.0;
    while along < length {
        let end = (along + DASH).min(length);
        painter.line_segment([from + direction * along, from + direction * end], stroke);
        along += DASH + GAP;
    }
}

/// A ring of short arcs around `center`.
fn dashed_ring(painter: &Painter, center: Pos2, radius: f32, stroke: Stroke) {
    let dashes = 12;
    for i in 0..dashes {
        let a0 = i as f32 / dashes as f32 * std::f32::consts::TAU;
        let a1 = a0 + std::f32::consts::TAU / dashes as f32 * 0.55;
        let p0 = center + egui::vec2(a0.cos(), a0.sin()) * radius;
        let p1 = center + egui::vec2(a1.cos(), a1.sin()) * radius;
        painter.line_segment([p0, p1], stroke);
    }
}

/// Initials for an author name, such as "AP" for "Ada Park".
pub fn initials(name: &str) -> String {
    let mut letters = name.split_whitespace().filter_map(|word| word.chars().find(|c| c.is_alphanumeric()));
    let first = letters.next();
    let last = letters.next_back();
    [first, last].into_iter().flatten().flat_map(char::to_uppercase).collect()
}

/// Paints one row: lines entering from the row above, lines leaving towards the row below,
/// and the node.
pub fn paint_row(painter: &Painter, rect: Rect, row: &GraphRow, node: Node, background: Color32) {
    let (top, middle, bottom) = (rect.top(), rect.center().y, rect.bottom());
    for segment in &row.segments {
        let start = Pos2::new(lane_x(rect, segment.from_lane), if segment.starts_at_node { middle } else { top });
        let end = Pos2::new(lane_x(rect, segment.to_lane), if segment.ends_at_node { middle } else { bottom });
        let stroke = Stroke::new(2.0, color(segment.color));
        if segment.from_lane == segment.to_lane {
            painter.line_segment([start, end], stroke);
        } else {
            // Bend with vertical tangents, so lines leave and enter lanes smoothly.
            let bend = (end.y - start.y) / 2.0;
            let points = [start, Pos2::new(start.x, start.y + bend), Pos2::new(end.x, end.y - bend), end];
            painter.add(CubicBezierShape::from_points_stroke(points, false, Color32::TRANSPARENT, PathStroke::from(stroke)));
        }
    }
    let center = Pos2::new(lane_x(rect, row.lane), middle);
    let node_color = color(row.color);
    match node {
        Node::WorkingTree => {
            painter.circle_filled(center, NODE_RADIUS, background);
            dashed_ring(painter, center, NODE_RADIUS - 1.0, Stroke::new(2.0, node_color));
        }
        Node::Stash => {
            painter.circle_filled(center, STASH_RADIUS, background);
            dashed_ring(painter, center, STASH_RADIUS, Stroke::new(1.5, node_color));
        }
        Node::Commit { merge: true, .. } => {
            painter.circle(center, 4.5, node_color, Stroke::new(2.0, background));
        }
        Node::Commit { initials, .. } => {
            painter.circle(center, NODE_RADIUS, node_color, Stroke::new(2.0, background));
            painter.text(center, egui::Align2::CENTER_CENTER, initials, crate::theme::strong(9.5), Color32::WHITE);
        }
    }
}

#[cfg(test)]
mod tests {
    use nicegit_core::graph::layout;
    use nicegit_core::Commit;

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

    /// Stash `index`, made on `base`, as Git lists it: stash@{index}.
    fn stash(index: usize, base: Option<&str>) -> Stash {
        Stash {
            hash: format!("stash-{index}"),
            reference: format!("stash@{{{index}}}"),
            message: format!("Work {index}"),
            base: base.map(str::to_string),
        }
    }

    /// Lays out `commits` and lists them with `stashes`, the way the history does.
    fn list(commits: &[Commit], pinned: &str, stashes: &[Stash]) -> (Vec<GraphRow>, Vec<Entry>) {
        let rows = layout(commits, Some(pinned), 8);
        let entries = with_stashes(&rows, |index| commits.get(index).map(|c| c.hash.as_str()), stashes);
        (rows, entries)
    }

    #[test]
    fn a_stash_is_listed_directly_above_its_base_commit() {
        let commits = [commit("c2", &["c1"]), commit("c1", &["c0"]), commit("c0", &[])];
        let (rows, entries) = list(&commits, "c2", &[stash(0, Some("c1"))]);
        let [Entry::Commit(0), Entry::Stash(row), Entry::Commit(1), Entry::Commit(2)] = entries.as_slice() else {
            panic!("stash above its base, got {entries:?}");
        };
        assert_eq!(row.stash, 0);
        assert_eq!(row.toward, rows[1].lane, "the dashed line leads to the base's lane");
        assert_eq!(row.color, rows[1].color, "the stash takes its base's colour");
        assert!(row.lane > rows[1].lane, "the node sits beside the base's lane");
    }

    #[test]
    fn stashes_sharing_a_base_are_listed_newest_first_and_chained() {
        let commits = [commit("c2", &["c1"]), commit("c1", &["c0"]), commit("c0", &[])];
        let (rows, entries) = list(&commits, "c2", &[stash(0, Some("c1")), stash(1, Some("c1"))]);
        let [Entry::Commit(0), Entry::Stash(first), Entry::Stash(second), Entry::Commit(1), Entry::Commit(2)] = entries.as_slice() else {
            panic!("two stashes above one base, got {entries:?}");
        };
        assert_eq!((first.stash, second.stash), (0, 1), "stash@{{0}} is the upper row");
        assert_eq!(first.lane, second.lane, "stashes on one base share a lane");
        assert_eq!(first.toward, first.lane, "the upper node leads down to the lower one");
        assert_eq!(second.toward, rows[1].lane, "the lower node leads to the base");
    }

    #[test]
    fn a_stash_whose_base_is_not_loaded_is_not_listed() {
        let commits = [commit("c2", &["c1"]), commit("c1", &[])];
        let (_, entries) = list(&commits, "c2", &[stash(0, Some("elsewhere")), stash(1, None)]);
        assert_eq!(entries, vec![Entry::Commit(0), Entry::Commit(1)]);
        let (_, none) = list(&commits, "c2", &[]);
        assert_eq!(none, vec![Entry::Commit(0), Entry::Commit(1)]);
    }

    #[test]
    fn a_stash_passes_the_lanes_it_crosses_and_takes_a_free_lane_beside_its_base() {
        // m merges f into b; f and b both branch from a. Lane 0 is the checkout's line.
        let commits = [commit("m", &["b", "f"]), commit("f", &["a"]), commit("b", &["a"]), commit("a", &[])];
        let before = layout(&commits, Some("m"), 8);
        let (rows, entries) = list(&commits, "m", &[stash(0, Some("b"))]);
        assert_eq!(rows, before, "listing the stash leaves the commit rows as they were");
        let row = entries
            .iter()
            .find_map(|entry| match entry {
                Entry::Stash(row) => Some(row),
                Entry::Commit(_) => None,
            })
            .expect("the stash is listed");
        let crossing: Vec<usize> = row.passing.iter().map(|segment| segment.to_lane).collect();
        assert_eq!(crossing, vec![0, 1], "the lines into b and f pass straight through");
        assert!(row.passing.iter().all(|segment| !segment.starts_at_node && !segment.ends_at_node && segment.from_lane == segment.to_lane));
        assert_eq!(row.lane, 2, "the node skips the lanes the passing lines hold");
        assert_eq!(row.toward, rows[2].lane);
    }

    #[test]
    fn stash_lanes_counts_the_lanes_its_nodes_need() {
        let commits = [commit("c1", &["c0"]), commit("c0", &[])];
        let (_, none) = list(&commits, "c1", &[]);
        assert_eq!(stash_lanes(&none), 0);
        let (_, one) = list(&commits, "c1", &[stash(0, Some("c0"))]);
        assert_eq!(stash_lanes(&one), 2, "a stash beside lane 0 needs lane 1");
    }

    #[test]
    fn initials_use_first_and_last_words() {
        assert_eq!(initials("Ada Park"), "AP");
        assert_eq!(initials("lee van chen"), "LC");
        assert_eq!(initials("Prince"), "P");
        assert_eq!(initials(""), "");
    }
}
