import Foundation

/// The commit that last changed a group of blamed lines.
public struct GitBlameCommit: Hashable, Sendable {
    public let hash: String
    public let authorName: String
    public let authorEmail: String
    public let date: Date?
    public let summary: String
    /// The path this commit's version of the lines used, which differs after a rename.
    public let path: String

    /// Lines Git has not seen in any commit, such as unsaved working-tree edits.
    public var isUncommitted: Bool { hash.allSatisfy { $0 == "0" } }
    public var shortHash: String { String(hash.prefix(7)) }
}

public struct GitBlameLine: Identifiable, Hashable, Sendable {
    /// One-based line number in the blamed version of the file.
    public let number: Int
    public let content: String
    public let commit: GitBlameCommit
    public var id: Int { number }
}

public enum GitBlameParser {
    /// Parses `git blame --porcelain`. Commit details appear only on a commit's first group,
    /// so later groups reuse them; every content line starts with a tab.
    public static func parse(_ output: String) -> [GitBlameLine] {
        var details: [String: [String: String]] = [:]
        var result: [GitBlameLine] = []
        // Split on bytes: Swift treats CRLF as one character, so splitting the String would
        // merge every line of a file with Windows line endings.
        var lines = output.utf8.split(separator: UInt8(ascii: "\n"), omittingEmptySubsequences: false)
            .map { String(decoding: $0, as: UTF8.self) }[...]
        while let header = lines.popFirst() {
            let fields = header.split(separator: " ")
            guard fields.count >= 3, fields[0].count >= 40, let number = Int(fields[2]) else { continue }
            let hash = String(fields[0])
            var info = details[hash] ?? [:]
            while let line = lines.first, !line.hasPrefix("\t") {
                lines.removeFirst()
                let pair = line.split(separator: " ", maxSplits: 1, omittingEmptySubsequences: false)
                info[String(pair[0])] = pair.count > 1 ? String(pair[1]) : ""
            }
            details[hash] = info
            guard let content = lines.popFirst() else { break }
            let commit = GitBlameCommit(
                hash: hash,
                authorName: info["author"] ?? "",
                authorEmail: (info["author-mail"] ?? "").trimmingCharacters(in: CharacterSet(charactersIn: "<>")),
                date: info["author-time"].flatMap(TimeInterval.init).map { Date(timeIntervalSince1970: $0) },
                summary: info["summary"] ?? "",
                path: unquote(info["filename"] ?? ""))
            result.append(GitBlameLine(number: number, content: String(content.dropFirst()), commit: commit))
        }
        return result
    }

    /// Reverses Git's C-style path quoting, including octal escapes for raw bytes.
    public static func unquote(_ value: String) -> String {
        guard value.count >= 2, value.hasPrefix("\""), value.hasSuffix("\"") else { return value }
        var bytes: [UInt8] = []
        var input = Array(value.utf8.dropFirst().dropLast())[...]
        while let byte = input.popFirst() {
            guard byte == UInt8(ascii: "\\"), let next = input.popFirst() else { bytes.append(byte); continue }
            switch next {
            case UInt8(ascii: "n"): bytes.append(0x0A)
            case UInt8(ascii: "t"): bytes.append(0x09)
            case UInt8(ascii: "r"): bytes.append(0x0D)
            case UInt8(ascii: "a"): bytes.append(0x07)
            case UInt8(ascii: "b"): bytes.append(0x08)
            case UInt8(ascii: "f"): bytes.append(0x0C)
            case UInt8(ascii: "v"): bytes.append(0x0B)
            case UInt8(ascii: "0")...UInt8(ascii: "7"):
                var octal = Int(next - UInt8(ascii: "0"))
                for _ in 0..<2 {
                    guard let digit = input.first, (UInt8(ascii: "0")...UInt8(ascii: "7")).contains(digit) else { break }
                    input.removeFirst()
                    octal = octal * 8 + Int(digit - UInt8(ascii: "0"))
                }
                bytes.append(UInt8(truncatingIfNeeded: octal))
            default: bytes.append(next)
            }
        }
        return String(decoding: bytes, as: UTF8.self)
    }
}

extension GitClient {
    /// Blames each line of `path` in `revision`, or in the working file when `revision` is nil.
    public func blame(path: String, revision: String? = nil, ignoreWhitespace: Bool = false, in repositoryURL: URL) throws -> [GitBlameLine] {
        var arguments = ["blame", "--porcelain", "--no-progress"]
        if ignoreWhitespace { arguments.append("-w") }
        if let revision {
            // Blame parses its own arguments, so pass a resolved object ID rather than user text.
            arguments.append(try run(["rev-parse", "--verify", "--end-of-options", revision + "^{commit}"], in: repositoryURL)
                .trimmingCharacters(in: .whitespacesAndNewlines))
        }
        let output = try run(arguments + ["--", path], in: repositoryURL)
        guard !output.contains("\0") else {
            throw GitClientError.commandFailed(command: "blame", message: "This file is binary, so it has no lines to blame.")
        }
        return GitBlameParser.parse(output)
    }
}
