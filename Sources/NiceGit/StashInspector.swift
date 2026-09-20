import NiceGitCore
import SwiftUI

struct StashInspector: View {
    let stash: GitStash
    let repositoryURL: URL
    let showWorkingTree: () -> Void
    @EnvironmentObject private var model: AppModel
    @State private var files: [String] = []
    @State private var loading = true
    @State private var error: String?

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack {
                Label("Stash", systemImage: "archivebox.fill").font(.headline)
                Spacer()
                Button(action: showWorkingTree) { Image(systemName: "folder") }.help("Show working tree")
            }.padding(16)
            Divider()
            VStack(alignment: .leading, spacing: 12) {
                Text(stash.message).font(.system(size: 14)).textSelection(.enabled)
                Text(stash.reference).font(.caption.monospaced()).foregroundStyle(.secondary)
                Button("View full patch") {
                    model.diffSelection = DiffSelection(title: stash.message, repositoryURL: repositoryURL, stashHash: stash.hash)
                }
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
                        Label(path, systemImage: "doc.text").font(.system(size: 12))
                            .textSelection(.enabled).frame(maxWidth: .infinity, alignment: .leading)
                            .padding(.horizontal, 16).padding(.vertical, 10).help(path)
                    }
                    if !loading && files.isEmpty && error == nil {
                        Text("No file changes").foregroundStyle(.secondary).padding(16)
                    }
                }
            }
        }.background(AppPalette.panel)
            .task(id: repositoryURL.path + stash.hash) {
                loading = true
                files = []
                error = nil
                let hash = stash.hash
                let url = repositoryURL
                let control = GitCommandControl()
                do {
                    let result = try await withTaskCancellationHandler {
                        try await Task.detached {
                            try GitClient(control: control).stashFiles(hash: hash, in: url)
                        }.value
                    } onCancel: { control.cancel() }
                    guard !Task.isCancelled else { return }
                    files = result
                } catch {
                    guard !Task.isCancelled else { return }
                    self.error = error.localizedDescription
                }
                loading = false
            }
    }
}
