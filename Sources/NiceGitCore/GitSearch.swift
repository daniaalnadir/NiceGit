import Foundation

public enum GitCommitSearchField: String, CaseIterable, Sendable {
    case message = "Message"
    case author = "Author"
    /// Commits that add or remove the text in a file's contents.
    case change = "Code change"
}

extension GitClient {
    /// Searches every branch, tag, and remote-tracking branch for commits matching `text`,
    /// newest first. The text is matched literally and, for messages and authors, without case.
    public func searchCommits(_ text: String, in field: GitCommitSearchField, limit: Int = 200, in url: URL) throws -> [GitCommit] {
        let query = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !query.isEmpty else { return [] }
        let format = "%H%x1f%h%x1f%P%x1f%(decorate:prefix=,suffix=,separator=%x1d,pointer=%x1c,tag=tag: )%x1f%s%x1f%an%x1f%ae%x1f%cr%x1f%ct%x1e"
        var arguments = ["log", "--all", "--exclude=refs/stash", "--decorate=short", "--no-color", "-n", String(max(1, limit)), "--pretty=format:" + format]
        switch field {
        case .message: arguments += ["--fixed-strings", "--regexp-ignore-case", "--grep=" + query]
        case .author: arguments += ["--fixed-strings", "--regexp-ignore-case", "--author=" + query]
        case .change: arguments += ["-S" + query]
        }
        var commits = GitLogParser.parse(try run(arguments + ["--"], in: url))
        // A pasted commit ID finds that commit regardless of the chosen field.
        if query.count >= 4, query.count <= 64, query.allSatisfy(\.isHexDigit),
           let exact = try? run(["log", "-1", "--decorate=short", "--no-color", "--pretty=format:" + format, "--end-of-options", query + "^{commit}", "--"], in: url),
           let commit = GitLogParser.parse(exact).first, !commits.contains(where: { $0.hash == commit.hash }) {
            commits.insert(commit, at: 0)
        }
        return commits
    }
}
