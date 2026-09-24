import NiceGitCore
import SwiftUI

struct TagView: View {
    let commit: GitCommit
    @EnvironmentObject private var model: AppModel
    @Environment(\.dismiss) private var dismiss
    @State private var name = ""
    @State private var annotated = false
    @State private var message = ""

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            Text("Create tag").font(.title2.bold())
            Text(commit.subject).lineLimit(2)
            Text(commit.shortHash).font(.caption.monospaced()).foregroundStyle(.secondary)
            TextField("Tag name", text: $name).textFieldStyle(.roundedBorder)
            Picker("Tag type", selection: $annotated) {
                Text("Lightweight").tag(false)
                Text("Annotated").tag(true)
            }.pickerStyle(.segmented)
            if annotated {
                TextField("Tag message", text: $message, axis: .vertical)
                    .lineLimit(4...8).textFieldStyle(.roundedBorder)
            }
            if let error = model.errorMessage { Text(error).foregroundStyle(.red) }
            HStack {
                Button("Cancel") { dismiss() }.keyboardShortcut(.cancelAction)
                Spacer()
                Button("Create local tag") { model.createTag(name: name, target: commit.hash, message: annotated ? message : nil) }
                    .buttonStyle(.borderedProminent)
                    .disabled(name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || (annotated && message.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty))
            }
        }.padding(24).frame(width: 440).disabled(model.isLoading)
    }
}
