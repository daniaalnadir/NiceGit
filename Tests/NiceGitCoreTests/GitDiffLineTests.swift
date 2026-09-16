import NiceGitCore
import Testing

@Test func diffLineNumbersStopAtDeclaredHunkBoundary() {
    let lines = GitDiffLine.parse("@@ -1 +1 @@\n-old\n+new\n-- trailer\n metadata\n@@ -4,0 +5,1 @@\n+added\n+not part of the hunk")
    #expect(lines[1].oldNumber == 1)
    #expect(lines[2].newNumber == 1)
    #expect(lines[3].kind == .metadata)
    #expect(lines[4].oldNumber == nil)
    #expect(lines[6].newNumber == 5)
    #expect(lines[7].kind == .metadata)
}

@Test func diffLineNumbersFollowHunksAndResetBetweenFiles() {
    let lines = GitDiffLine.parse("diff --git a/a b/a\n--- a/a\n+++ b/a\n@@ -4,2 +8,2 @@\n same\n-old\n+new\n\\ No newline at end of file\ndiff --git a/b b/b\n--- a/b\n+++ b/b\n@@ -0,0 +1 @@\n+first")
    #expect(lines[2].kind == .metadata)
    #expect(lines[4].oldNumber == 4 && lines[4].newNumber == 8)
    #expect(lines[5].oldNumber == 5 && lines[5].newNumber == nil)
    #expect(lines[6].oldNumber == nil && lines[6].newNumber == 9)
    #expect(lines[7].kind == .metadata)
    #expect(lines[10].kind == .metadata)
    #expect(lines[12].newNumber == 1)
    #expect(GitDiffLine.parse("").isEmpty)
}
