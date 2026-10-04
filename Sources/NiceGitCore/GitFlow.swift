import Foundation

/// GitFlow settings, stored under the same `gitflow.*` keys as the git-flow command-line tool.
public struct GitFlowConfiguration: Equatable, Sendable {
    public var mainBranch: String
    public var developBranch: String
    public var featurePrefix: String
    public var releasePrefix: String
    public var hotfixPrefix: String
    public var versionTagPrefix: String

    public init(mainBranch: String = "main", developBranch: String = "develop", featurePrefix: String = "feature/",
                releasePrefix: String = "release/", hotfixPrefix: String = "hotfix/", versionTagPrefix: String = "") {
        self.mainBranch = mainBranch; self.developBranch = developBranch; self.featurePrefix = featurePrefix
        self.releasePrefix = releasePrefix; self.hotfixPrefix = hotfixPrefix; self.versionTagPrefix = versionTagPrefix
    }
}

public enum GitFlowKind: String, CaseIterable, Sendable {
    case feature, release, hotfix
}

extension GitClient {
    public func gitFlowConfiguration(in url: URL) -> GitFlowConfiguration? {
        func value(_ key: String) -> String? {
            (try? run(["config", "--get", "gitflow." + key], in: url)).map { $0.hasSuffix("\n") ? String($0.dropLast()) : $0 }
        }
        guard let main = value("branch.master"), let develop = value("branch.develop") else { return nil }
        return GitFlowConfiguration(mainBranch: main, developBranch: develop, featurePrefix: value("prefix.feature") ?? "feature/",
                                    releasePrefix: value("prefix.release") ?? "release/", hotfixPrefix: value("prefix.hotfix") ?? "hotfix/",
                                    versionTagPrefix: value("prefix.versiontag") ?? "")
    }

    /// Saves the configuration and creates the develop branch from the main branch if it is missing.
    public func initializeGitFlow(_ configuration: GitFlowConfiguration, in url: URL) throws {
        for branch in [configuration.mainBranch, configuration.developBranch] { try run(["check-ref-format", "--branch", branch], in: url) }
        guard configuration.mainBranch != configuration.developBranch else {
            throw GitClientError.commandFailed(command: "gitflow", message: "The main and develop branches must be different.")
        }
        try run(["show-ref", "--verify", "--quiet", "refs/heads/" + configuration.mainBranch], in: url)
        if (try? run(["show-ref", "--verify", "--quiet", "refs/heads/" + configuration.developBranch], in: url)) == nil {
            try run(["branch", "--no-track", "--", configuration.developBranch, "refs/heads/" + configuration.mainBranch], in: url)
        }
        let values = [("branch.master", configuration.mainBranch), ("branch.develop", configuration.developBranch),
                      ("prefix.feature", configuration.featurePrefix), ("prefix.release", configuration.releasePrefix),
                      ("prefix.hotfix", configuration.hotfixPrefix), ("prefix.versiontag", configuration.versionTagPrefix)]
        for (key, value) in values { try run(["config", "--local", "gitflow." + key, value], in: url) }
    }

    /// Creates `<prefix><name>` from develop (features, releases) or main (hotfixes) and checks it out.
    public func startGitFlow(_ kind: GitFlowKind, name: String, expectedBranch: String, expectedHead: String?, in url: URL) throws {
        let configuration = try requireGitFlow(in: url)
        try requireCleanCheckout(expectedBranch: expectedBranch, expectedHead: expectedHead, command: "gitflow start", in: url)
        let trimmed = name.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { throw GitClientError.emptyBranchName }
        let branch = prefix(kind, configuration) + trimmed
        try run(["check-ref-format", "--branch", branch], in: url)
        let base = kind == .hotfix ? configuration.mainBranch : configuration.developBranch
        try run(["switch", "--no-overwrite-ignore", "--no-track", "--create", branch, "refs/heads/" + base], in: url)
    }

    /// Finishes the current GitFlow branch: features merge into develop; releases and hotfixes merge
    /// into main, are tagged, then merge into develop. Merges never fast-forward, so history keeps
    /// the branch. The branch is deleted only once every merge has succeeded; a conflict stops
    /// with the merge in progress.
    public func finishGitFlow(expectedBranch: String, expectedHead: String?, tagMessage: String? = nil, in url: URL) throws {
        let configuration = try requireGitFlow(in: url)
        try requireCleanCheckout(expectedBranch: expectedBranch, expectedHead: expectedHead, command: "gitflow finish", in: url)
        guard let kind = GitFlowKind.allCases.first(where: { expectedBranch.hasPrefix(prefix($0, configuration)) && expectedBranch.count > prefix($0, configuration).count }) else {
            throw GitClientError.commandFailed(command: "gitflow finish", message: "\(expectedBranch) is not a GitFlow feature, release, or hotfix branch.")
        }
        let name = String(expectedBranch.dropFirst(prefix(kind, configuration).count))
        let targets = kind == .feature ? [configuration.developBranch] : [configuration.mainBranch, configuration.developBranch]
        let tag = configuration.versionTagPrefix + name
        let tip = try run(["rev-parse", "--verify", "refs/heads/" + expectedBranch], in: url).trimmingCharacters(in: .whitespacesAndNewlines)
        let contains = { (commitish: String) in (try? self.run(["merge-base", "--is-ancestor", tip, commitish], in: url)) != nil }
        var needsTag = false
        if kind != .feature {
            try run(["check-ref-format", "refs/tags/" + tag], in: url)
            if (try? run(["show-ref", "--verify", "--quiet", "refs/tags/" + tag], in: url)) != nil {
                // A tag from an earlier, interrupted finish is reused only if it already includes this branch.
                guard contains("refs/tags/" + tag + "^{commit}") else {
                    throw GitClientError.commandFailed(command: "gitflow finish", message: "The tag \(tag) already exists for other work.")
                }
            } else {
                needsTag = true
            }
        }
        // Finishing can be repeated after resolving a conflict: targets that already contain the
        // branch are skipped.
        for (index, target) in targets.enumerated() {
            if !contains("refs/heads/" + target) {
                try run(["switch", "--no-overwrite-ignore", "--", target], in: url)
                try requireNoIgnoredMergeCollisions(target: tip, in: url)
                do {
                    try run(["merge", "--no-ff", "--no-edit", "--no-overwrite-ignore", tip], in: url)
                } catch {
                    throw GitClientError.commandFailed(command: "gitflow finish", message: "Merging \(expectedBranch) into \(target) stopped, usually for a conflict. Resolve it and continue the merge, then check out \(expectedBranch) and finish again; it has been kept.")
                }
            }
            if needsTag && index == 0 {
                let message = tagMessage.map { $0.trimmingCharacters(in: .whitespacesAndNewlines) }.flatMap { $0.isEmpty ? nil : $0 }
                    ?? "\(kind == .release ? "Release" : "Hotfix") \(name)"
                try run(["tag", "--annotate", "--message", message, "--", tag, "refs/heads/" + target], in: url)
            }
        }
        try run(["switch", "--no-overwrite-ignore", "--", targets.last!], in: url)
        try run(["branch", "--delete", "--", expectedBranch], in: url)
    }

    private func prefix(_ kind: GitFlowKind, _ configuration: GitFlowConfiguration) -> String {
        switch kind {
        case .feature: configuration.featurePrefix
        case .release: configuration.releasePrefix
        case .hotfix: configuration.hotfixPrefix
        }
    }

    private func requireGitFlow(in url: URL) throws -> GitFlowConfiguration {
        guard let configuration = gitFlowConfiguration(in: url) else {
            throw GitClientError.commandFailed(command: "gitflow", message: "Set up GitFlow for this repository first.")
        }
        return configuration
    }

    private func requireCleanCheckout(expectedBranch: String, expectedHead: String?, command: String, in url: URL) throws {
        try requireSelectedCheckout(branch: expectedBranch, head: expectedHead, command: command, in: url)
        guard try currentOperation(in: url) == nil, try loadStatus(in: url).allSatisfy({ $0.kind == .untracked }) else {
            throw GitClientError.commandFailed(command: command, message: "Commit or stash your changes and finish any Git operation first.")
        }
    }
}
