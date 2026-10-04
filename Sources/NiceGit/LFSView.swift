import NiceGitCore
import SwiftUI

/// Which file patterns Git LFS stores, which files it holds, and whether their content is here.
struct LFSView: View {
    @EnvironmentObject private var model: AppModel
    @Environment(\.dismiss) private var dismiss
    @State private var status: GitLFSStatus?
    @State private var error: String?
    @State private var pattern = ""

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            HStack {
                Text("Git LFS").font(.title2.bold())
                Spacer()
                Button { dismiss() } label: { Image(systemName: "xmark") }.help("Close")
            }
            if let error { Text(error).foregroundStyle(.red) }
            if let status {
                if let version = status.version {
                    Label(version, systemImage: "checkmark.circle").font(.caption).foregroundStyle(.secondary)
                } else {
                    Label("Git LFS is not installed, so new files cannot be stored with it. Install it with `brew install git-lfs`, then run `git lfs install` once.",
                          systemImage: "exclamationmark.triangle").font(.caption).foregroundStyle(.orange)
                }
                Text("Tracked patterns").font(.headline)
                if status.patterns.isEmpty { Text("None").foregroundStyle(.secondary) }
                ForEach(status.patterns, id: \.self) { tracked in
                    HStack {
                        Text(tracked).font(.body.monospaced())
                        Spacer()
                        Button("Untrack") { model.untrackLFS(tracked) }
                            .help("Store new versions of matching files in Git itself. Files already committed are unchanged.")
                    }
                }
                HStack {
                    TextField("Pattern, such as *.psd", text: $pattern).textFieldStyle(.roundedBorder)
                    Button("Track") { model.trackLFS(pattern) { pattern = "" } }
                        .disabled(status.version == nil || pattern.trimmingCharacters(in: .whitespaces).isEmpty)
                }
                Text("Tracking changes .gitattributes; commit it so others store these files the same way.")
                    .font(.caption).foregroundStyle(.secondary)
                Divider()
                Text("Files in Git LFS (\(status.files.count))").font(.headline)
                ScrollView {
                    VStack(alignment: .leading, spacing: 4) {
                        ForEach(status.files) { file in
                            HStack {
                                Text(file.path).font(.system(size: 12, design: .monospaced)).lineLimit(1).truncationMode(.middle)
                                Spacer()
                                if file.isPointerOnly {
                                    Text("Not downloaded").font(.caption).foregroundStyle(.orange)
                                        .help("Only the LFS pointer is here. Pull with Git LFS installed to download the content.")
                                }
                            }
                        }
                    }.frame(maxWidth: .infinity, alignment: .leading)
                }.frame(maxHeight: 200)
            } else if error == nil {
                ProgressView()
            }
        }
        .padding(24).frame(width: 520)
        .disabled(model.isLoading)
        .task(id: model.snapshot?.lastUpdated) {
            guard let url = model.repositoryURL else { return }
            do { status = try await Task.detached { try GitClient().lfsStatus(in: url) }.value; error = nil }
            catch { self.error = error.localizedDescription }
        }
    }
}
