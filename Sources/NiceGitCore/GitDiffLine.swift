import Foundation

public struct GitDiffLine: Sendable, Equatable {
    public enum Kind: Sendable { case metadata, hunk, context, addition, deletion }
    public let text: String
    public let kind: Kind
    public let oldNumber: Int?
    public let newNumber: Int?

    public static func changesOnly(_ patch: String) -> [GitDiffLine] {
        parse(patch).filter {
            switch $0.kind {
            case .hunk, .addition, .deletion: true
            case .metadata, .context: false
            }
        }
    }

    public static func codeOnly(_ patch: String) -> [GitDiffLine] {
        parse(patch).filter { $0.kind != .metadata }
    }

    public static func parse(_ patch: String) -> [GitDiffLine] {
        guard !patch.isEmpty else { return [] }
        var old: Int?
        var new: Int?
        var oldRemaining = 0
        var newRemaining = 0
        let header = /^@@ -(\d+)(?:,(\d+))? \+(\d+)(?:,(\d+))? @@/
        return patch.components(separatedBy: "\n").map { text in
            if let match = text.prefixMatch(of: header) {
                old = Int(match.1)
                new = Int(match.3)
                oldRemaining = match.2.flatMap { Int($0) } ?? 1
                newRemaining = match.4.flatMap { Int($0) } ?? 1
                return Self(text: text, kind: .hunk, oldNumber: nil, newNumber: nil)
            }
            if text.hasPrefix("diff --git ") || text.hasPrefix("@@@") {
                old = nil
                new = nil
            }
            guard let oldValue = old, let newValue = new,
                  oldRemaining > 0 || newRemaining > 0 else {
                return Self(text: text, kind: .metadata, oldNumber: nil, newNumber: nil)
            }
            if text.hasPrefix("+"), newRemaining > 0 {
                new = newValue + 1
                newRemaining -= 1
                return Self(text: text, kind: .addition, oldNumber: nil, newNumber: newValue)
            }
            if text.hasPrefix("-"), oldRemaining > 0 {
                old = oldValue + 1
                oldRemaining -= 1
                return Self(text: text, kind: .deletion, oldNumber: oldValue, newNumber: nil)
            }
            if text.hasPrefix(" ") || text.isEmpty, oldRemaining > 0, newRemaining > 0 {
                old = oldValue + 1
                new = newValue + 1
                oldRemaining -= 1
                newRemaining -= 1
                // diff.suppressBlankEmpty omits the prefix on empty context lines.
                return Self(text: text.isEmpty ? " " : text, kind: .context, oldNumber: oldValue, newNumber: newValue)
            }
            return Self(text: text, kind: .metadata, oldNumber: nil, newNumber: nil)
        }
    }
}

/// One row of a side-by-side diff, referring to lines by their index in the unified list.
public enum GitSplitRow: Equatable, Sendable {
    /// A hunk header or file metadata, shown across both sides.
    case banner(Int)
    /// The old version's line on the left and the new version's on the right; either may be absent.
    case pair(left: Int?, right: Int?)
}

extension GitDiffLine {
    /// Arranges unified diff lines side by side. Unchanged lines appear on both sides, and each
    /// run of removed lines pairs row by row with the added lines that follow it. Git's
    /// missing-newline note belongs to the change around it, so it does not break the pairing.
    public static func sideBySide(_ lines: [GitDiffLine]) -> [GitSplitRow] {
        var rows: [GitSplitRow] = []
        var index = 0
        let isNote = { (i: Int) in lines[i].kind == .metadata && lines[i].text.hasPrefix("\\") }
        while index < lines.count {
            switch lines[index].kind {
            case .context:
                rows.append(.pair(left: index, right: index))
                index += 1
            case .deletion, .addition:
                var removed: [Int] = [], added: [Int] = [], notes: [Int] = []
                while index < lines.count, lines[index].kind == .deletion || isNote(index) {
                    if isNote(index) { notes.append(index) } else { removed.append(index) }
                    index += 1
                }
                while index < lines.count, lines[index].kind == .addition || isNote(index) {
                    if isNote(index) { notes.append(index) } else { added.append(index) }
                    index += 1
                }
                for row in 0..<max(removed.count, added.count) {
                    rows.append(.pair(left: row < removed.count ? removed[row] : nil, right: row < added.count ? added[row] : nil))
                }
                rows += notes.map(GitSplitRow.banner)
            case .hunk, .metadata:
                rows.append(.banner(index))
                index += 1
            }
        }
        return rows
    }
}
