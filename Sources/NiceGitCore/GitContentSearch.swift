import Foundation

public struct GitContentMatch: Identifiable, Equatable, Sendable {
    public let path: String
    /// One-based line number.
    public let line: Int
    public let text: String
    public var id: String { path + "\0" + String(line) }
    public init(path: String, line: Int, text: String) { self.path = path; self.line = line; self.text = text }
}

extension GitClient {
    /// Finds lines containing `text` in tracked text files, at `revision` or in the working files
    /// when it is nil. The text is matched literally. Results stop at `limit` lines.
    public func searchContents(_ text: String, at revision: String? = nil, ignoreCase: Bool = true, limit: Int = 1000, in url: URL) throws -> [GitContentMatch] {
        guard !text.isEmpty, !text.contains("\n") else { return [] }
        let commit = try revision.map {
            try run(["rev-parse", "--verify", "--end-of-options", $0 + "^{commit}"], in: url).trimmingCharacters(in: .whitespacesAndNewlines)
        }
        var arguments = ["grep", "-n", "-I", "--null", "--full-name", "--no-color", "--fixed-strings", "--max-count=100"]
        if ignoreCase { arguments.append("--ignore-case") }
        arguments += ["-e", text] + (commit.map { [$0] } ?? []) + ["--"]
        // Exit status 1 means no matches.
        let output = try run(arguments, in: url, acceptedStatuses: [0, 1])
        // Each match is "path NUL line NUL text LF". Text never holds a newline but a path can,
        // so split on NUL and end each text at its first newline.
        let prefix = commit.map { $0 + ":" } ?? ""
        var tokens = output.split(separator: "\0", omittingEmptySubsequences: false)[...]
        var matches: [GitContentMatch] = []
        guard var path = tokens.popFirst().map(String.init) else { return [] }
        while matches.count < limit, let number = tokens.popFirst(), let rest = tokens.popFirst() {
            let newline = rest.firstIndex(of: "\n") ?? rest.endIndex
            let file = path.hasPrefix(prefix) ? String(path.dropFirst(prefix.count)) : path
            if let line = Int(number) { matches.append(GitContentMatch(path: file, line: line, text: String(rest[..<newline]))) }
            path = newline < rest.endIndex ? String(rest[rest.index(after: newline)...]) : ""
        }
        return matches
    }
}
