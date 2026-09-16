import NiceGitCore
import SwiftUI

struct OperationBar: View {
    let operation: GitOperation
    let hasConflicts: Bool
    @EnvironmentObject private var model: AppModel
    @State private var confirmingAbort = false

    var body: some View {
        HStack {
            Label("\(operation.rawValue.capitalized) in progress", systemImage: "arrow.triangle.branch")
            if hasConflicts { Text("Unresolved conflicts").foregroundStyle(.red) }
            Spacer()
            Button("Abort", role: .destructive) { confirmingAbort = true }
            Button("Continue") { model.continueOperation() }.disabled(hasConflicts)
        }
        .padding(12)
        .background(Color.orange.opacity(0.12))
        .confirmationDialog("Abort \(operation.rawValue)?", isPresented: $confirmingAbort) {
            Button("Abort operation", role: .destructive) { model.abortOperation() }
        } message: {
            Text("Git will restore the state before this operation. Conflict-resolution edits may be lost.")
        }
    }
}
