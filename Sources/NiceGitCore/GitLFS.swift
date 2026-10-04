import Foundation

public struct GitLFSFile: Identifiable, Equatable, Sendable {
    public let path: String
    /// The working file is still an LFS pointer because its content has not been downloaded.
    public let isPointerOnly: Bool
    public var id: String { path }
    public init(path: String, isPointerOnly: Bool) { self.path = path; self.isPointerOnly = isPointerOnly }
}

public struct GitLFSStatus: Equatable, Sendable {
    /// The installed git-lfs version, or nil when Git LFS is not installed.
    public let version: String?
    /// Patterns in the root .gitattributes that store matching files with Git LFS.
    public let patterns: [String]
    public let files: [GitLFSFile]
}

extension GitClient {
    public func lfsStatus(in url: URL) throws -> GitLFSStatus {
        let version = (try? run(["lfs", "version"], in: url))?.trimmingCharacters(in: .whitespacesAndNewlines)
        let tracked = try run(["ls-files", "-z"], in: url).split(separator: "\0").map(String.init)
        var files: [GitLFSFile] = []
        // check-attr takes plain pathnames; ask in batches to keep argument lists short.
        for start in stride(from: 0, to: tracked.count, by: 500) {
            let batch = Array(tracked[start..<min(start + 500, tracked.count)])
            let fields = try run(["check-attr", "-z", "filter", "--"] + batch, in: url).split(separator: "\0", omittingEmptySubsequences: false)
            for index in stride(from: 0, to: fields.count - 2, by: 3) where fields[index + 2] == "lfs" {
                let path = String(fields[index])
                files.append(GitLFSFile(path: path, isPointerOnly: Self.isLFSPointer(url.appendingPathComponent(path))))
            }
        }
        return GitLFSStatus(version: version, patterns: try lfsPatterns(in: url), files: files)
    }

    /// Stores files matching `pattern` with Git LFS from their next commit, as `git lfs track` does.
    /// Refused while Git LFS is not installed, since Git would then commit the full files.
    public func trackLFS(_ pattern: String, in url: URL) throws {
        let pattern = pattern.trimmingCharacters(in: .whitespaces)
        guard !pattern.isEmpty, !pattern.contains("\n") else {
            throw GitClientError.commandFailed(command: "lfs track", message: "Enter a file pattern such as *.psd.")
        }
        guard (try? run(["lfs", "version"], in: url)) != nil else {
            throw GitClientError.commandFailed(command: "lfs track", message: "Git LFS is not installed. Install it (for example with `brew install git-lfs`) before tracking files.")
        }
        guard try !lfsPatterns(in: url).contains(pattern) else { return }
        // Patterns with spaces are quoted the way .gitattributes expects.
        let written = pattern.contains(" ") || pattern.contains("\"")
            ? "\"" + pattern.replacingOccurrences(of: "\\", with: "\\\\").replacingOccurrences(of: "\"", with: "\\\"") + "\""
            : pattern
        try editAttributes(in: url) { lines in lines + [written + " filter=lfs diff=lfs merge=lfs -text"] }
    }

    /// Stops storing new versions of matching files with Git LFS; files already committed stay as they are.
    public func untrackLFS(_ pattern: String, in url: URL) throws {
        try editAttributes(in: url) { lines in
            lines.filter { line in !(Self.attributePattern(line) == pattern && line.split(separator: " ").contains("filter=lfs")) }
        }
    }

    func lfsPatterns(in url: URL) throws -> [String] {
        let file = url.appendingPathComponent(".gitattributes")
        guard let text = try? String(contentsOf: file, encoding: .utf8) else { return [] }
        return text.split(separator: "\n").map(String.init)
            .filter { $0.split(whereSeparator: \.isWhitespace).contains("filter=lfs") }
            .compactMap(Self.attributePattern)
    }

    private func editAttributes(in url: URL, _ change: ([String]) -> [String]) throws {
        let file = url.appendingPathComponent(".gitattributes")
        let text = (try? String(contentsOf: file, encoding: .utf8)) ?? ""
        var lines = text.components(separatedBy: "\n")
        if lines.last == "" { lines.removeLast() }
        let updated = change(lines)
        guard updated != lines else { return }
        try Data((updated.joined(separator: "\n") + (updated.isEmpty ? "" : "\n")).utf8).write(to: file)
    }

    /// The pattern at the start of a .gitattributes line, unquoting a C-style quoted pattern.
    static func attributePattern(_ line: String) -> String? {
        let trimmed = line.trimmingCharacters(in: .whitespaces)
        guard !trimmed.isEmpty, !trimmed.hasPrefix("#") else { return nil }
        if trimmed.hasPrefix("\"") {
            // The pattern ends at the first closing quote that is not escaped.
            var escaped = false
            for index in trimmed.indices.dropFirst() {
                let character = trimmed[index]
                if character == "\"" && !escaped { return GitBlameParser.unquote(String(trimmed[...index])) }
                escaped = character == "\\" && !escaped
            }
            return nil
        }
        return trimmed.split(whereSeparator: \.isWhitespace).first.map(String.init)
    }

    static func isLFSPointer(_ file: URL) -> Bool {
        guard let handle = try? FileHandle(forReadingFrom: file) else { return false }
        defer { try? handle.close() }
        let prefix = (try? handle.read(upToCount: 64)) ?? Data()
        return prefix.starts(with: Data("version https://git-lfs.github.com/spec/v1".utf8))
    }
}
