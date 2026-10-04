public struct GitGraphSegment: Equatable, Sendable {
    public let fromLane: Int
    public let toLane: Int
    public let startsAtNode: Bool
    public let endsAtNode: Bool
    /// A parent connection keeps the colour of the line it joins below this row.
    public let color: Int
    /// The line this segment belongs to below the row, and the line it leaves above or at the node.
    public let line: Int
    public let fromLine: Int
}

public struct GitGraphRow: Equatable, Sendable {
    public let lane: Int
    public let laneCount: Int
    public let segments: [GitGraphSegment]
    public let color: Int
    /// Identifies the first-parent line the commit sits on, stable across rows.
    public let line: Int
}

public enum GitGraph {
    public static let workingTreeHash = "WORKING_TREE"

    public static func layoutWithWorkingTree(_ commits: [GitCommit], headHash: String?, colorCount: Int = 8) -> [GitGraphRow] {
        let workingTree = GitCommit(hash: workingTreeHash, shortHash: "", parents: headHash.map { [$0] } ?? [], refs: [], subject: "Working tree", authorName: "", authorEmail: "", relativeDate: "")
        return layout([workingTree] + commits, pinning: workingTreeHash, colorCount: colorCount)
    }

    private struct Line: Equatable {
        let hash: String
        let id: Int
        let color: Int
    }

    /// Tracks pending parents through Git's topological commit order. The pinned commit's
    /// first-parent chain always occupies the leftmost lane in colour 0, and every other
    /// line keeps one colour from its tip until it ends.
    public static func layout(_ commits: [GitCommit], pinning pinnedHash: String? = nil, colorCount: Int = 8) -> [GitGraphRow] {
        let palette = max(1, colorCount)
        let positions = Dictionary(commits.enumerated().map { ($0.element.hash, $0.offset) }, uniquingKeysWith: { first, _ in first })
        let byHash = Dictionary(commits.map { ($0.hash, $0) }, uniquingKeysWith: { first, _ in first })
        // Follow first parents strictly downward; HEAD appended after its page can have
        // ancestors listed above it, and those must not be pulled into the pinned lane.
        var mainline = Set<String>()
        var cursor = pinnedHash.flatMap { byHash[$0] }
        while let commit = cursor, let position = positions[commit.hash], mainline.insert(commit.hash).inserted {
            cursor = commit.parents.first.flatMap { parent in
                positions[parent].flatMap { $0 > position ? byHash[parent] : nil }
            }
        }
        // A lone pinned commit is either first already or appended below its page;
        // reserving a lane for it would only leave an empty column above.
        if mainline.count < 2 { mainline.removeAll() }
        let mainLineID = 0
        var nextLineID = 1
        var nextColor = 0
        var mainPending = !mainline.isEmpty
        var lanes: [Line?] = []

        func laneIndex(of hash: String) -> Int? { lanes.firstIndex { $0?.hash == hash } }

        func freeLane() -> Int {
            // Keep the leftmost lane for the pinned line until it starts.
            if mainPending && lanes.isEmpty { lanes.append(nil) }
            if let free = lanes.indices.first(where: { lanes[$0] == nil && !(mainPending && $0 == 0) }) { return free }
            lanes.append(nil)
            return lanes.count - 1
        }

        func newLine(_ hash: String, at index: Int) -> Line {
            // Colour 0 belongs to the pinned line; prefer colours no active line uses.
            let reserved = mainline.isEmpty ? 0 : 1
            let choices = palette - reserved
            var color = reserved
            if choices > 0 {
                let active = Set(lanes.compactMap { $0?.color })
                let neighbours = Set([index - 1, index + 1].compactMap { lanes.indices.contains($0) ? lanes[$0]?.color : nil })
                let candidates = (0..<choices).map { reserved + (nextColor + $0) % choices }
                color = candidates.first(where: { !active.contains($0) }) ?? candidates.first(where: { !neighbours.contains($0) }) ?? candidates[0]
                nextColor = (color - reserved + 1) % choices
            }
            defer { nextLineID += 1 }
            return Line(hash: hash, id: nextLineID, color: color)
        }

        return commits.map { commit -> GitGraphRow in
            let incoming = lanes
            let isMain = mainline.contains(commit.hash)
            let lane: Int
            let node: Line
            if isMain {
                lane = 0
                if let existing = laneIndex(of: commit.hash), existing != 0 { lanes[existing] = nil }
                if lanes.isEmpty { lanes.append(nil) }
                node = Line(hash: commit.hash, id: mainLineID, color: 0)
                mainPending = false
            } else if let existing = laneIndex(of: commit.hash), let line = lanes[existing] {
                lane = existing
                node = line
            } else {
                lane = freeLane()
                node = newLine(commit.hash, at: lane)
            }
            lanes[lane] = node
            let widthBefore = lanes.count
            lanes[lane] = nil
            for (index, parent) in commit.parents.enumerated() {
                if index == 0 && isMain {
                    // Draw a branch that already reached this parent into the pinned lane.
                    if let existing = laneIndex(of: parent) { lanes[existing] = nil }
                    lanes[0] = Line(hash: parent, id: mainLineID, color: 0)
                } else if laneIndex(of: parent) == nil {
                    // Continue the first parent straight down. Ending another branch must
                    // never shift a still-active line into a different lane or colour.
                    if index == 0 {
                        lanes[lane] = Line(hash: parent, id: node.id, color: node.color)
                    } else {
                        let destination = freeLane()
                        lanes[destination] = newLine(parent, at: destination)
                    }
                }
            }
            while lanes.last == .some(nil) { lanes.removeLast() }
            var segments: [GitGraphSegment] = []
            for (index, line) in incoming.enumerated() {
                guard let line else { continue }
                if line.hash == commit.hash {
                    segments.append(.init(fromLane: index, toLane: lane, startsAtNode: false, endsAtNode: true,
                                          color: line.color, line: line.id, fromLine: line.id))
                } else if let destination = laneIndex(of: line.hash), let target = lanes[destination] {
                    segments.append(.init(fromLane: index, toLane: destination, startsAtNode: false, endsAtNode: false,
                                          color: target.color, line: target.id, fromLine: line.id))
                }
            }
            for parent in commit.parents {
                if let destination = laneIndex(of: parent), let target = lanes[destination] {
                    segments.append(.init(fromLane: lane, toLane: destination, startsAtNode: true, endsAtNode: false,
                                          color: target.color, line: target.id, fromLine: node.id))
                }
            }
            return GitGraphRow(lane: lane, laneCount: max(widthBefore, lanes.count), segments: segments,
                               color: node.color, line: node.id)
        }
    }
}
