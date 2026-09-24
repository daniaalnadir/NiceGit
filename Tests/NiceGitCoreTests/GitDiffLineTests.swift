import NiceGitCore
import Testing

@Test func changesOnlyOmitsCommitMetadataAndContext() {
    let patch = "commit abc123\nAuthor: Example\ndiff --git a/file b/file\nindex 123..456\n--- a/file\n+++ b/file\n@@ -2,2 +2,2 @@\n context\n-before\n+after\n\\ No newline at end of file\n"
    let lines = GitDiffLine.changesOnly(patch)
    #expect(lines.map(\.kind) == [.hunk, .deletion, .addition])
    #expect(lines[1].text == "-before")
    #expect(lines[2].text == "+after")
}

@Test func codeOnlyKeepsNearbyContextButNoPatchMetadata() {
    let patch = "commit abc123\ndiff --git a/file b/file\n--- a/file\n+++ b/file\n@@ -2,2 +2,2 @@\n context\n-before\n+after\n"
    let lines = GitDiffLine.codeOnly(patch)
    #expect(lines.map(\.kind) == [.hunk, .context, .deletion, .addition])
    #expect(lines[1].oldNumber == 2)
    #expect(lines[1].newNumber == 2)
}

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

@Test func inlineChangesHighlightOnlyReplacedTextAndKeepHunksSeparate() {
    let lines = GitDiffLine.parse("@@ -1 +1 @@\n-let color = red\n+let color = green\n@@ -5,0 +5 @@\n+new line")
    let changes = GitInlineChange.highlights(in: lines)
    #expect(changes[1] == GitInlineChange(prefix: "let color = ", changed: "red", suffix: ""))
    #expect(changes[2] == GitInlineChange(prefix: "let color = ", changed: "green", suffix: ""))
    #expect(changes[4] == nil)
}

@Test func whollyAddedLinesDoNotReceiveInlineHighlight() {
    let lines = GitDiffLine.parse("@@ -1 +1,3 @@\n unchanged\n+entirely new\n+also new")
    #expect(GitInlineChange.highlights(in: lines).isEmpty)
}

@Test func unrelatedReplacementLinesKeepWholeLineHighlight() {
    let lines = GitDiffLine.parse("@@ -1 +1 @@\n-    old content\n+    entirely new")
    #expect(GitInlineChange.highlights(in: lines).isEmpty)
}

@Test func inlineChangesHandleInsertionAndDeletionWithinLines() {
    let lines = GitDiffLine.parse("@@ -1 +1 @@\n-let value = oldName()\n+let value = oldLongName()")
    let changes = GitInlineChange.highlights(in: lines)
    #expect(changes[1] == GitInlineChange(prefix: "let value = old", changed: "", suffix: "Name()"))
    #expect(changes[2] == GitInlineChange(prefix: "let value = old", changed: "Long", suffix: "Name()"))
}
