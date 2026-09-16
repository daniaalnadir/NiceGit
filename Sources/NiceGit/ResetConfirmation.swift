import NiceGitCore
import SwiftUI

struct ResetRequest {
    let target: String
    let mode: GitResetMode
    let branch: String
    let head: String
}

struct ResetConfirmation: ViewModifier {
    @EnvironmentObject private var model: AppModel
    @Binding var request: ResetRequest?

    func body(content: Content) -> some View {
        content.confirmationDialog("Reset \(request?.branch ?? "") to \(request.map { String($0.target.prefix(8)) } ?? "")?", isPresented: Binding(get: { request != nil }, set: { if !$0 { request = nil } })) {
            if let request {
                Button(request.mode == .hard ? "Discard changes and hard reset" : "Reset branch", role: request.mode == .hard ? .destructive : nil) {
                    model.reset(to: request.target, mode: request.mode, expectedHead: request.head, expectedBranch: request.branch)
                }
            }
        } message: {
            if let request {
                Text(request.mode.warning + " The branch will point to the selected commit. Commits outside its ancestry will no longer be in this branch's history. Remote branches are not changed.")
            }
        }
    }
}
