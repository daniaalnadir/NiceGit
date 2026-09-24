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
            }.padding(.horizontal, 16).padding(.vertical, 8)
                .disabled(loading)
            Divider()
            if loading {
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
        .onChange(of: query) { updateMatches() }
        .onChange(of: selectedMatch) { _, index in
            if let index { proxy.scrollTo(index, anchor: .center) }
        }
        .frame(width: onClose == nil ? 900 : nil, height: onClose == nil ? 620 : nil)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .onDisappear { control.cancel() }
        .task {
            let url = selection.repositoryURL
            let path = selection.path
            let hash = selection.commitHash
            let staged = selection.staged
            let untracked = selection.untracked
            let original = selection.originalPath
            let stashHash = selection.stashHash
            let commandControl = control
            let embedded = onClose != nil
            do {
                let text = try await Task.detached {
                    let git = GitClient(control: commandControl)
                    if let stashHash { return try git.stashDiff(hash: stashHash, in: url) }
                    if let hash, let path, embedded {
                        return try git.commitFileDiff(hash: hash, path: path, in: url)
                    }
                    if let hash { return try git.commitDiff(hash: hash, path: path, in: url) }
                    return try git.diff(path: path ?? "", staged: staged, untracked: untracked, originalPath: original, in: url)
                }.value
                lines = embedded ? GitDiffLine.codeOnly(text) : GitDiffLine.parse(text)
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
