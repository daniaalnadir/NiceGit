import NiceGitCore
import SwiftUI

struct FileHistoryRequest: Identifiable {
    let id = UUID()
    let path: String
    let repositoryURL: URL
}

/// A restore confirmed against the checkout that was current when the dialog opened.
struct RestoreFileRequest {
    let path: String
    let source: String
    let sourceDescription: String
    let removesFile: Bool
    let branch: String
    let head: String?

    @MainActor static func make(path: String, source: String, sourceDescription: String, removesFile: Bool, model: AppModel) -> RestoreFileRequest? {
        guard canRestore(model), let snapshot = model.snapshot else { return nil }
        return RestoreFileRequest(path: path, source: source, sourceDescription: sourceDescription, removesFile: removesFile,
                                  branch: snapshot.currentBranch, head: snapshot.headHash)
    }

    @MainActor static func canRestore(_ model: AppModel) -> Bool {
        !model.isLoading && model.snapshot != nil && model.snapshot?.operation == nil
    }
}

struct RestoreFileConfirmation: ViewModifier {
    @Binding var request: RestoreFileRequest?
    var onConfirm: () -> Void = {}
    @EnvironmentObject private var model: AppModel

    func body(content: Content) -> some View {
        content.confirmationDialog(request.map { $0.removesFile ? "Delete \($0.path)?" : "Restore \($0.path)?" } ?? "",
                                   isPresented: Binding(get: { request != nil }, set: { if !$0 { request = nil } })) {
            if let request {
                Button(request.removesFile ? "Delete file" : "Restore file", role: .destructive) {
                    model.restore(path: request.path, from: request.source, expectedBranch: request.branch, expectedHead: request.head)
                    onConfirm()
                }
            }
        } message: {
            if let request {
                Text(request.removesFile
                     ? "This file does not exist \(request.sourceDescription). It will be deleted from your working files and the deletion staged. Any uncommitted changes to it will be lost. Your commits and other files are not changed."
                     : "Your working copy of this file will be replaced with its version \(request.sourceDescription), and the change staged. Any staged or unstaged changes to it will be lost. Your commits and other files are not changed.")
            }
        }
    }
}

/// Commits in the current checkout that changed one file, following renames, beside the
/// selected commit's change to that file.
struct FileHistoryView: View {
    let request: FileHistoryRequest
    @EnvironmentObject private var model: AppModel
    @Environment(\.dismiss) private var dismiss
    @State private var entries: [GitFileHistoryEntry] = []
    @State private var loading = true
    @State private var error: String?
    @State private var selected: GitFileHistoryEntry?
    @State private var restoreRequest: RestoreFileRequest?
    @State private var blameRequest: BlameRequest?
    private let limit = 200

    var body: some View {
        VStack(spacing: 0) {
            HStack(spacing: 10) {
                VStack(alignment: .leading, spacing: 3) {
                    Text("File history").font(.headline)
                    Text(request.path).font(.system(size: 11, design: .monospaced)).foregroundStyle(.secondary)
                        .lineLimit(1).truncationMode(.middle).help(request.path)
                }
                Spacer()
                if !loading {
                    Text(entries.count == limit ? "Latest \(limit) commits" : "\(entries.count) \(entries.count == 1 ? "commit" : "commits")")
                        .font(.caption.monospaced()).foregroundStyle(.secondary)
                }
                Button("Done") { dismiss() }.keyboardShortcut(.defaultAction)
            }.padding()
            Divider()
            HSplitView {
                list.frame(minWidth: 280, idealWidth: 340, maxWidth: 480)
                Group {
                    if let selected {
                        DiffView(selection: DiffSelection(title: selected.commit.subject, repositoryURL: request.repositoryURL,
                                                          path: selected.path, commitHash: selected.commit.hash),
                                 onClose: { self.selected = nil })
                            .id(selected.id)
                    } else {
                        Text(entries.isEmpty ? "" : "Select a commit to see how it changed this file")
                            .foregroundStyle(.secondary).frame(maxWidth: .infinity, maxHeight: .infinity)
                    }
                }.frame(minWidth: 420, maxWidth: .infinity, maxHeight: .infinity)
            }
        }
        .frame(minWidth: 900, minHeight: 560)
        .modifier(RestoreFileConfirmation(request: $restoreRequest, onConfirm: { dismiss() }))
        .sheet(item: $blameRequest) { BlameView(request: $0) }
        .task(id: request.id) { await load() }
    }

    private var list: some View {
        ScrollView {
            LazyVStack(alignment: .leading, spacing: 0) {
                if loading { ProgressView().frame(maxWidth: .infinity).padding() }
                if let error { Text(error).foregroundStyle(.red).padding(16) }
                if !loading && error == nil && entries.isEmpty {
                    Text("No commits in this checkout changed this file.").foregroundStyle(.secondary).padding(16)
                }
                ForEach(entries) { entry in
                    Button { selected = entry } label: { row(entry) }
                        .buttonStyle(.plain)
                        .help(entry.commit.subject)
                        .accessibilityAddTraits(selected?.id == entry.id ? .isSelected : [])
                        .contextMenu {
                            Button(entry.deletesFile ? "Restore file to this commit (deletes it)..." : "Restore file to this commit's version...") {
                                requestRestore(entry)
                            }
                            // Git restores by path; a commit before a rename holds the file under another name.
                            .disabled(!RestoreFileRequest.canRestore(model) || entry.path != request.path)
                            Button("Blame at this commit") {
                                blameRequest = BlameRequest(path: entry.path, repositoryURL: request.repositoryURL,
                                                            revision: entry.commit.hash, revisionLabel: "at \(entry.commit.shortHash)")
                            }.disabled(entry.deletesFile)
                            Divider()
                            Button("Copy commit hash") {
                                NSPasteboard.general.clearContents()
                                NSPasteboard.general.setString(entry.commit.hash, forType: .string)
                            }
                        }
                        .accessibilityAction(named: "Restore file to this commit's version") { requestRestore(entry) }
                }
            }
        }.background(AppPalette.panel)
    }

    private func row(_ entry: GitFileHistoryEntry) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            HStack(spacing: 6) {
                Text(entry.commit.subject).font(.system(size: 13)).lineLimit(1)
                Spacer(minLength: 4)
                if let badge = badge(entry) {
                    Text(badge).font(.system(size: 10, weight: .semibold)).foregroundStyle(.secondary)
                        .padding(.horizontal, 5).padding(.vertical, 1)
                        .background(Color.primary.opacity(0.08), in: RoundedRectangle(cornerRadius: 3))
                }
            }
            Text("\(entry.commit.authorName) · \(entry.commit.relativeDate) · \(entry.commit.shortHash)")
                .font(.system(size: 11)).foregroundStyle(.secondary).lineLimit(1)
            if entry.path != request.path {
                Text("As \(entry.path)").font(.system(size: 11, design: .monospaced)).foregroundStyle(.secondary)
                    .lineLimit(1).truncationMode(.middle)
            }
        }
        .padding(.horizontal, 14).padding(.vertical, 9)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(AppPalette.signal.opacity(selected?.id == entry.id ? 0.20 : 0))
        .contentShape(Rectangle())
    }

    private func badge(_ entry: GitFileHistoryEntry) -> String? {
        switch entry.status {
        case "A": "Added"
        case "D": "Deleted"
        case "R": "Renamed"
        case "C": "Copied"
        default: nil
        }
    }

    private func requestRestore(_ entry: GitFileHistoryEntry) {
        guard entry.path == request.path else { return }
        restoreRequest = RestoreFileRequest.make(path: entry.path, source: entry.commit.hash,
                                                 sourceDescription: "in commit \(entry.commit.shortHash)",
                                                 removesFile: entry.deletesFile, model: model)
    }

    private func load() async {
        loading = true
        error = nil
        let path = request.path, url = request.repositoryURL, limit = limit
        let control = GitCommandControl()
        do {
            let loaded = try await withTaskCancellationHandler {
                try await Task.detached { try GitClient(control: control).fileHistory(path: path, limit: limit, in: url) }.value
            } onCancel: { control.cancel() }
            guard !Task.isCancelled else { return }
            entries = loaded
            selected = loaded.first
        } catch { if !Task.isCancelled { self.error = error.localizedDescription } }
        if !Task.isCancelled { loading = false }
    }
}
