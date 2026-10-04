import Foundation

extension GitClient {
    /// Files that differ between two commits, or between a commit and the working files when
    /// `newer` is nil. Untracked files are not part of a comparison.
    public func compareFiles(from older: String, to newer: String?, in url: URL) throws -> [GitCommitFileChange] {
        let output = try run(["diff", "--name-status", "-z", "--no-renames", "--no-ext-diff"] + (try revisions(older, newer, in: url)) + ["--"], in: url)
        let fields = output.split(separator: "\0", omittingEmptySubsequences: false)
        return stride(from: 0, to: max(0, fields.count - 1), by: 2).map {
            GitCommitFileChange(path: String(fields[$0 + 1]), status: String(fields[$0]))
        }.sorted { $0.path < $1.path }
    }

    /// The change to one file between two commits, or from a commit to the working file.
    public func compareFileDiff(from older: String, to newer: String?, path: String, ignoreWhitespace: Bool = false, in url: URL) throws -> String {
        try run(["diff", "--patch", "--unified=3", "--no-renames", "--no-ext-diff", "--no-color"] + (ignoreWhitespace ? ["-w"] : []) + (try revisions(older, newer, in: url)) + ["--", path], in: url)
    }

    /// Resolves both sides to object IDs, so neither can be read as an option or a path.
    private func revisions(_ older: String, _ newer: String?, in url: URL) throws -> [String] {
        try ([older] + (newer.map { [$0] } ?? [])).map {
            try run(["rev-parse", "--verify", "--end-of-options", $0 + "^{commit}"], in: url).trimmingCharacters(in: .whitespacesAndNewlines)
        }
    }
}
