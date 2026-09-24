import NiceGitCore
import SwiftUI

struct CommitInspector: View {
    let commit: GitCommit
    let repositoryURL: URL
    let showFile: (DiffSelection) -> Void
    let showWorkingTree: () -> Void
    @State private var files: [GitCommitFileChange] = []
    @State private var loading = false
    @State private var error: String?
    @State private var message = ""
    @State private var hoveredFile: String?
    @State private var selectedFile: String?
    @State private var showingFullMessage = false
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        GeometryReader { geometry in
        let summaryHeight = geometry.size.height / 3
        let compact = summaryHeight < 150
        VStack(alignment: .leading, spacing: 0) {
            VStack(alignment: .leading, spacing: 0) {
                HStack(spacing: 10) {
                    Text("Commit").font(.system(size: 13, weight: .semibold))
                    Text(commit.shortHash).font(.caption.monospaced()).foregroundStyle(.secondary)
                        .help(commit.hash)
                    Spacer(minLength: 0)
                    Button(action: showWorkingTree) { Image(systemName: "folder") }
                        .buttonStyle(.plain).help("Show working tree")
                }
                .padding(.horizontal, compact ? 10 : 14)
                .frame(height: compact ? 32 : 44)
                Divider()
                VStack(alignment: .leading, spacing: compact ? 6 : 10) {
                    HStack(alignment: .top, spacing: 8) {
                        Text(message.isEmpty ? commit.subject : message)
                            .font(.system(size: 13, weight: .medium))
                            .lineLimit(summaryHeight >= 300 ? 6 : summaryHeight >= 230 ? 3 : summaryHeight >= 150 ? 2 : 1)
                            .frame(maxWidth: .infinity, alignment: .leading)
                        Button { showingFullMessage = true } label: {
                            Image(systemName: "arrow.up.left.and.arrow.down.right")
                        }.buttonStyle(.plain).help("Read full commit message")
                            .popover(isPresented: $showingFullMessage) {
                                VStack(alignment: .leading, spacing: 12) {
                                    Text("Commit message").font(.headline)
                                    Divider()
                                    ScrollView {
                                        Text(message.isEmpty ? commit.subject : message)
                                            .textSelection(.enabled)
                                            .frame(maxWidth: .infinity, alignment: .leading)
                                    }
                                }.padding(16).frame(width: 460, height: 320)
                            }
                    }
                    .padding(compact ? 6 : 12)
                    .background(Color.primary.opacity(0.07), in: RoundedRectangle(cornerRadius: 4))
                    if summaryHeight >= 185 {
                        HStack(spacing: 8) {
                            Image(systemName: "person.crop.circle").foregroundStyle(.secondary)
                            VStack(alignment: .leading, spacing: 2) {
                                Text(commit.authorName).font(.system(size: 12, weight: .medium))
                                Text(commit.authorEmail).font(.system(size: 10))
                                    .foregroundStyle(.secondary).lineLimit(1).truncationMode(.middle)
                            }
                        }
                    }
                    if summaryHeight >= 245 {
                        Label(commit.commitDate?.formatted(date: .abbreviated, time: .shortened) ?? commit.relativeDate,
                              systemImage: "calendar")
                            .font(.system(size: 11)).foregroundStyle(.secondary)
                        Label(commit.parents.isEmpty ? "Initial commit" : "Parent: " + commit.parents.map { String($0.prefix(7)) }.joined(separator: ", "),
                              systemImage: "arrow.turn.down.right")
                            .font(.system(size: 11, design: .monospaced)).foregroundStyle(.secondary)
                    }
                    if summaryHeight >= 300 {
                        Text(commit.hash).font(.system(size: 10, design: .monospaced))
                            .foregroundStyle(.secondary).lineLimit(1).truncationMode(.middle)
                            .textSelection(.enabled)
                    }
                }
                .padding(compact ? 6 : 14)
                .frame(maxWidth: .infinity, alignment: .leading)
                Spacer(minLength: 0)
            }
            .frame(height: summaryHeight)
            .background(AppPalette.toolbar)
        Divider()
        VStack(alignment: .leading, spacing: 0) {
            ViewThatFits(in: .horizontal) {
                HStack(spacing: 12) { changeCounts }
                VStack(alignment: .leading, spacing: 6) { changeCounts }
            }.padding(.horizontal, 16).padding(.top, 16).padding(.bottom, 8)
            HStack {
                Text("Changed files").font(.subheadline.weight(.semibold))
                Spacer()
                Text("\(files.count)").font(.caption.monospaced())
            }.padding(.horizontal, 16).padding(.vertical, 8)
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
        }.frame(maxWidth: .infinity, maxHeight: .infinity)
            .background(AppPalette.panel)
        }
        }
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
