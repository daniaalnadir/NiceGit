import NiceGitCore
import SwiftUI

struct StashView: View {
    @EnvironmentObject private var model: AppModel
    @Environment(\.dismiss) private var dismiss
    @State private var message = ""
    @State private var includeUntracked = true
    @State private var pendingDrop: GitStash?
    @State private var pendingPop: GitStash?
    @State private var preview: DiffSelection?

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            HStack {
                Text("Stashes").font(.title2.bold())
                Spacer()
                if model.isLoading { ProgressView().controlSize(.small) }
                Button { dismiss() } label: { Image(systemName: "xmark") }.help("Close stashes")
            }
            HStack {
                TextField("Stash message", text: $message).textFieldStyle(.roundedBorder)
                Button("Stash changes") {
                    model.saveStash(message: message, includeUntracked: includeUntracked) { message = "" }
                }.disabled(model.snapshot?.status.isEmpty != false || model.snapshot?.commits.isEmpty != false)
            }
            Toggle("Include untracked files", isOn: $includeUntracked)
            Divider()
            if let error = model.errorMessage {
                Text(error).foregroundStyle(.red).textSelection(.enabled)
            }
            ScrollView {
                LazyVStack(alignment: .leading, spacing: 0) {
                    ForEach(model.snapshot?.stashes ?? []) { stash in
                HStack {
                    VStack(alignment: .leading, spacing: 4) {
                        Text(stash.message).lineLimit(2)
                        Text(stash.reference).font(.caption.monospaced()).foregroundStyle(.secondary)
                    }
                    Spacer()
                    Button {
                        guard let url = model.repositoryURL else { return }
                        preview = DiffSelection(title: stash.message, repositoryURL: url, stashHash: stash.hash)
                    } label: { Image(systemName: "doc.text.magnifyingglass") }
                        .help("Inspect stash").accessibilityLabel("Inspect \(stash.reference)")
                    Button("Apply") { model.applyStash(stash) }.help("Restore changes and keep the stash")
                    Button("Pop") { pendingPop = stash }.help("Restore changes and remove the stash after a successful apply")
                    Button { pendingDrop = stash } label: { Image(systemName: "trash") }
                        .help("Delete stash").accessibilityLabel("Delete \(stash.reference)")
                }.padding(.vertical, 6)
                    .accessibilityElement(children: .contain)
                    }
                }.padding(.horizontal, 12)
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .overlay {
                if model.snapshot?.stashes.isEmpty != false {
                    ContentUnavailableView("No stashes", systemImage: "archivebox")
                }
            }
        }
        .padding(20).frame(width: 620, height: 460)
        .onAppear { message = "WIP on \(model.snapshot?.currentBranch ?? "HEAD")" }
        .disabled(model.isLoading)
        .operationCancellation()
        .sheet(item: $preview) { DiffView(selection: $0) }
        .confirmationDialog("Apply and remove this stash?", isPresented: Binding(get: { pendingPop != nil }, set: { if !$0 { pendingPop = nil } })) {
            if let stash = pendingPop {
                Button("Pop stash") { model.popStash(stash); pendingPop = nil }
            }
        } message: {
            Text("The stash is removed only after its changes apply successfully. If applying fails or conflicts, it is kept.")
        }
        .confirmationDialog("Delete this stash permanently?", isPresented: Binding(get: { pendingDrop != nil }, set: { if !$0 { pendingDrop = nil } })) {
            if let stash = pendingDrop {
                Button("Delete stash", role: .destructive) { model.dropStash(stash); pendingDrop = nil }
            }
        }
    }
}
