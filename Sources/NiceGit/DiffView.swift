import AppKit
import NiceGitCore
import SwiftUI

struct DiffSelection: Identifiable {
    let id = UUID()
    let title: String
    let repositoryURL: URL
    var path: String?
    var staged = false
    var untracked = false
    var commitHash: String?
    var conflicted = false
    var originalPath: String?
    var stashHash: String?
    /// With `compareFrom`, shows `path` changing from that commit to `commitHash`, or to the
    /// working file when `commitHash` is nil.
    var compareFrom: String?
}

struct DiffView: View {
    let selection: DiffSelection
    var onClose: (() -> Void)? = nil
    @Environment(\.dismiss) private var dismiss
    @Environment(\.colorScheme) private var colorScheme
    @State private var lines: [GitDiffLine] = []
    @State private var error: String?
    @State private var loading = true
    @State private var hasNonTextChanges = false
    @State private var control = GitCommandControl()
    @AppStorage("NiceGit.splitDiff") private var splitDiff = false
    @AppStorage("NiceGit.diffIgnoresWhitespace") private var ignoreWhitespace = false
    @State private var query = ""
    @State private var matches: [Int] = []
    @State private var matchPosition = 0
    @FocusState private var searchFocused: Bool

    private var selectedMatch: Int? {
        matches.indices.contains(matchPosition) ? matches[matchPosition] : nil
    }

    var body: some View {
        let highlights = GitInlineChange.highlights(in: lines)
        return ScrollViewReader { proxy in
        VStack(spacing: 0) {
            HStack {
                VStack(alignment: .leading, spacing: 4) {
                    Text(selection.title).font(.headline).lineLimit(1)
                    if onClose == nil {
                        Text(selection.stashHash != nil ? "Stashed changes" : selection.commitHash == nil ? (selection.staged ? "Staged changes" : "Working tree") : "Commit details")
                            .font(.caption).foregroundStyle(.secondary)
                    }
                }
                Spacer()
                Button { if let onClose { onClose() } else { dismiss() } } label: { Image(systemName: "xmark") }
                    .help("Close diff")
                    .keyboardShortcut(.cancelAction)
            }.padding()
            Divider()
            HStack(spacing: 10) {
                Image(systemName: "magnifyingglass").foregroundStyle(.secondary)
                TextField("Find in diff", text: $query)
                    .textFieldStyle(.plain).focused($searchFocused)
                    .onSubmit { moveMatch(1) }
                if !query.isEmpty {
                    Text(matches.isEmpty ? "No matches" : "\(matchPosition + 1) of \(matches.count) lines")
                        .font(.caption.monospacedDigit()).foregroundStyle(.secondary)
                }
                Button { moveMatch(-1) } label: { Image(systemName: "chevron.up") }
                    .help("Previous matching line").disabled(matches.isEmpty)
                Button { moveMatch(1) } label: { Image(systemName: "chevron.down") }
                    .help("Next matching line").disabled(matches.isEmpty)
                Button { searchFocused = true } label: { Image(systemName: "text.magnifyingglass") }
                    .help("Find in diff").keyboardShortcut("f", modifiers: .command)
                Toggle(isOn: $ignoreWhitespace) { Image(systemName: "space") }
                    .toggleStyle(.button)
                    .help(ignoreWhitespace ? "Showing changes other than whitespace" : "Hide whitespace-only changes")
                    .accessibilityLabel("Ignore whitespace")
                Button { splitDiff.toggle() } label: { Image(systemName: splitDiff ? "rectangle.split.2x1.fill" : "rectangle.split.2x1") }
                    .help(splitDiff ? "Show changes in one column" : "Show old and new side by side")
                    .accessibilityLabel(splitDiff ? "Unified diff" : "Side-by-side diff")
            }.padding(.horizontal, 16).padding(.vertical, 8)
                .disabled(loading)
            Divider()
            if let path = selection.path, selection.stashHash == nil, !selection.conflicted, ImageComparisonView.isImage(path) {
                ImageComparisonView(selection: selection)
            } else if loading {
                ProgressView().frame(maxWidth: .infinity, maxHeight: .infinity)
            } else if let error {
                ContentUnavailableView("Unable to load diff", systemImage: "exclamationmark.triangle", description: Text(error))
            } else if lines.isEmpty {
                ContentUnavailableView(hasNonTextChanges ? "No text hunks" : "No differences", systemImage: hasNonTextChanges ? "doc" : "checkmark.circle",
                    description: hasNonTextChanges ? Text("This file has binary or file-property changes.") : nil)
            } else {
                GeometryReader { geometry in
                let font = NSFont.monospacedSystemFont(ofSize: 12, weight: .regular)
                let textWidth = lines.map { line in
                    let code = [.addition, .deletion, .context].contains(line.kind) ? String(line.text.dropFirst()) : line.text
                    return (code as NSString).size(withAttributes: [.font: font]).width
                }.max() ?? 0
                let contentWidth = max(geometry.size.width, ceil(textWidth) + 142)
                if splitDiff {
                    // Both sides always fit; long lines wrap instead of pushing the new side off-screen.
                    splitContent(width: floor(geometry.size.width / 2), minHeight: geometry.size.height, highlights: highlights)
                } else {
                ScrollView([.horizontal, .vertical]) {
                    LazyVStack(alignment: .leading, spacing: 0) {
                        ForEach(lines.indices, id: \.self) { index in
                            if lines[index].kind == .hunk {
                                Color.clear.frame(height: index == 0 ? 12 : 28)
                                HStack {
                                    Text(hunkLabel(lines[index].text))
                                        .font(.system(size: 12, weight: .semibold, design: .monospaced))
                                        .foregroundStyle(.secondary)
                                    Spacer(minLength: 0)
                                }
                                .padding(.horizontal, 12)
                                .frame(width: contentWidth, height: 30)
                                .background(AppPalette.toolbar)
                                .overlay(alignment: .bottom) { AppPalette.line.frame(height: 1) }
                                .id(index)
                            } else {
                            HStack(spacing: 0) {
                                Text(lines[index].oldNumber.map(String.init) ?? "")
                                    .frame(width: 40, alignment: .trailing)
                                    .foregroundStyle(.secondary)
                                Text(lines[index].newNumber.map(String.init) ?? "")
                                    .frame(width: 40, alignment: .trailing)
                                    .foregroundStyle(.secondary)
                                Text(marker(for: lines[index]))
                                    .frame(width: 24, alignment: .trailing)
                                    .foregroundStyle(.secondary)
                                InlineDiffText(line: lines[index], change: highlights[index], path: selection.path)
                                    .padding(.leading, 8)
                                Spacer(minLength: 0)
                            }
                                .font(.system(size: 12, design: .monospaced))
                                .textSelection(.enabled)
                                .frame(width: contentWidth, height: 25, alignment: .leading)
                                .background(DiffHighlight.row(for: lines[index].kind, scheme: colorScheme))
                                .overlay(alignment: .leading) {
                                    AppPalette.line.opacity(0.7).frame(width: 1).offset(x: 105)
                                }
                                .overlay(alignment: .leading) {
                                    if selectedMatch == index {
                                        Rectangle().fill(.yellow).frame(width: 3)
                                    }
                                }
                                .id(index)
                            }
                        }
                    }.frame(width: contentWidth, alignment: .leading)
                        .frame(minHeight: geometry.size.height, alignment: .topLeading)
                }
                .scrollIndicators(.visible, axes: .horizontal)
                }
                }
            }
        }
        .onChange(of: query) { updateMatches() }
        .onChange(of: selectedMatch) { _, index in
            guard let index else { return }
            if splitDiff {
                // Split rows are identified by row, so find the row that shows this line.
                let rows = GitDiffLine.sideBySide(lines)
                if let row = rows.firstIndex(where: {
                    switch $0 {
                    case let .banner(line): line == index
                    case let .pair(left, right): left == index || right == index
                    }
                }) { proxy.scrollTo("split-\(row)", anchor: .center) }
            } else {
                proxy.scrollTo(index, anchor: .center)
            }
        }
        .frame(width: onClose == nil ? 900 : nil, height: onClose == nil ? 620 : nil)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .onDisappear { control.cancel() }
        .task(id: ignoreWhitespace) {
            // Reloads when whitespace handling changes, so clear the previous result first.
            error = nil
            loading = true
            let url = selection.repositoryURL
            let path = selection.path
            let hash = selection.commitHash
            let staged = selection.staged
            let untracked = selection.untracked
            let original = selection.originalPath
            let stashHash = selection.stashHash
            let compareFrom = selection.compareFrom
            let ignoreWhitespace = ignoreWhitespace
            let commandControl = control
            let embedded = onClose != nil
            do {
                let text = try await Task.detached {
                    let git = GitClient(control: commandControl)
                    if let stashHash { return try git.stashDiff(hash: stashHash, ignoreWhitespace: ignoreWhitespace, in: url) }
                    if let compareFrom, let path {
                        return try git.compareFileDiff(from: compareFrom, to: hash, path: path, ignoreWhitespace: ignoreWhitespace, in: url)
                    }
                    if let hash, let path, embedded {
                        return try git.commitFileDiff(hash: hash, path: path, ignoreWhitespace: ignoreWhitespace, in: url)
                    }
                    if let hash { return try git.commitDiff(hash: hash, path: path, ignoreWhitespace: ignoreWhitespace, in: url) }
                    return try git.diff(path: path ?? "", staged: staged, untracked: untracked, originalPath: original, ignoreWhitespace: ignoreWhitespace, in: url)
                }.value
                guard !Task.isCancelled else { return }
                lines = embedded ? GitDiffLine.codeOnly(text) : GitDiffLine.parse(text)
                updateMatches()
                hasNonTextChanges = onClose != nil && lines.isEmpty && !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            } catch { self.error = error.localizedDescription }
            loading = false
        }
        }
    }

    private func updateMatches() {
        matches = query.isEmpty ? [] : lines.indices.filter {
            (onClose == nil || lines[$0].kind != .hunk) && lines[$0].text.localizedCaseInsensitiveContains(query)
        }
        matchPosition = 0
    }

    private func moveMatch(_ step: Int) {
        guard !matches.isEmpty else { return }
        matchPosition = (matchPosition + step + matches.count) % matches.count
    }

    /// Old lines on the left and new lines on the right; each side keeps its own line numbers.
    private func splitContent(width half: CGFloat, minHeight: CGFloat, highlights: [Int: GitInlineChange]) -> some View {
        let rows = GitDiffLine.sideBySide(lines)
        return ScrollView(.vertical) {
            LazyVStack(alignment: .leading, spacing: 0) {
                ForEach(rows.indices, id: \.self) { row in
                    Group {
                        switch rows[row] {
                        case let .banner(index):
                            HStack {
                                Text(lines[index].kind == .hunk ? hunkLabel(lines[index].text) : lines[index].text)
                                    .font(.system(size: 12, weight: lines[index].kind == .hunk ? .semibold : .regular, design: .monospaced))
                                    .foregroundStyle(.secondary).lineLimit(1)
                                Spacer(minLength: 0)
                            }
                            .padding(.horizontal, 12)
                            .frame(width: half * 2, height: lines[index].kind == .hunk ? 30 : 22)
                            .background(lines[index].kind == .hunk ? AppPalette.toolbar : Color.clear)
                            .padding(.top, lines[index].kind == .hunk && row > 0 ? 16 : 0)
                        case let .pair(left, right):
                            HStack(spacing: 0) {
                                splitSide(left, number: left.flatMap { lines[$0].oldNumber }, width: half, highlights: highlights)
                                AppPalette.line.frame(width: 1)
                                splitSide(right, number: right.flatMap { lines[$0].newNumber }, width: half - 1, highlights: highlights)
                            }.fixedSize(horizontal: false, vertical: true)
                        }
                    }.id("split-\(row)")
                }
            }.frame(width: half * 2, alignment: .leading).frame(minHeight: minHeight, alignment: .topLeading)
        }
    }

    @ViewBuilder
    private func splitSide(_ index: Int?, number: Int?, width: CGFloat, highlights: [Int: GitInlineChange]) -> some View {
        if let index {
            HStack(alignment: .firstTextBaseline, spacing: 0) {
                Text(number.map(String.init) ?? "").frame(width: 40, alignment: .trailing).foregroundStyle(.secondary)
                Text(marker(for: lines[index])).frame(width: 20, alignment: .trailing).foregroundStyle(.secondary)
                InlineDiffText(line: lines[index], change: highlights[index], path: selection.path)
                    .fixedSize(horizontal: false, vertical: true).padding(.leading, 8).padding(.trailing, 6)
                Spacer(minLength: 0)
            }
            .font(.system(size: 12, design: .monospaced))
            .textSelection(.enabled)
            .padding(.vertical, 4)
            .frame(width: width, alignment: .topLeading)
            .frame(maxHeight: .infinity, alignment: .top)
            .background(DiffHighlight.row(for: lines[index].kind, scheme: colorScheme))
            .overlay(alignment: .leading) { if selectedMatch == index { Rectangle().fill(.yellow).frame(width: 3) } }
        } else {
            // No counterpart on this side: an addition has no old line, a removal no new one.
            Color.primary.opacity(0.04).frame(width: width).frame(maxHeight: .infinity)
        }
    }

    private func marker(for line: GitDiffLine) -> String {
        switch line.kind {
        case .addition: "+"
        case .deletion: "-"
        default: ""
        }
    }

    private func hunkLabel(_ text: String) -> String {
        guard text.hasPrefix("@@"),
              let end = text.range(of: "@@", range: text.index(text.startIndex, offsetBy: 2)..<text.endIndex) else {
            return text
        }
        return String(text[..<end.upperBound])
    }

}
