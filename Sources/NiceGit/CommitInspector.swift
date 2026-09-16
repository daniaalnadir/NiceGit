import NiceGitCore
import SwiftUI

struct CommitInspector: View {
    let commit: GitCommit
    let repositoryURL: URL
    let showWorkingTree: () -> Void
    @EnvironmentObject private var model: AppModel
    @State private var files: [String] = []
    @State private var loading = false
    @State private var error: String?
    @State private var message = ""

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
                Text(commit.relativeDate).font(.caption).foregroundStyle(.secondary)
                Text(commit.hash).font(.system(size: 10, design: .monospaced)).textSelection(.enabled)
                Button("View full patch") { model.inspect(commit) }
            }.padding(16)
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
                    ForEach(files, id: \.self) { path in
                        Button {
                            model.diffSelection = DiffSelection(title: path, repositoryURL: repositoryURL, path: path, commitHash: commit.hash)
                        } label: {
                            Label(path, systemImage: "doc.text")
                                .font(.system(size: 12)).lineLimit(2)
                                .frame(maxWidth: .infinity, alignment: .leading).padding(.horizontal, 16).padding(.vertical, 10)
                        }.buttonStyle(.plain).help(path)
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
                let hash = commit.hash
                let url = repositoryURL
                let control = GitCommandControl()
                do {
                    let loaded = try await withTaskCancellationHandler {
                        try Task.checkCancellation()
                        return try await Task.detached {
                            let git = GitClient(control: control)
                            return (try git.commitFiles(hash: hash, in: url), try git.commitMessage(hash: hash, in: url))
                        }.value
                    } onCancel: {
                        control.cancel()
                    }
                    guard !Task.isCancelled else { return }
                    files = Array(Set(loaded.0)).sorted()
                    message = loaded.1.trimmingCharacters(in: .newlines)
                } catch { if !Task.isCancelled { self.error = error.localizedDescription } }
                if !Task.isCancelled { loading = false }
            }
    }

    private struct LookupID: Hashable {
        let repositoryURL: URL
        let hash: String
    }
}
