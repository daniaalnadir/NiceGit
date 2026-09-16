import AppKit
import Combine
import NiceGitCore
import SwiftUI
@preconcurrency import SwiftTerm

@MainActor
final class TerminalSession: NSObject, ObservableObject, LocalProcessTerminalViewDelegate {
    let path: String
    let view = LocalProcessTerminalView(frame: NSRect(x: 0, y: 0, width: 800, height: 240))
    @Published private(set) var status = "Shell"
    @Published private(set) var ended = false
    private var started = false

    init(path: String) {
        self.path = path
        super.init()
        view.processDelegate = self
        view.font = .monospacedSystemFont(ofSize: 12, weight: .regular)
        view.nativeBackgroundColor = NSColor(white: 0.09, alpha: 1)
        view.nativeForegroundColor = NSColor(white: 0.9, alpha: 1)
    }

    func start() {
        guard !started else { return }
        started = true
        var environment = GitClient.repositoryEnvironment(ProcessInfo.processInfo.environment)
        environment["TERM"] = "xterm-256color"
        environment["COLORTERM"] = "truecolor"
        let preferred = environment["SHELL"] ?? "/bin/zsh"
        let shell = FileManager.default.isExecutableFile(atPath: preferred) ? preferred : "/bin/zsh"
        view.startProcess(executable: shell, args: ["-l"], environment: environment.map { "\($0.key)=\($0.value)" }, currentDirectory: path)
        if !view.process.running { ended = true; status = "Shell failed to start" }
    }

    func end() { view.terminate() }
    nonisolated func sizeChanged(source: LocalProcessTerminalView, newCols: Int, newRows: Int) {}
    nonisolated func setTerminalTitle(source: LocalProcessTerminalView, title: String) {
        Task { @MainActor in self.status = title }
    }
    nonisolated func hostCurrentDirectoryUpdate(source: TerminalView, directory: String?) {}
    nonisolated func processTerminated(source: TerminalView, exitCode: Int32?) {
        Task { @MainActor in
            self.ended = true
            self.status = "Shell exited" + (exitCode.map { " (\($0))" } ?? "")
        }
    }
}

private struct EmbeddedTerminal: NSViewRepresentable {
    let session: TerminalSession
    func makeNSView(context: Context) -> LocalProcessTerminalView {
        DispatchQueue.main.async {
            session.start()
            session.view.window?.makeFirstResponder(session.view)
        }
        return session.view
    }
    func updateNSView(_ nsView: LocalProcessTerminalView, context: Context) {}
}

struct TerminalPanel: View {
    @EnvironmentObject private var model: AppModel
    @ObservedObject var session: TerminalSession
    @State private var confirmingEnd = false

    var body: some View {
        VStack(spacing: 0) {
            HStack(spacing: 10) {
                Image(systemName: "terminal")
                Text(session.status).lineLimit(1)
                Text(session.path).foregroundStyle(.secondary).lineLimit(1).truncationMode(.middle)
                Spacer(minLength: 0)
                if session.ended {
                    Button { model.openTerminal(restart: true) } label: { Image(systemName: "arrow.clockwise") }
                        .help("Start a new shell")
                } else {
                    Button { confirmingEnd = true } label: { Image(systemName: "stop.circle") }
                        .help("End terminal session")
                }
                Button { model.toggleTerminal() } label: { Image(systemName: "xmark") }
                    .help("Hide terminal; keep shell running")
            }.font(.system(size: 11)).buttonStyle(.plain).padding(10)
                .background(AppPalette.toolbar)
            EmbeddedTerminal(session: session).id(ObjectIdentifier(session))
        }
        .confirmationDialog("End this terminal session?", isPresented: $confirmingEnd) {
            Button("End session", role: .destructive) { session.end() }
        } message: { Text("Running commands may be interrupted. Hide the panel instead to keep the shell running.") }
    }
}
