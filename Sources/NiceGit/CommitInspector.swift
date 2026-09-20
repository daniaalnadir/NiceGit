import NiceGitCore
import SwiftUI

struct CommitInspector: View {
    let commit: GitCommit
    let repositoryURL: URL
    let showFile: (DiffSelection) -> Void
    let showWorkingTree: () -> Void
    @EnvironmentObject private var model: AppModel
    @State private var files: [GitCommitFileChange] = []
    @State private var loading = false
    @State private var error: String?
    @State private var message = ""
    @State private var hoveredFile: String?
    @State private var selectedFile: String?
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack {
                Text("Commit").font(.headline)
                Spacer()
                Button(action: showWorkingTree) { Image(systemName: "folder") }.help("Show working tree")
            }.padding(16)
            Divider()
            VStack(alignment: .leading, spacing: 12) {
                ScrollView {
                    Text(message.isEmpty ? commit.subject : message)
                        .font(.system(size: 14))
                        .textSelection(.enabled)
                        .frame(maxWidth: .infinity, alignment: .leading)
                }.frame(maxHeight: 160)
                Label(commit.authorName, systemImage: "person.crop.circle").font(.subheadline)
                Text(commit.authorEmail).font(.caption).foregroundStyle(.secondary).textSelection(.enabled)
                Text(commit.commitDate?.formatted(date: .abbreviated, time: .shortened) ?? commit.relativeDate).font(.caption).foregroundStyle(.secondary)
                Text("Parents: " + (commit.parents.isEmpty ? "None (initial commit)" : commit.parents.map { String($0.prefix(7)) }.joined(separator: ", ")))
                    .font(.caption.monospaced()).foregroundStyle(.secondary).textSelection(.enabled)
                Text(commit.hash).font(.system(size: 10, design: .monospaced)).textSelection(.enabled)
                Button("View full patch") { model.inspect(commit) }
            }.padding(16)
            ViewThatFits(in: .horizontal) {
                HStack(spacing: 12) { changeCounts }
                VStack(alignment: .leading, spacing: 6) { changeCounts }
            }.padding(.horizontal, 16).padding(.bottom, 12)
            Divider()
            HStack {
                Text("Changed files").font(.subheadline.weight(.semibold))
                Spacer()
                Text("\(files.count)").font(.caption.monospaced())
            }.padding(16)
            if loading { ProgressView().frame(maxWidth: .infinity).padding() }
            if let error { Text(error).foregroundStyle(.red).padding(16) }
            ScrollView {
                LazyVStack(alignment: .leading, spacing: 0) {
                    ForEach(files) { file in
                        Button {
                            selectedFile = file.path
                            showFile(DiffSelection(title: file.path, repositoryURL: repositoryURL, path: file.path, commitHash: commit.hash))
                        } label: {
                            HStack(alignment: .top, spacing: 8) {
                                Image(systemName: file.status == "A" ? "plus" : file.status == "D" ? "minus" : "pencil")
                                    .foregroundStyle(file.status == "A" ? Color.green : file.status == "D" ? Color.red : Color.yellow)
                                Text(file.path)
                            }
                                .font(.system(size: 12)).lineLimit(2)
                                .frame(maxWidth: .infinity, alignment: .leading).padding(.horizontal, 16).padding(.vertical, 10)
                                .background(AppPalette.signal.opacity(selectedFile == file.path ? 0.20 : hoveredFile == file.path ? 0.10 : 0))
                                .contentShape(Rectangle())
                        }.buttonStyle(.plain).help(file.path)
                            .onHover { inside in
                                if inside { hoveredFile = file.path }
                                else if hoveredFile == file.path { hoveredFile = nil }
                            }
                            .animation(reduceMotion ? nil : .easeInOut(duration: 0.12), value: hoveredFile)
                            .accessibilityAddTraits(selectedFile == file.path ? .isSelected : [])
                    }
                    if !loading && files.isEmpty && error == nil {
                        Text("No file changes").foregroundStyle(.secondary).padding(16)
                    }
                }
            }
        }.background(AppPalette.panel)
            .task(id: LookupID(repositoryURL: repositoryURL, hash: commit.hash)) {
                loading = true
                error = nil
                files = []
                message = ""
                selectedFile = nil
                hoveredFile = nil
                let hash = commit.hash
                let url = repositoryURL
                let control = GitCommandControl()
                do {
                    let loaded = try await withTaskCancellationHandler {
                        try Task.checkCancellation()
                        return try await Task.detached {
                            let git = GitClient(control: control)
                            return (try git.commitFileChanges(hash: hash, in: url), try git.commitMessage(hash: hash, in: url))
                        }.value
                    } onCancel: {
                        control.cancel()
                    }
                    guard !Task.isCancelled else { return }
                    files = loaded.0
                    message = loaded.1.trimmingCharacters(in: .newlines)
                } catch { if !Task.isCancelled { self.error = error.localizedDescription } }
                if !Task.isCancelled { loading = false }
            }
    }

    private struct LookupID: Hashable {
        let repositoryURL: URL
        let hash: String
    }

    private var changeCounts: some View {
        Group {
            Label("\(files.filter { $0.status == "A" }.count) added", systemImage: "plus").foregroundStyle(.green)
            Label("\(files.filter { $0.status == "D" }.count) removed", systemImage: "minus").foregroundStyle(.red)
            Label("\(files.filter { $0.status != "A" && $0.status != "D" }.count) modified", systemImage: "pencil").foregroundStyle(.yellow)
        }.font(.system(size: 11))
    }
}
