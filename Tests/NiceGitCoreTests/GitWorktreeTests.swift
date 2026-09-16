import NiceGitCore
import Testing

@Test func worktreesPreservePathsAndCheckoutStates() {
    let trees = GitWorktree.parse("worktree /repo with\nnewline\0HEAD abc\0branch refs/heads/main\0\0worktree /detached\0HEAD def\0detached\0locked reason\0\0worktree /missing\0prunable missing folder\0bare\0\0")
    #expect(trees.count == 3)
    #expect(trees[0].path == "/repo with\nnewline")
    #expect(trees[0].branch == "main")
    #expect(trees[1].branch == nil)
    #expect(trees[1].isLocked)
    #expect(trees[2].isBare && trees[2].isPrunable)
}
