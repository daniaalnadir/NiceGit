import NiceGitCore
import SwiftUI

struct CompareRequest: Identifiable {
    let id = UUID()
    let repositoryURL: URL
    let older: GitCommit
    /// The newer side, or nil for the working files.
    let newer: GitCommit?
}

/// Files that differ between two commits, or between a commit and the working files, beside the
/// selected file's change.
struct CompareView: View {
    let request: CompareRequest
    @Environment(\.dismiss) private var dismiss
    @State private var files: [GitCommitFileChange] = []
    @State private var loading = true
    @State private var error: String?
    @State private var selected: String?

    var body: some View {
        VStack(spacing: 0) {
            HStack(spacing: 10) {
                VStack(alignment: .leading, spacing: 3) {
                    Text("Compare").font(.headline)
                    Text("\(label(request.older)) → \(request.newer.map(label) ?? "working files")")
                        .font(.system(size: 11)).foregroundStyle(.secondary).lineLimit(1).truncationMode(.middle)
                }
                Spacer()
                if !loading { Text("\(files.count) \(files.count == 1 ? "file" : "files")").font(.caption.monospaced()).foregroundStyle(.secondary) }
                Button("Done") { dismiss() }.keyboardShortcut(.defaultAction)
            }.padding()
            Divider()
            HSplitView {
                ScrollView {
                    LazyVStack(alignment: .leading, spacing: 0) {
                        if loading { ProgressView().frame(maxWidth: .infinity).padding() }
                        if let error { Text(error).foregroundStyle(.red).padding() }
                        if !loading && error == nil && files.isEmpty {
                            Text("No differences in tracked files").foregroundStyle(.secondary).padding()
                        }
                        ForEach(files) { file in
                            Button { selected = file.path } label: {
                                HStack(spacing: 8) {
                                    Image(systemName: file.status == "A" ? "plus" : file.status == "D" ? "minus" : "pencil")
                                        .foregroundStyle(file.status == "A" ? Color.green : file.status == "D" ? Color.red : Color.yellow)
                                        .frame(width: 14)
                                    Text(file.path).font(.system(size: 12)).lineLimit(1).truncationMode(.middle)
                                    Spacer(minLength: 0)
                                }
                                .padding(.horizontal, 12).padding(.vertical, 7)
                                .background(AppPalette.signal.opacity(selected == file.path ? 0.20 : 0))
                                .contentShape(Rectangle())
                            }.buttonStyle(.plain).help(file.path)
                                .accessibilityAddTraits(selected == file.path ? .isSelected : [])
                        }
                    }
                }.frame(minWidth: 240, idealWidth: 300, maxWidth: 420).background(AppPalette.panel)
                Group {
                    if let selected {
                        DiffView(selection: DiffSelection(title: selected, repositoryURL: request.repositoryURL, path: selected,
                                                          commitHash: request.newer?.hash, compareFrom: request.older.hash),
                                 onClose: { self.selected = nil })
                            .id(selected)
                    } else {
                        Text(files.isEmpty ? "" : "Select a file to see how it changed").foregroundStyle(.secondary)
                            .frame(maxWidth: .infinity, maxHeight: .infinity)
                    }
                }.frame(minWidth: 420, maxWidth: .infinity, maxHeight: .infinity)
            }
        }
        .frame(minWidth: 900, minHeight: 560)
        .task(id: request.id) { await load() }
    }

    private func label(_ commit: GitCommit) -> String { "\(commit.shortHash) \(commit.subject)" }

    private func load() async {
        let url = request.repositoryURL, older = request.older.hash, newer = request.newer?.hash
        do {
            let loaded = try await Task.detached { try GitClient().compareFiles(from: older, to: newer, in: url) }.value
            files = loaded
            selected = loaded.first?.path
        } catch { self.error = error.localizedDescription }
        loading = false
    }
}
