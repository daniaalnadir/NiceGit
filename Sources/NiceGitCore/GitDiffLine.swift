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
            if text.hasPrefix(" "), oldRemaining > 0, newRemaining > 0 {
                old = oldValue + 1
                new = newValue + 1
                oldRemaining -= 1
                newRemaining -= 1
                return Self(text: text, kind: .context, oldNumber: oldValue, newNumber: newValue)
            }
            return Self(text: text, kind: .metadata, oldNumber: nil, newNumber: nil)
        }
    }
}
