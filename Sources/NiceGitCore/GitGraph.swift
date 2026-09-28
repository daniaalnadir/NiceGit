public struct GitGraphSegment: Equatable, Sendable {
    public let fromLane: Int
    public let toLane: Int
    public let startsAtNode: Bool
    public let endsAtNode: Bool

    /// A parent connection keeps the colour of the lane it joins below this row.
    public var colorLane: Int { startsAtNode ? toLane : fromLane }
}

public struct GitGraphRow: Equatable, Sendable {
    public let lane: Int
    public let laneCount: Int
    public let segments: [GitGraphSegment]
}

public enum GitGraph {
    public static func layoutWithWorkingTree(_ commits: [GitCommit], headHash: String?) -> [GitGraphRow] {
        let workingTree = GitCommit(hash: "WORKING_TREE", shortHash: "", parents: headHash.map { [$0] } ?? [], refs: [], subject: "Working tree", authorName: "", authorEmail: "", relativeDate: "")
        return layout([workingTree] + commits)
    }

    /// Tracks pending parents through Git's topological commit order.
    public static func layout(_ commits: [GitCommit]) -> [GitGraphRow] {
        var lanes: [String?] = []
        return commits.map { commit in
            let incoming = lanes
            let lane: Int
            if let existing = lanes.firstIndex(of: commit.hash) {
                lane = existing
            } else {
                lane = lanes.firstIndex(of: nil) ?? lanes.count
                if lane == lanes.count { lanes.append(commit.hash) }
                else { lanes[lane] = commit.hash }
            }
            let widthBefore = lanes.count
            lanes[lane] = nil
            for (index, parent) in commit.parents.enumerated() where !lanes.contains(parent) {
                // Continue the first parent straight down. Ending another branch must
                // never shift a still-active line into a different lane or colour.
                let destination = index == 0 ? lane : (lanes.firstIndex(of: nil) ?? lanes.count)
                if destination == lanes.count { lanes.append(parent) }
                else { lanes[destination] = parent }
            }
            while lanes.last == .some(nil) { lanes.removeLast() }
            var segments: [GitGraphSegment] = []
            for (index, hash) in incoming.enumerated() {
                guard let hash else { continue }
                if hash == commit.hash {
                    segments.append(.init(fromLane: index, toLane: lane, startsAtNode: false, endsAtNode: true))
                } else if let destination = lanes.firstIndex(of: hash) {
                    segments.append(.init(fromLane: index, toLane: destination, startsAtNode: false, endsAtNode: false))
                }
            }
            for parent in commit.parents {
                if let destination = lanes.firstIndex(of: parent) {
                    segments.append(.init(fromLane: lane, toLane: destination, startsAtNode: true, endsAtNode: false))
                }
            }
            return GitGraphRow(lane: lane, laneCount: max(widthBefore, lanes.count), segments: segments)
        }
    }
}
