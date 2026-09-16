import NiceGitCore
import SwiftUI

struct RepositorySettingsView: View {
    @EnvironmentObject private var model: AppModel
    @Environment(\.dismiss) private var dismiss
    @State private var name = ""
    @State private var email = ""
    @State private var remoteName = "origin"
    @State private var address = ""
    @State private var saved = false
    @State private var loading = true

    var body: some View {
        VStack(alignment: .leading, spacing: 18) {
            HStack {
                Text("Repository settings").font(.title2.bold())
                Spacer()
                Button { dismiss() } label: { Image(systemName: "xmark") }.help("Close settings")
            }
            Form {
                TextField("Commit name", text: $name)
                TextField("Commit email", text: $email)
            }
            HStack {
                Text("Applies to this repository").font(.caption).foregroundStyle(.secondary)
                Spacer()
                if saved { Image(systemName: "checkmark.circle.fill").foregroundStyle(.green) }
                Button("Save identity") { model.setIdentity(name: name, email: email) { saved = true } }
                    .disabled(name.isEmpty || email.isEmpty)
            }
            Divider()
            Text("Remotes").font(.headline)
            ForEach(model.snapshot?.remotes ?? [], id: \.self) { Text($0).font(.body.monospaced()) }
            Form {
                TextField("Remote name", text: $remoteName)
                TextField("Remote URL", text: $address)
            }
            HStack {
                Spacer()
                Button("Add remote") {
                    model.addRemote(name: remoteName, address: address) { address = "" }
                }.disabled(remoteName.isEmpty || address.isEmpty)
            }
            if let error = model.errorMessage { Text(error).foregroundStyle(.red) }
        }
        .textFieldStyle(.roundedBorder)
        .padding(24).frame(width: 520)
        .disabled(model.isLoading || loading)
        .operationCancellation()
        .task {
            if let url = model.repositoryURL {
                let identity = await Task.detached { GitClient().identity(in: url) }.value
                name = identity.name
                email = identity.email
            }
            loading = false
        }
        .onChange(of: name) { saved = false }
        .onChange(of: email) { saved = false }
    }
}
