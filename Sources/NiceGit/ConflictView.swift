import NiceGitCore
import SwiftUI

struct ConflictView: View {
    let selection: DiffSelection
    @EnvironmentObject private var model: AppModel
    @Environment(\.dismiss) private var dismiss
    @State private var document: GitConflictDocument?
    @State private var content = ""
    @State private var error: String?
    @State private var busy = true
    @State private var confirmClose = false
    @State private var version = 2
    @State private var confirmReplace = false
    @State private var control = GitCommandControl()
    @State private var wholeFileChoice: Int?

    private var reference: String? {
        switch version {
        case 1: document?.base
        case 2: document?.current
        default: document?.incoming
        }
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack {
                Text(selection.title).font(.headline).lineLimit(1)
                Spacer()
                Button { close() } label: { Image(systemName: "xmark") }.help("Close conflict editor")
            }
            if let error { Text(error).foregroundStyle(.red).textSelection(.enabled) }
            if document != nil {
                HSplitView {
                    VStack(alignment: .leading, spacing: 10) {
                        Picker("Version", selection: $version) {
                            Text("Base").tag(1)
                            Text(model.snapshot?.operation == .rebase ? "Destination" : "Current").tag(2)
                            Text(model.snapshot?.operation == .rebase ? "Replayed commit" : "Incoming").tag(3)
                        }.pickerStyle(.segmented)
                        ScrollView([.horizontal, .vertical]) {
                            Text(reference ?? "Version unavailable")
                                .font(.system(size: 12, design: .monospaced))
                                .textSelection(.enabled)
                                .frame(maxWidth: .infinity, alignment: .topLeading)
                                .padding(8)
                        }
                        Button("Use version") { confirmReplace = true }.disabled(reference == nil)
                    }.frame(minWidth: 280)
                    VStack(alignment: .leading, spacing: 10) {
                        Text("Result").font(.headline)
                        TextEditor(text: $content)
                            .font(.system(size: 13, design: .monospaced))
                            .border(Color.secondary.opacity(0.3))
                    }.frame(minWidth: 300)
                }
            } else if busy {
                ProgressView().frame(maxWidth: .infinity, maxHeight: .infinity)
            } else {
                ContentUnavailableView("Cannot edit this conflict", systemImage: "exclamationmark.triangle")
            }
            HStack {
                Menu("Resolve whole file") {
                    Button(model.snapshot?.operation == .rebase ? "Use destination" : "Use current") { wholeFileChoice = 2 }
                    Button(model.snapshot?.operation == .rebase ? "Use replayed commit" : "Use incoming") { wholeFileChoice = 3 }
                    Button("Delete file", role: .destructive) { wholeFileChoice = 0 }
                }
                Spacer()
                if busy { ProgressView().controlSize(.small) }
                Button("Save and stage resolution") { save() }
                    .buttonStyle(.borderedProminent)
                    .disabled(document == nil || busy)
            }
        }
        .padding(20).frame(width: 900, height: 620)
        .disabled(busy)
        .safeAreaInset(edge: .bottom) {
            if busy {
                Button("Cancel operation") { control.cancel() }
                    .disabled(false).padding(10)
            }
        }
        .onDisappear { control.cancel() }
        .interactiveDismissDisabled()
        .confirmationDialog("Discard editor changes?", isPresented: $confirmClose) {
            Button("Discard editor changes", role: .destructive) { dismiss() }
        }
        .confirmationDialog("Replace the result with this version?", isPresented: $confirmReplace) {
            Button("Use version") { if let reference { content = reference } }
        }
        .confirmationDialog("Resolve the entire file?", isPresented: Binding(get: { wholeFileChoice != nil }, set: { if !$0 { wholeFileChoice = nil } })) {
            if let choice = wholeFileChoice {
                Button(choice == 0 ? "Delete and stage" : "Replace and stage", role: .destructive) { resolveWholeFile(choice) }
            }
        } message: {
            Text("This replaces the working file and discards unsaved editor changes. The resolution will be staged.")
        }
        .task {
            let path = selection.path ?? ""
            let url = selection.repositoryURL
            let commandControl = control
            do {
                let loaded = try await Task.detached { try GitClient(control: commandControl).loadConflict(path: path, in: url) }.value
                document = loaded
                content = loaded.content
            } catch { self.error = error.localizedDescription }
            busy = false
        }
    }

    private func close() {
        if let document, content != document.content { confirmClose = true }
        else { dismiss() }
    }

    private func resolveWholeFile(_ choice: Int) {
        guard let path = selection.path else { return }
        let url = selection.repositoryURL
        let commandControl = GitCommandControl()
        control = commandControl
        busy = true
        error = nil
        Task {
            defer { busy = false }
            do {
                try await Task.detached {
                    let git = GitClient(control: commandControl)
                    if choice == 0 { try git.resolveConflictDeletion(path: path, in: url) }
                    else { try git.resolveConflictSide(path: path, incoming: choice == 3, in: url) }
                }.value
                model.refresh()
                dismiss()
            } catch { self.error = error.localizedDescription }
        }
    }

    private func save() {
        guard let document else { return }
        let resolved = content
        let url = selection.repositoryURL
        busy = true
        error = nil
        let commandControl = GitCommandControl()
        control = commandControl
        Task {
            defer { busy = false }
            do {
                try await Task.detached { try GitClient(control: commandControl).resolveConflict(document, content: resolved, in: url) }.value
                model.refresh()
                dismiss()
            } catch { self.error = error.localizedDescription }
        }
    }
}
