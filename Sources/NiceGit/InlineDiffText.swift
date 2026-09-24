import Foundation
import NiceGitCore
import SwiftUI

enum DiffHighlight {
    static func row(for kind: GitDiffLine.Kind, scheme: ColorScheme) -> Color {
        switch kind {
        case .addition:
            scheme == .dark ? Color(red: 0.14, green: 0.25, blue: 0.19) : Color(red: 0.87, green: 0.95, blue: 0.90)
        case .deletion:
            scheme == .dark ? Color(red: 0.25, green: 0.16, blue: 0.17) : Color.red.opacity(0.14)
        case .hunk: .blue.opacity(0.12)
        default: .clear
        }
    }

    static func inline(for kind: GitDiffLine.Kind, scheme: ColorScheme) -> Color {
        switch kind {
        case .addition:
            scheme == .dark ? Color(red: 0.12, green: 0.37, blue: 0.24) : Color(red: 0.48, green: 0.72, blue: 0.56)
        case .deletion:
            scheme == .dark ? Color(red: 0.45, green: 0.23, blue: 0.24) : Color.red.opacity(0.24)
        default: .clear
        }
    }
}

struct InlineDiffText: View {
    let line: GitDiffLine
    let change: GitInlineChange?
    var path: String? = nil
    @Environment(\.colorScheme) private var colorScheme

    var body: some View {
        Text(styledText)
    }

    private var styledText: AttributedString {
        let code = line.kind == .context || line.kind == .addition || line.kind == .deletion
            ? String(line.text.dropFirst()) : line.text
        var result = AttributedString(code.isEmpty ? " " : code)
        if path?.lowercased().hasSuffix(".swift") == true {
            for (regex, color) in SwiftCodeColors.rules(for: colorScheme) {
                for match in regex.matches(in: code, range: NSRange(code.startIndex..<code.endIndex, in: code)) {
                    guard let stringRange = Range(match.range, in: code),
                          let lower = AttributedString.Index(stringRange.lowerBound, within: result),
                          let upper = AttributedString.Index(stringRange.upperBound, within: result) else { continue }
                    result[lower..<upper].foregroundColor = color
                }
            }
        }
        if let change, !change.changed.isEmpty {
            let start = code.index(code.startIndex, offsetBy: change.prefix.count)
            let end = code.index(start, offsetBy: change.changed.count)
            if let lower = AttributedString.Index(start, within: result),
               let upper = AttributedString.Index(end, within: result) {
                result[lower..<upper].backgroundColor = DiffHighlight.inline(for: line.kind, scheme: colorScheme)
            }
        }
        return result
    }
}

private enum SwiftCodeColors {
    private static let types = try! NSRegularExpression(pattern: #"\b[A-Z][A-Za-z0-9_]*\b"#)
    private static let keywords = try! NSRegularExpression(pattern: #"\b(?:actor|as|async|await|case|catch|class|enum|extension|false|for|func|guard|if|import|in|init|let|nil|private|protocol|public|return|self|some|static|struct|switch|throw|throws|true|try|var|where|while)\b"#)
    private static let strings = try! NSRegularExpression(pattern: #"\"(?:\\.|[^\"\\])*\""#)
    private static let comments = try! NSRegularExpression(pattern: #"//.*$"#)

    static func rules(for scheme: ColorScheme) -> [(NSRegularExpression, Color)] {
        if scheme == .dark {
            return [(types, Color(red: 0.46, green: 0.80, blue: 0.71)),
                    (keywords, Color(red: 0.48, green: 0.71, blue: 0.92)),
                    (strings, Color(red: 0.86, green: 0.66, blue: 0.54)),
                    (comments, Color(red: 0.53, green: 0.58, blue: 0.63))]
        }
        return [(types, Color(red: 0.07, green: 0.43, blue: 0.38)),
                (keywords, Color(red: 0.11, green: 0.36, blue: 0.65)),
                (strings, Color(red: 0.56, green: 0.27, blue: 0.17)),
                (comments, Color(red: 0.40, green: 0.45, blue: 0.48))]
    }
}
