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
        ScrollViewReader { proxy in
        VStack(spacing: 0) {
            HStack {
                VStack(alignment: .leading, spacing: 4) {
                    Text(selection.title).font(.headline).lineLimit(1)
                    Text(selection.stashHash != nil ? "Stashed changes" : selection.commitHash == nil ? (selection.staged ? "Staged changes" : "Working tree") : "Commit details")
                        .font(.caption).foregroundStyle(.secondary)
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
                ScrollView([.horizontal, .vertical]) {
                    LazyVStack(alignment: .leading, spacing: 0) {
                        ForEach(lines.indices, id: \.self) { index in
                            HStack(spacing: 0) {
                                Text(lines[index].oldNumber.map(String.init) ?? "")
                                    .frame(width: 48, alignment: .trailing)
                                    .foregroundStyle(.secondary)
                                Text(lines[index].newNumber.map(String.init) ?? "")
                                    .frame(width: 48, alignment: .trailing)
                                    .foregroundStyle(.secondary)
                                Text(lines[index].text.isEmpty ? " " : lines[index].text)
                                    .padding(.leading, 16)
                            }
                                .font(.system(size: 12, design: .monospaced))
                                .textSelection(.enabled)
                                .fixedSize(horizontal: true, vertical: false)
                                .padding(.horizontal, 12).padding(.vertical, 2)
                                .frame(maxWidth: .infinity, alignment: .leading)
                                .background(lineColor(lines[index]))
                                .overlay(alignment: .leading) {
                                    if selectedMatch == index {
                                        Rectangle().fill(.yellow).frame(width: 3)
                                    }
                                }
                                .id(index)
                        }
                    }.fixedSize(horizontal: true, vertical: false)
                        .frame(minWidth: geometry.size.width, minHeight: geometry.size.height, alignment: .topLeading)
                }
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
            do {
                let text = try await Task.detached {
                    let git = GitClient(control: commandControl)
                    if let stashHash { return try git.stashDiff(hash: stashHash, in: url) }
                    if let hash { return try git.commitDiff(hash: hash, path: path, in: url) }
                    return try git.diff(path: path ?? "", staged: staged, untracked: untracked, originalPath: original, in: url)
                }.value
                let parsed = GitDiffLine.parse(text)
                lines = onClose != nil ? parsed.filter { $0.kind != .metadata || $0.text == "\\ No newline at end of file" } : parsed
                hasNonTextChanges = onClose != nil && lines.isEmpty && !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            } catch { self.error = error.localizedDescription }
            loading = false
        }
        }
    }

    private func updateMatches() {
        matches = query.isEmpty ? [] : lines.indices.filter { lines[$0].text.localizedCaseInsensitiveContains(query) }
        matchPosition = 0
    }

    private func moveMatch(_ step: Int) {
        guard !matches.isEmpty else { return }
        matchPosition = (matchPosition + step + matches.count) % matches.count
    }

    private func lineColor(_ line: GitDiffLine) -> Color {
        if line.kind == .addition { return .green.opacity(0.14) }
        if line.kind == .deletion { return .red.opacity(0.14) }
        if line.kind == .hunk { return .blue.opacity(0.12) }
        return .clear
    }
}
