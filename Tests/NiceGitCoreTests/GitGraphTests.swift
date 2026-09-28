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

@Test func endingALaneDoesNotMoveSurvivingBranches() {
    let rows = GitGraph.layout([
        commit("merge", parents: ["short", "long", "other"]),
        commit("short"),
        commit("long", parents: ["older"]),
        commit("other", parents: ["older"]),
        commit("older")
    ])
    #expect(rows.map(\.lane) == [0, 0, 1, 2, 1])
    #expect(rows[1].segments.contains { $0.fromLane == 2 && $0.toLane == 2 && !$0.startsAtNode && !$0.endsAtNode })
    #expect(rows[2].segments.contains { $0.startsAtNode && $0.fromLane == 1 && $0.toLane == 1 })
    // The joining edge and the continuation below use the same colour.
    #expect(rows[3].segments.first { $0.startsAtNode }?.color == 1)
    #expect(rows[4].segments.first { $0.endsAtNode }?.color == 1)
}

@Test func newTipsReuseEmptyLanesWithoutMovingExistingHistory() {
    let rows = GitGraph.layout([
        commit("merge", parents: ["short", "long"]), commit("short"),
        commit("new", parents: ["root"]), commit("long", parents: ["root"]), commit("root")
    ])
    #expect(rows.map(\.lane) == [0, 0, 0, 1, 0])
    #expect(rows[2].segments.contains { $0.fromLane == 1 && $0.toLane == 1 && !$0.startsAtNode })
}

@Test func checkoutStaysInFirstLaneWhenAnotherBranchIsNewer() {
    let rows = GitGraph.layout([commit("other", parents: ["root"]), commit("head", parents: ["root"]), commit("root")], pinning: "head")
    #expect(rows.map(\.lane) == [1, 0, 0])
    #expect(rows[1].color == 0 && rows[2].color == 0)
    #expect(rows[0].color != 0)
    // The newer branch joins the checkout's line below the checkout.
    #expect(rows[1].segments.contains { $0.fromLane == 1 && $0.toLane == 0 && !$0.startsAtNode && !$0.endsAtNode && $0.color == 0 })
}

@Test func branchReachingCheckoutAncestorFirstIsDrawnIntoCheckoutLane() {
    let rows = GitGraph.layout([
        commit("f2", parents: ["f1"]), commit("m3", parents: ["m2"]),
        commit("f1", parents: ["m1"]), commit("m2", parents: ["m1"]), commit("m1")
    ], pinning: "m3")
    #expect(rows.map(\.lane) == [1, 0, 1, 0, 0])
    #expect(rows[0].line == rows[2].line)
    #expect(rows[1].line == rows[3].line && rows[3].line == rows[4].line)
    #expect(rows[3].segments.contains { $0.fromLane == 1 && $0.toLane == 0 && !$0.startsAtNode && $0.fromLine == rows[2].line && $0.line == rows[3].line })
    #expect(rows[4].laneCount == 1)
}

@Test func childOfCheckoutAboveItConnectsIntoFirstLane() {
    let rows = GitGraph.layout([commit("child", parents: ["head"]), commit("head", parents: ["base"]), commit("base")], pinning: "head")
    #expect(rows.map(\.lane) == [1, 0, 0])
    #expect(rows[1].segments.contains { $0.fromLane == 1 && $0.toLane == 0 && $0.endsAtNode && $0.color == rows[0].color })
}

@Test func checkoutAppendedBelowItsPageDoesNotReserveEmptyLane() {
    let rows = GitGraph.layout([commit("a", parents: ["b"]), commit("b"), commit("head", parents: ["a"])], pinning: "head")
    #expect(rows.map(\.lane) == [0, 0, 0])
    #expect(rows.allSatisfy { $0.laneCount == 1 })
}

@Test func coloursFollowBranchesRatherThanLanes() {
    let rows = GitGraph.layout([
        commit("merge", parents: ["short", "long"]), commit("short"),
        commit("new", parents: ["root"]), commit("long", parents: ["root"]), commit("root")
    ])
    // A new tip reusing an ended lane gets its own line and colour.
    #expect(rows[2].lane == rows[1].lane)
    #expect(rows[2].line != rows[1].line)
    #expect(rows[2].color != rows[1].color && rows[2].color != rows[3].color)
    let concurrent = GitGraph.layout([
        commit("merge", parents: ["a", "b", "c"]), commit("a"), commit("b"), commit("c")
    ])
    #expect(Set(concurrent.map(\.color)).count == 3)
    #expect(concurrent.allSatisfy { $0.color < 8 })
}

@Test func workingTreeLinePinsTheCheckout() {
    let rows = GitGraph.layoutWithWorkingTree([commit("other", parents: ["root"]), commit("head", parents: ["root"]), commit("root")], headHash: "head")
    #expect(rows.map(\.lane) == [0, 1, 0, 0])
    #expect(rows[0].line == rows[2].line)
}
