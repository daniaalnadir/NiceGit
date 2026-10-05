import NiceGitCore
import SwiftUI

struct BlameRequest: Identifiable {
    let id = UUID()
    let path: String
    let repositoryURL: URL
    /// The commit to blame, or nil for the working file.
    var revision: String?
    var revisionLabel: String?
    /// A one-based line to scroll to and highlight, such as a search match.
    var focusLine: Int?
}

/// Each line of a file beside the commit that last changed it. Newer changes are tinted more
/// strongly; selecting a line shows that commit's change to the file.
struct BlameView: View {
    let request: BlameRequest
    @Environment(\.dismiss) private var dismiss
    @State private var lines: [GitBlameLine] = []
    @State private var loading = true
    @State private var error: String?
    @State private var ignoreWhitespace = false
    @State private var selected: GitBlameCommit?
    @State private var hovered: String?
    private let gutterWidth: CGFloat = 250

    var body: some View {
        let ages = ageRanks
        VStack(spacing: 0) {
            HStack(spacing: 12) {
                VStack(alignment: .leading, spacing: 3) {
                    Text("Blame").font(.headline)
                    Text(request.path + (request.revisionLabel.map { " · \($0)" } ?? " · working file"))
                        .font(.system(size: 11, design: .monospaced)).foregroundStyle(.secondary)
                        .lineLimit(1).truncationMode(.middle).help(request.path)
                }
                Spacer()
                Toggle("Ignore whitespace", isOn: $ignoreWhitespace).toggleStyle(.checkbox)
                    .help("Attribute lines past whitespace-only changes to the commit that changed their content")
                Button("Done") { dismiss() }.keyboardShortcut(.defaultAction)
            }.padding()
            Divider()
            HSplitView {
                Group {
                    if loading {
                        ProgressView().frame(maxWidth: .infinity, maxHeight: .infinity)
                    } else if let error {
                        Text(error).foregroundStyle(.red).padding().frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
                    } else if lines.isEmpty {
                        Text("This file is empty.").foregroundStyle(.secondary).frame(maxWidth: .infinity, maxHeight: .infinity)
                    } else {
                        GeometryReader { geometry in
                            ScrollViewReader { proxy in
                                ScrollView([.vertical, .horizontal]) {
                                    LazyVStack(alignment: .leading, spacing: 0) {
                                        ForEach(lines.indices, id: \.self) { index in
                                            // Rows fill the view so separators and highlights span it.
                                            row(index, ages: ages).frame(minWidth: geometry.size.width, alignment: .leading)
                                                .overlay {
                                                    if lines[index].number == request.focusLine { Color.yellow.opacity(0.18).allowsHitTesting(false) }
                                                }
                                                .id(lines[index].number)
                                        }
                                    }.padding(.vertical, 6)
                                }
                                .onAppear {
                                    // Scroll vertically only, keeping the commit column in view.
                                    if let line = request.focusLine { proxy.scrollTo(line, anchor: UnitPoint(x: 0, y: 0.3)) }
                                }
                            }
                        }
                    }
                }.frame(minWidth: 480, maxWidth: .infinity, maxHeight: .infinity)
                    .background(AppPalette.panel)
                if let selected, !selected.isUncommitted {
                    DiffView(selection: DiffSelection(title: selected.summary, repositoryURL: request.repositoryURL,
                                                      path: selected.path.isEmpty ? request.path : selected.path, commitHash: selected.hash),
                             onClose: { self.selected = nil })
                        .id(selected.hash)
                        .frame(minWidth: 380, idealWidth: 480, maxWidth: .infinity, maxHeight: .infinity)
                }
            }
        }
        .frame(minWidth: 960, minHeight: 600)
        .task(id: ignoreWhitespace) { await load() }
    }

    /// Newest commits rank 1, oldest 0, so the tint reads as recency rather than absolute age.
    private var ageRanks: [String: Double] {
        let dates = Dictionary(lines.map { ($0.commit.hash, $0.commit.date ?? .distantPast) }, uniquingKeysWith: { first, _ in first })
        let ordered = dates.sorted { $0.value < $1.value }.map(\.key)
        guard ordered.count > 1 else { return Dictionary(uniqueKeysWithValues: ordered.map { ($0, 1) }) }
        return Dictionary(uniqueKeysWithValues: ordered.enumerated().map { ($1, Double($0) / Double(ordered.count - 1)) })
    }

    private func row(_ index: Int, ages: [String: Double]) -> some View {
        let line = lines[index]
        let commit = line.commit
        let startsGroup = index == 0 || lines[index - 1].commit.hash != commit.hash
        let isSelected = selected?.hash == commit.hash
        let accent = commit.isUncommitted ? AppPalette.conflict : AppPalette.signal
        return HStack(spacing: 0) {
            accent.opacity(commit.isUncommitted ? 0.9 : 0.2 + 0.7 * (ages[commit.hash] ?? 0)).frame(width: 4)
            Group {
                if startsGroup {
                    HStack(spacing: 8) {
                        Text(commit.isUncommitted ? "Uncommitted" : commit.shortHash)
                            .font(.system(size: 11, design: .monospaced)).foregroundStyle(.secondary)
                        Text(commit.isUncommitted ? "Your working changes" : commit.authorName)
                            .font(.system(size: 11, weight: .medium)).lineLimit(1)
                        Spacer(minLength: 4)
                        if let date = commit.date, !commit.isUncommitted {
                            Text(date, format: .relative(presentation: .named)).font(.system(size: 10)).foregroundStyle(.secondary).lineLimit(1)
                        }
                    }
                } else { Color.clear }
            }
            .padding(.horizontal, 8).frame(width: gutterWidth, alignment: .leading)
            Text("\(line.number)").font(.system(size: 11, design: .monospaced)).foregroundStyle(.tertiary)
                .frame(width: 48, alignment: .trailing).padding(.trailing, 10)
            Text(displayed(line.content)).font(.system(size: 12, design: .monospaced))
                .lineLimit(1).fixedSize().textSelection(.enabled)
            Spacer(minLength: 16)
        }
        .frame(height: 20)
        .overlay(alignment: .top) { if startsGroup && index > 0 { Divider().opacity(0.6) } }
        .background(accent.opacity(isSelected ? 0.16 : hovered == commit.hash ? 0.07 : 0))
        .contentShape(Rectangle())
        .onTapGesture { selected = commit.isUncommitted ? nil : commit }
        .onHover { inside in
            if inside { hovered = commit.hash } else if hovered == commit.hash { hovered = nil }
        }
        .help(commit.isUncommitted ? "Not committed yet" : "\(commit.summary)\n\(commit.authorName) <\(commit.authorEmail)> · \(commit.shortHash)")
        .accessibilityElement(children: .combine)
        .accessibilityLabel("Line \(line.number), \(commit.isUncommitted ? "not committed" : "\(commit.authorName), \(commit.summary)"): \(line.content)")
        .accessibilityAddTraits(isSelected ? .isSelected : [])
    }

    /// Tabs keep their width and a trailing carriage return from CRLF files stays invisible.
    private func displayed(_ content: String) -> String {
        (content.hasSuffix("\r") ? String(content.dropLast()) : content).replacingOccurrences(of: "\t", with: "    ")
    }

    private func load() async {
        loading = true
        error = nil
        let path = request.path, url = request.repositoryURL, revision = request.revision, ignoreWhitespace = ignoreWhitespace
        let control = GitCommandControl()
        do {
            let loaded = try await withTaskCancellationHandler {
                try await Task.detached {
                    try GitClient(control: control).blame(path: path, revision: revision, ignoreWhitespace: ignoreWhitespace, in: url)
                }.value
            } onCancel: { control.cancel() }
            guard !Task.isCancelled else { return }
            lines = loaded
            if let selected, !loaded.contains(where: { $0.commit.hash == selected.hash }) { self.selected = nil }
        } catch { if !Task.isCancelled { self.error = error.localizedDescription } }
        if !Task.isCancelled { loading = false }
    }
}
