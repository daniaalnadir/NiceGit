import Foundation

public enum GitIgnoreRule: Sendable {
    /// Only the selected path, anchored at the repository root.
    case path
    /// Every file with the selected file's extension, in any folder.
    case fileExtension
}

public enum GitIgnoreScope: Sendable {
    /// `.gitignore` at the repository root, shared when committed.
    case shared
    /// `info/exclude` in the Git directory, which stays on this computer.
    case local
}

extension GitClient {
    /// The ignore pattern for `path`, escaped so wildcard and comment characters in the name
    /// are matched literally. Returns nil for a name no ignore file can express.
    public static func ignorePattern(for path: String, rule: GitIgnoreRule) -> String? {
        guard !path.isEmpty, !path.contains("\n"), !path.contains("\r") else { return nil }
        func escape(_ text: Substring) -> String {
            var escaped = ""
            for character in text {
                if "\\*?[".contains(character) { escaped.append("\\") }
                escaped.append(character)
            }
            // Git trims unescaped trailing spaces from patterns.
            let trailing = escaped.reversed().prefix { $0 == " " }.count
            return String(escaped.dropLast(trailing)) + String(repeating: "\\ ", count: trailing)
        }
        switch rule {
        case .path:
            let isFolder = path.hasSuffix("/")
            let body = escape(isFolder ? path.dropLast() : Substring(path))
            return "/" + body + (isFolder ? "/" : "")
        case .fileExtension:
            let name = path.split(separator: "/").last ?? Substring(path)
            guard !path.hasSuffix("/"), let dot = name.lastIndex(of: "."), dot != name.startIndex,
                  name.index(after: dot) != name.endIndex else { return nil }
            return "*" + escape(name[dot...])
        }
    }

    /// Adds an ignore rule for an untracked path and confirms Git now ignores it. An untracked
    /// path that is already ignored is left as it is.
    public func ignore(path: String, rule: GitIgnoreRule, scope: GitIgnoreScope, in url: URL) throws {
        guard let pattern = Self.ignorePattern(for: path, rule: rule) else {
            throw GitClientError.commandFailed(command: "ignore", message: rule == .fileExtension
                ? "This file has no extension to ignore."
                : "This name contains a line break, which ignore files cannot express.")
        }
        // check-ignore takes plain pathnames (it rejects literal pathspec magic), so a glob-like name is safe here.
        let isIgnored = { (try? self.run(["check-ignore", "--quiet", "--no-index", "--", path], in: url)) != nil }
        guard try loadStatus(in: url).contains(where: { $0.path == path && $0.kind == .untracked }) else {
            if try run(["ls-files", "-z", "--", path], in: url).isEmpty && isIgnored() { return }
            throw GitClientError.commandFailed(command: "ignore", message: "Only untracked files can be ignored. Refresh and review this file again.")
        }
        let file: URL
        switch scope {
        case .shared:
            file = url.appendingPathComponent(".gitignore")
        case .local:
            var location = try run(["rev-parse", "--git-path", "info/exclude"], in: url)
            if location.hasSuffix("\n") { location.removeLast() }
            file = location.hasPrefix("/") ? URL(fileURLWithPath: location) : url.appendingPathComponent(location)
            try FileManager.default.createDirectory(at: file.deletingLastPathComponent(), withIntermediateDirectories: true)
        }
        let existing = (try? Data(contentsOf: file)) ?? Data()
        let lines = String(decoding: existing, as: UTF8.self).split(separator: "\n", omittingEmptySubsequences: false)
        if !lines.contains(where: { $0 == Substring(pattern) }) {
            var addition = Data()
            if let last = existing.last, last != UInt8(ascii: "\n") { addition.append(UInt8(ascii: "\n")) }
            addition.append(contentsOf: Array((pattern + "\n").utf8))
            if FileManager.default.fileExists(atPath: file.path) {
                let handle = try FileHandle(forWritingTo: file)
                defer { try? handle.close() }
                try handle.seekToEnd()
                try handle.write(contentsOf: addition)
            } else {
                try addition.write(to: file)
            }
        }
        guard isIgnored() else {
            throw GitClientError.commandFailed(command: "ignore", message: "The rule was added, but Git still does not ignore this path. A later negated rule may re-include it.")
        }
    }
}
