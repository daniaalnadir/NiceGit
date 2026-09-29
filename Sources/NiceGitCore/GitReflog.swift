import Foundation

/// One position HEAD has pointed to, newest first; commits left behind by resets, rebases,
/// amends, or deleted branches stay reachable here until Git expires them.
public struct GitReflogEntry: Identifiable, Hashable, Sendable {
    public let hash: String
    /// Git's selector for this entry, such as `HEAD@{3}`.
    public let selector: String
    /// What moved HEAD, such as `commit: Fix typo` or `rebase (finish): returning to refs/heads/main`.
    public let action: String
    public let subject: String
    public let date: Date?
    public var id: String { selector }
    public var shortHash: String { String(hash.prefix(7)) }
}

extension GitClient {
    public func reflog(limit: Int = 300, in url: URL) throws -> [GitReflogEntry] {
        guard (try? run(["rev-parse", "--verify", "HEAD"], in: url)) != nil else { return [] }
        let output = try run(["log", "--walk-reflogs", "--no-color", "-n", String(max(1, limit)),
                              "--format=%H%x1f%gd%x1f%gs%x1f%s%x1f%ct%x1e", "HEAD", "--"], in: url)
        return output.split(separator: "\u{1e}").compactMap { record in
            let fields = record.trimmingCharacters(in: .newlines).components(separatedBy: "\u{1f}")
            guard fields.count == 5, !fields[0].isEmpty else { return nil }
            return GitReflogEntry(hash: fields[0], selector: fields[1], action: fields[2], subject: fields[3],
                                  date: TimeInterval(fields[4]).map { Date(timeIntervalSince1970: $0) })
        }
    }

    /// The commits among `hashes` that no branch, tag, or remote-tracking branch contains.
    public func unreachableCommits(_ hashes: [String], in url: URL) throws -> Set<String> {
        let unique = Array(Set(hashes.filter { !$0.isEmpty && $0.allSatisfy(\.isHexDigit) }))
        guard !unique.isEmpty else { return [] }
        let output = try run(["rev-list", "--no-walk=unsorted"] + unique + ["--not", "--all", "--"], in: url)
        return Set(output.split(separator: "\n").map(String.init))
    }
}
