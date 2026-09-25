import SwiftUI

struct PublishBranchView: View {
    @EnvironmentObject private var model: AppModel
    @Environment(\.dismiss) private var dismiss
    @State private var remote = ""

    var body: some View {
        VStack(alignment: .leading, spacing: 18) {
            Text("Publish branch").font(.title2.bold())
            Text(model.snapshot?.currentBranch ?? "").font(.headline)
            Picker("Remote", selection: $remote) {
                ForEach(model.snapshot?.remotes ?? [], id: \.self) { Text($0).tag($0) }
            }
            if let error = model.errorMessage { Text(error).foregroundStyle(.red) }
            HStack {
                Button("Cancel") { dismiss() }
                Spacer()
                if model.isLoading { ProgressView().controlSize(.small) }
                Button("Publish") { model.publish(remote: remote) }
                    .buttonStyle(.borderedProminent)
                    .disabled(remote.isEmpty)
            }
        }.padding(24).frame(width: 460)
            .disabled(model.isLoading)
            .onAppear { remote = model.snapshot?.remotes.first ?? "" }
    }
}
