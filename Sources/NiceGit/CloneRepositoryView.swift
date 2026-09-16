import SwiftUI

struct CloneRepositoryView: View {
    @EnvironmentObject private var model: AppModel
    @Environment(\.dismiss) private var dismiss
    @State private var source = ""

    var body: some View {
        VStack(alignment: .leading, spacing: 18) {
            Text("Clone repository").font(.title2.bold())
            TextField("Repository URL or local path", text: $source)
                .textFieldStyle(.roundedBorder)
                .onSubmit { clone() }
            HStack {
                Button("Cancel") { dismiss() }.keyboardShortcut(.cancelAction)
                Spacer()
                Button("Choose destination...") { clone() }
                    .keyboardShortcut(.defaultAction)
                    .disabled(source.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
            }
        }.padding(24).frame(width: 460)
    }

    private func clone() {
        let trimmed = source.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return }
        model.clone(source: trimmed)
    }
}
