public struct GitInlineChange: Equatable, Sendable {
    public let prefix: String
    public let changed: String
    public let suffix: String

    public init(prefix: String, changed: String, suffix: String) {
        self.prefix = prefix
        self.changed = changed
        self.suffix = suffix
    }

    public static func highlights(in lines: [GitDiffLine]) -> [Int: Self] {
        var result: [Int: Self] = [:]
        var removed: [Int] = []
        var added: [Int] = []

        func flush() {
            for offset in 0..<min(removed.count, added.count) {
                let old = removed[offset]
                let new = added[offset]
                if let pair = changedParts(old: String(lines[old].text.dropFirst()), new: String(lines[new].text.dropFirst())) {
                    result[old] = pair.old
                    result[new] = pair.new
                }
            }
            removed.removeAll(keepingCapacity: true)
            added.removeAll(keepingCapacity: true)
        }

        for index in lines.indices {
            switch lines[index].kind {
            case .deletion: removed.append(index)
            case .addition: added.append(index)
            default: flush()
            }
        }
        flush()
        return result
    }

    private static func changedParts(old: String, new: String) -> (old: Self, new: Self)? {
        let before = Array(old)
        let after = Array(new)
        let prefixCount = zip(before, after).prefix(while: ==).count
        let suffixCount = zip(before.dropFirst(prefixCount).reversed(), after.dropFirst(prefixCount).reversed())
            .prefix(while: ==).count
        let shared = before.prefix(prefixCount) + before.suffix(suffixCount)
        guard shared.filter({ !$0.isWhitespace }).count >= 2 else { return nil }

        func parts(_ characters: [Character]) -> Self {
            Self(
                prefix: String(characters.prefix(prefixCount)),
                changed: String(characters.dropFirst(prefixCount).dropLast(suffixCount)),
                suffix: String(characters.suffix(suffixCount))
            )
        }
        return (parts(before), parts(after))
    }
}
