import Foundation

public struct GitDiffHunk: Identifiable, Sendable {
    public let lineIndices: Range<Int>
    public let changedIndices: Set<Int>
    public var id: Int { lineIndices.lowerBound }

    public static func grouped(_ lines: [GitDiffLine], context: Int = 3) -> [Self] {
        let context = max(0, context)
        var ranges: [Range<Int>] = []
        for index in lines.indices where lines[index].kind == .addition || lines[index].kind == .deletion {
            var start = index
            var end = index + 1
            var count = 0
            while start > 0 && count < context && lines[start - 1].kind == .context {
                start -= 1
                count += 1
            }
            count = 0
            while end < lines.count && count < context && lines[end].kind == .context {
                end += 1
                count += 1
            }
            // Git's missing-newline marker belongs to the surrounding change block.
            if let previous = ranges.last,
               start <= previous.upperBound || lines[previous.upperBound..<start].allSatisfy({ $0.text == "\\ No newline at end of file" }) {
                ranges[ranges.count - 1] = previous.lowerBound..<max(previous.upperBound, end)
            } else {
                ranges.append(start..<end)
            }
        }
        return ranges.map { range in
            Self(lineIndices: range, changedIndices: Set(range.filter { lines[$0].kind == .addition || lines[$0].kind == .deletion }))
        }
    }
}
