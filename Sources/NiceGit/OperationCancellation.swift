import SwiftUI

private struct OperationCancellation: ViewModifier {
    @EnvironmentObject private var model: AppModel

    func body(content: Content) -> some View {
        content.safeAreaInset(edge: .bottom) {
            if model.isLoading {
                HStack {
                    ProgressView().controlSize(.small)
                    Spacer()
                    Button("Cancel operation") { model.cancelOperation() }
                        .disabled(false)
                }.padding(12).background(.regularMaterial)
            }
        }
    }
}

extension View {
    func operationCancellation() -> some View { modifier(OperationCancellation()) }
}
