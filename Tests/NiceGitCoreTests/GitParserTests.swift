import NiceGitCore
import Testing

@Test func remoteHeadAliasIsNotABranch() {
    let entries = GitBranchParser.parse("refs/remotes/origin/HEAD\t\tabc\tAlias\nrefs/remotes/origin/main\t\tabc\tRemote\nrefs/heads/topic/HEAD\t*\tabc\tLocal")
    #expect(entries.map(\.name) == ["remotes/origin/main", "topic/HEAD"])
    let withSymbolicRefs = GitBranchParser.parse(
        "refs/remotes/team/shared/HEAD\t \tabc\tAlias\t\trefs/remotes/team/shared/main\n" +
        "refs/remotes/team/shared/main\t \tabc\tRemote\t\t\n" +
        "refs/remotes/origin/topic/HEAD\t \tabc\tReal branch\t\t\n" +
        "refs/heads/main\t*\tabc\tSubject\twith tab\trefs/remotes/origin/main\t",
        includesSymref: true
    )
    #expect(withSymbolicRefs.map(\.name) == ["remotes/team/shared/main", "remotes/origin/topic/HEAD", "main"])
    #expect(withSymbolicRefs.last?.subject == "Subject\twith tab")
    #expect(withSymbolicRefs.last?.upstream == "refs/remotes/origin/main")
}

@Test func remoteAddressesUseFetchURLsForProviderIdentity() {
    let output = "origin\tgit@github.com:example/project.git (fetch)\norigin\thttps://elsewhere.example/project.git (push)\nbackup\tssh://git@example.org/project.git (fetch)\n"
    let addresses = GitRemoteParser.addresses(output)
    #expect(addresses["origin"] == "git@github.com:example/project.git")
    #expect(addresses["backup"] == "ssh://git@example.org/project.git")
    #expect((try? GitHubRepository(remoteAddress: addresses["origin"]!)) != nil)
    #expect((try? GitHubRepository(remoteAddress: addresses["backup"]!)) == nil)
    #expect(GitRemoteParser.allAddresses(output, direction: "fetch")["origin"] == ["git@github.com:example/project.git"])
    #expect(GitRemoteParser.allAddresses(output, direction: "push")["origin"] == ["https://elsewhere.example/project.git"])
    let multiple = "origin\thttps://first.example/project.git (push)\norigin\thttps://second.example/project.git (push)\n"
    #expect(GitRemoteParser.allAddresses(multiple, direction: "push")["origin"] == ["https://first.example/project.git", "https://second.example/project.git"])
}

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
    let entries = GitBranchParser.parse("refs/heads/origin/main\t*\tabc\tLocal\nrefs/remotes/origin/main\t\tabc\tRemote\nrefs/remotes/origin/topic/remotes/demo\t\tabc\tNested\nrefs/heads/remotes/origin/main\t\tabc\tLocal collision")
    #expect(entries[0].name == "origin/main")
    #expect(!entries[0].isRemote)
    #expect(entries[1].isRemote)
    #expect(entries[1].displayName == "origin/main")
    #expect(entries[2].displayName == "origin/topic/remotes/demo")
    #expect(entries[3].name == entries[1].name)
    #expect(entries[3].id != entries[1].id)
}

@Test func overlappingRemoteNamesAssignEachBranchOnce() {
    let branch = GitBranch(name: "remotes/team/shared/main", isCurrent: false, isRemote: true, tip: "abc", subject: "")
    let local = GitBranch(name: branch.name, isCurrent: false, isRemote: false, tip: "abc", subject: "")
    #expect(branch.remoteName(among: ["team", "team/shared"]) == "team/shared")
    #expect(local.remoteName(among: ["team", "team/shared"]) == nil)
}

@Test func statusEntryIdentityKeepsLiteralSeparatorPathsDistinct() {
    let entries = GitStatusParser.parseNullTerminated("R  b|c\0a\0R  c\0a|b\0")
    #expect(entries.count == 2)
    #expect(entries[0].id != entries[1].id)
}

@Test func quickStatusParserKeepsCheckoutAndExactPaths() {
    let output = "# branch.oid abc123\0# branch.head topic\0" +
        "2 R. N... 100644 100644 100644 abc abc R100 new name.txt\0old.txt\0" +
        "?  leading.txt\0" +
        "u UU N... 100644 100644 100644 100644 abc def ghi conflict.txt\0"
    let parsed = GitStatusParser.parseWithCheckout(output)
    #expect(parsed.isComplete)
    #expect(parsed.branch == "topic")
    #expect(parsed.headHash == "abc123")
    #expect(parsed.entries.map(\.path) == ["new name.txt", " leading.txt", "conflict.txt"])
    #expect(parsed.entries[0].originalPath == "old.txt")
    #expect(parsed.entries[0].kind == .renamed)
    #expect(parsed.entries[0].indexStatus == "R")
    #expect(parsed.entries[0].workTreeStatus == " ")
    #expect(parsed.entries[1].kind == .untracked)
    #expect(parsed.entries[2].kind == .conflicted)
    #expect(GitStatusParser.parseWithCheckout("# branch.oid (initial)\0# branch.head main\0").headHash == nil)
    #expect(!GitStatusParser.parseWithCheckout("# branch.oid abc123\0# branch.head main\0new-format path\0").isComplete)
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
    #expect(commits[0].commitDate == nil)
    let dated = output.replacingOccurrences(of: "\u{1e}", with: "\u{1f}1700000000\u{1e}")
    #expect(GitLogParser.parse(dated).first?.commitDate?.timeIntervalSince1970 == 1_700_000_000)
}
