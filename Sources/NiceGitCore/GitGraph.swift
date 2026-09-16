public struct GitGraphSegment: Equatable, Sendable {
    public let fromLane: Int
    public let toLane: Int
    public let startsAtNode: Bool
    public let endsAtNode: Bool
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
        var lanes: [String] = []
        return commits.map { commit in
            let incoming = lanes
            let lane: Int
            if let existing = lanes.firstIndex(of: commit.hash) {
                lane = existing
            } else {
                lane = lanes.count
                lanes.append(commit.hash)
            }
            let widthBefore = lanes.count
            lanes.remove(at: lane)
            for parent in commit.parents.reversed() where !lanes.contains(parent) {
                lanes.insert(parent, at: min(lane, lanes.count))
            }
            var segments: [GitGraphSegment] = []
            for (index, hash) in incoming.enumerated() {
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
