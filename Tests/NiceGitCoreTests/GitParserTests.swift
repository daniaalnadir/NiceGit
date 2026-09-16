import NiceGitCore
import Testing

@Test func exactPathsAndConflictStatesArePreserved() {
    let entries = GitStatusParser.parseNullTerminated("?? folder/a -> b\nfile.txt\0R  new name.txt\0old name.txt\0AA both.txt\0DD deleted.txt\0")
    #expect(entries.count == 4)
    #expect(entries[0].path == "folder/a -> b\nfile.txt")
    #expect(entries[1].path == "new name.txt")
    #expect(entries[1].originalPath == "old name.txt")
    #expect(entries[2].kind == .conflicted)
    #expect(!entries[2].isStaged)
    #expect(entries[3].kind == .conflicted)
    #expect(entries[3].isUnstaged)
}

@Test func fullReferencesDistinguishRemoteFromLocalBranches() {
    let entries = GitBranchParser.parse("refs/heads/origin/main\t*\tabc\tLocal\nrefs/remotes/origin/main\t\tabc\tRemote")
    #expect(entries[0].name == "origin/main")
    #expect(!entries[0].isRemote)
    #expect(entries[1].isRemote)
    #expect(entries[1].displayName == "origin/main")
}

@Test func statusParserClassifiesStagedUnstagedAndRenamedFiles() {
    let output = """
     M Sources/App.swift
    A  Sources/NewView.swift
    R  Old.swift -> New.swift
    ?? Notes.md
    UU Merge.swift
    """

    let entries = GitStatusParser.parse(output)

    #expect(entries.count == 5)
    #expect(entries[0].path == "Sources/App.swift")
    #expect(entries[0].kind == .modified)
    #expect(entries[0].isUnstaged)
    #expect(!entries[0].isStaged)
    #expect(entries[1].kind == .added)
    #expect(entries[1].isStaged)
    #expect(entries[2].originalPath == "Old.swift")
    #expect(entries[2].path == "New.swift")
    #expect(entries[2].kind == .renamed)
    #expect(entries[3].kind == .untracked)
    #expect(entries[4].kind == .conflicted)
}

@Test func branchParserSeparatesCurrentAndRemoteBranches() {
    let output = """
    main\t*\tabc1234\tInitial app
    feature/free-client\t\tdef5678\tAdd graph
    remotes/origin/main\t\tabc1234\tInitial app
    """

    let branches = GitBranchParser.parse(output)

    #expect(branches.count == 3)
    #expect(branches[0].isCurrent)
    #expect(!branches[1].isRemote)
    #expect(branches[2].isRemote)
    #expect(branches[2].displayName == "origin/main")
}

@Test func logParserReadsCommitMetadataAndRefs() {
    let output = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\u{1f}aaaaaaa\u{1f}bbbb cccc\u{1f}HEAD -> main, origin/main\u{1f}Ship MVP\u{1f}Daniaal\u{1f}d@example.com\u{1f}2 hours ago\u{1e}"

    let commits = GitLogParser.parse(output)

    #expect(commits.count == 1)
    #expect(commits[0].shortHash == "aaaaaaa")
    #expect(commits[0].parents == ["bbbb", "cccc"])
    #expect(commits[0].refs == ["HEAD -> main", "origin/main"])
    #expect(commits[0].subject == "Ship MVP")
}
