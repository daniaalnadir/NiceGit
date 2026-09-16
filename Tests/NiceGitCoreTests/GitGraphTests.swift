import NiceGitCore
import Testing

private func commit(_ hash: String, parents: [String] = []) -> GitCommit {
    GitCommit(hash: hash, shortHash: hash, parents: parents, refs: [], subject: hash, authorName: "Test", authorEmail: "", relativeDate: "")
}

@Test func workingTreeConnectsToHeadEvenWhenOtherBranchIsNewer() {
    let history = [commit("other", parents: ["root"]), commit("head", parents: ["root"]), commit("root")]
    let rows = GitGraph.layoutWithWorkingTree(history, headHash: "head")
    #expect(rows.count == 4)
    #expect(rows[0].segments.count == 1)
    #expect(rows[1].lane == 1)
    #expect(rows[1].segments.contains { $0.fromLane == 0 && $0.toLane == 0 && !$0.endsAtNode })
    #expect(rows[2].lane == 0)
    #expect(rows[2].segments.contains { $0.fromLane == 0 && $0.endsAtNode })
    #expect(GitGraph.layoutWithWorkingTree([], headHash: nil)[0].segments.isEmpty)
}

@Test func linearHistoryConnectsParentsAndStopsAtRoot() {
    let rows = GitGraph.layout([commit("c", parents: ["b"]), commit("b", parents: ["a"]), commit("a")])
    #expect(rows.map(\.lane) == [0, 0, 0])
    #expect(rows[0].segments.count == 1)
    #expect(rows[1].segments.count == 2)
    #expect(rows[2].segments.count == 1)
    #expect(rows[2].segments.allSatisfy { $0.endsAtNode })
}

@Test func mergeHistorySplitsAndRejoinsAtSharedAncestor() {
    let rows = GitGraph.layout([
        commit("merge", parents: ["left", "right"]),
        commit("left", parents: ["root"]),
        commit("right", parents: ["root"]),
        commit("root")
    ])
    #expect(rows.map(\.lane) == [0, 0, 1, 0])
    #expect(rows[0].segments.map(\.toLane) == [0, 1])
    #expect(rows[1].segments.contains { !$0.startsAtNode && !$0.endsAtNode && $0.fromLane == 1 && $0.toLane == 1 })
    #expect(rows[2].segments.contains { $0.startsAtNode && $0.fromLane == 1 && $0.toLane == 0 })
    #expect(rows[3].segments.allSatisfy { $0.endsAtNode })
}

@Test func independentRootsHaveNoInventedEdges() {
    let rows = GitGraph.layout([commit("one"), commit("two")])
    #expect(rows.allSatisfy { $0.segments.isEmpty })
}

@Test func separateTipsConvergeWithoutDuplicatingParentLane() {
    let rows = GitGraph.layout([commit("one", parents: ["root"]), commit("two", parents: ["root"]), commit("root")])
    #expect(rows.map(\.lane) == [0, 1, 0])
    #expect(rows[1].segments.contains { $0.startsAtNode && $0.toLane == 0 })
    #expect(rows[2].laneCount == 1)
}
