import Foundation
import NiceGitCore
import Testing

@Test func githubCommitLinksUseFullHashesAndValidatedRepositories() throws {
    let repository = try GitHubRepository(remoteAddress: "git@github.com:owner/repo.git")
    for length in [40, 64] {
        let hash = String(repeating: "a", count: length)
        #expect(try repository.commitURL(hash: hash).absoluteString == "https://github.com/owner/repo/commit/" + hash)
    }
    for hash in ["", "abc1234", "../main", String(repeating: "z", count: 40), String(repeating: "a", count: 39) + "?"] {
        #expect(throws: (any Error).self) { try repository.commitURL(hash: hash) }
    }
}

@Test func githubRemoteAddressesAreScopedToSupportedHost() throws {
    for address in ["git@github.com:owner/repo.git", "https://github.com/owner/repo", "ssh://git@github.com/owner/repo.git"] {
        #expect(try GitHubRepository(remoteAddress: address).slug == "owner/repo")
    }
    for address in ["https://github.com.evil.test/owner/repo", "file:///owner/repo", "https://github.com/owner/repo?token=secret", "https://github.com/owner/repo/more", "https://github.com/owner/%2E%2E", "https://github.com/owner/.git"] {
        #expect(throws: (any Error).self) { try GitHubRepository(remoteAddress: address) }
    }
}

@Test func githubItemsDecodeDraftsAndValidateRepositoryLinks() throws {
    let repository = try GitHubRepository(remoteAddress: "https://github.com/owner/repo")
    let json = #"[{"number":12,"title":"A pull request","url":"https://github.com/owner/repo/pull/12","author":{"login":"author"},"isDraft":true}]"#
    let items = try GitHubClient.decode(Data(json.utf8), repository: repository, kind: .pullRequest)
    #expect(items.first?.number == 12)
    #expect(items.first?.isDraft == true)
    #expect(items.first?.author?.login == "author")
    #expect(items.first?.matches("PULL REQUEST") == true)
    #expect(items.first?.matches(" #12 ") == true)
    #expect(items.first?.matches("AUTHOR") == true)
    #expect(items.first?.matches("unrelated") == false)
    let issue = #"[{"number":3,"title":"An issue","url":"https://github.com/owner/repo/issues/3","author":null}]"#
    #expect(try GitHubClient.decode(Data(issue.utf8), repository: repository, kind: .issue).first?.author == nil)
    for url in ["https://evil.test/owner/repo/pull/12", "https://github.com/other/repo/pull/12", "https://github.com/owner/repo/issues/12", "https://github.com/owner/repo/pull/13", "http://github.com/owner/repo/pull/12"] {
        let invalid = json.replacingOccurrences(of: "https://github.com/owner/repo/pull/12", with: url)
        #expect(throws: (any Error).self) { try GitHubClient.decode(Data(invalid.utf8), repository: repository, kind: .pullRequest) }
    }
    #expect(throws: (any Error).self) { try GitHubClient.decode(Data("not json".utf8), repository: repository, kind: .issue) }
}

@Test(.enabled(if: ProcessInfo.processInfo.environment["NICEGIT_LIVE_GITHUB_TEST"] == "1"))
func liveGitHubSidebarLoadsPublicRepository() throws {
    let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: root) }
    let git = GitClient()
    try git.initialize(at: root)
    try git.addRemote(name: "origin", address: "https://github.com/cli/cli.git", in: root)
    for kind in [GitHubItemKind.pullRequest, .issue] {
        let items = try GitHubClient().load(kind: kind, remote: "origin", in: root, limit: 1, control: GitCommandControl())
        #expect(items.count == 1)
        #expect(items.allSatisfy { $0.state?.lowercased() == "open" })
    }
    for (kind, state) in [(GitHubItemKind.pullRequest, GitHubItemState.merged), (.issue, .closed)] {
        let items = try GitHubClient().load(kind: kind, remote: "origin", in: root, limit: 1, state: state, control: GitCommandControl())
        #expect(items.count == 1)
        #expect(items.allSatisfy { $0.state?.lowercased() == state.rawValue })
    }
}
