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
    @State private var signingKey = ""
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
            HStack {
                Menu("Use a profile") {
                    if model.identityProfiles.isEmpty { Text("No saved profiles") }
                    ForEach(model.identityProfiles) { profile in
                        Button(profile.label + (profile.signingKey == nil ? "" : " (signs commits)")) {
                            model.applyIdentityProfile(profile) {
                                name = profile.name
                                email = profile.email
                                saved = true
                            }
                        }
                    }
                    if !model.identityProfiles.isEmpty {
                        Divider()
                        Menu("Delete profile") {
                            ForEach(model.identityProfiles) { profile in
                                Button(profile.label, role: .destructive) { model.deleteIdentityProfile(profile) }
                            }
                        }
                    }
                }.fixedSize()
                Spacer()
                TextField("Signing key (optional)", text: $signingKey).frame(width: 180)
                    .help("A GPG key ID or SSH public key path. Profiles with a key turn on commit signing when applied.")
                Button("Save as profile") {
                    model.saveIdentityProfile(IdentityProfile(name: name, email: email,
                                                              signingKey: signingKey.trimmingCharacters(in: .whitespaces).isEmpty ? nil : signingKey))
                }.disabled(name.isEmpty || email.isEmpty)
            }
            Divider()
            Text("Remotes").font(.headline)
            if model.snapshot?.remotes.isEmpty ?? true {
                Text("No remotes").foregroundStyle(.secondary)
            }
            ForEach(model.snapshot?.remotes ?? [], id: \.self) { remote in
                RemoteRow(name: remote, address: model.snapshot?.remoteAddresses[remote],
                          pushAddresses: model.snapshot?.remotePushAddresses[remote] ?? [])
            }
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

/// One remote with its addresses, and controls to rename, re-point, or remove it. Each action
/// carries the address shown here so a remote changed elsewhere is not modified by mistake.
private struct RemoteRow: View {
    let name: String
    let address: String?
    let pushAddresses: [String]
    @EnvironmentObject private var model: AppModel
    @State private var editing = false
    @State private var newName = ""
    @State private var newAddress = ""
    @State private var confirmingRemoval = false

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            if editing {
                TextField("Remote name", text: $newName)
                TextField("Fetch URL", text: $newAddress)
                HStack {
                    Spacer()
                    Button("Cancel") { editing = false }
                    Button("Save") { save() }
                        .keyboardShortcut(.defaultAction)
                        .disabled(newName.trimmingCharacters(in: .whitespaces).isEmpty || newAddress.trimmingCharacters(in: .whitespaces).isEmpty
                                  || (newName == name && newAddress == address))
                }
            } else {
                HStack(spacing: 8) {
                    Text(name).font(.body.monospaced().weight(.semibold))
                    Spacer()
                    Button("Edit") {
                        newName = name
                        newAddress = address ?? ""
                        editing = true
                    }
                    Button("Remove...", role: .destructive) { confirmingRemoval = true }
                }
                Text(address ?? "No fetch URL").font(.caption.monospaced()).foregroundStyle(.secondary)
                    .textSelection(.enabled).lineLimit(2).truncationMode(.middle)
                // A remote's push URLs apply instead of its fetch URL when configured.
                if !pushAddresses.isEmpty && pushAddresses != [address ?? ""] {
                    ForEach(pushAddresses, id: \.self) { push in
                        Text("Push: " + push).font(.caption.monospaced()).foregroundStyle(.secondary)
                            .textSelection(.enabled).lineLimit(2).truncationMode(.middle)
                    }
                }
            }
        }
        .padding(10)
        .background(Color.primary.opacity(0.05), in: RoundedRectangle(cornerRadius: 6))
        .confirmationDialog("Remove remote \(name)?", isPresented: $confirmingRemoval) {
            Button("Remove remote", role: .destructive) { model.removeRemote(name, expectedAddress: address) }
        } message: {
            Text("NiceGit forgets this remote and deletes its remote-tracking branches from this repository. Local branches that track it lose their upstream. Nothing on the server is changed.")
        }
    }

    private func save() {
        model.updateRemote(name, name: newName.trimmingCharacters(in: .whitespaces),
                           address: newAddress.trimmingCharacters(in: .whitespaces), expectedAddress: address) { editing = false }
    }
}

