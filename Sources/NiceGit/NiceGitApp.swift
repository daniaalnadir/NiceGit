import SwiftUI

@main
@MainActor
struct NiceGitApp: App {
    @NSApplicationDelegateAdaptor(NiceGitApplicationDelegate.self) private var applicationDelegate
    @StateObject private var model = AppModel()

    var body: some Scene {
        WindowGroup {
            ContentView()
                .environmentObject(model)
        }
        .windowStyle(.hiddenTitleBar)
        .commands {
            CommandGroup(after: .sidebar) {
                Button(model.showingTerminal ? "Hide Terminal" : "Show Terminal") {
                    model.toggleTerminal()
                }
                .keyboardShortcut("`", modifiers: .control)
                .disabled(model.snapshot == nil || model.isLoading)
            }
            CommandMenu("Repository") {
                Button("New Repository...") { model.initializeRepository() }
                Button("Repository Settings...") { model.showingRepositorySettings = true }
                    .disabled(model.snapshot == nil)
                Divider()
                Button("Open Repository...") {
                    model.openRepository()
                }
                .keyboardShortcut("o")

                Button("Refresh") {
                    model.refresh()
                }
                .keyboardShortcut("r")
                .disabled(model.snapshot == nil)

                Divider()

                Button("Apply Patch...") { model.importPatch() }
                    .disabled(model.snapshot == nil || model.isLoading || model.snapshot?.operation != nil)

                Button("Publish Branch...") { model.showingPublish = true }
                    .disabled(model.snapshot?.remotes.isEmpty != false)

                Button("Fetch") {
                    model.fetch()
                }
                .disabled(model.snapshot == nil)
            }
        }
    }
}

@MainActor
final class NiceGitApplicationDelegate: NSObject, NSApplicationDelegate {
    func applicationDidFinishLaunching(_ notification: Notification) {
        // Xcode runs the Swift package as a bare executable, without an app bundle's activation policy.
        guard Bundle.main.bundleURL.pathExtension != "app" else { return }
        NSApplication.shared.setActivationPolicy(.regular)
        NSApplication.shared.activate()
        NSApplication.shared.windows.first(where: { $0.canBecomeKey })?.makeKeyAndOrderFront(nil)
    }
}
