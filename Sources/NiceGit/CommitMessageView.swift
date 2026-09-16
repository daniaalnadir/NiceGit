import NiceGitCore
import SwiftUI

struct CommitMessageView: View {
    let commit: GitCommit
    @EnvironmentObject private var model: AppModel
    @Environment(\.dismiss) private var dismiss
    @State private var message = ""
    @State private var original = ""
    @State private var loading = true
    @State private var error: String?
    @State private var confirming = false

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            Text("Edit commit message").font(.title2.bold())
            Text(commit.shortHash).font(.caption.monospaced()).foregroundStyle(.secondary)
            TextEditor(text: $message).font(.body.monospaced()).frame(height: 180)
                .accessibilityLabel("Commit message")
            if loading { ProgressView().controlSize(.small) }
            if let error { Text(error).foregroundStyle(.red) }
            if let error = model.errorMessage { Text(error).foregroundStyle(.red) }
            HStack {
                Button("Cancel") { dismiss() }.keyboardShortcut(.cancelAction)
                Spacer()
                Button("Amend message...") { confirming = true }
                    .disabled(loading || error != nil || message == original || message.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
            }
        }.padding(24).frame(width: 520).disabled(model.isLoading)
        .operationCancellation()
        .confirmationDialog("Rewrite the HEAD commit?", isPresented: $confirming) {
            Button("Amend message") { model.amendMessage(message, for: commit) }
        } message: {
            Text("This changes the commit hash. If already shared, coordinate with collaborators before updating the remote. Staged and unstaged file changes are not included.")
        }
        .task {
            guard let url = model.repositoryURL else { loading = false; return }
            let control = GitCommandControl()
            defer { loading = false }
            do {
                let text = try await withTaskCancellationHandler {
                    try await Task.detached { try GitClient(control: control).commitMessage(hash: commit.hash, in: url) }.value
                } onCancel: { control.cancel() }
                guard !Task.isCancelled else { return }
                message = text.trimmingCharacters(in: .newlines)
                original = message
            } catch { if !Task.isCancelled { self.error = error.localizedDescription } }
        }
    }
}
