import AppKit
import NiceGitCore
import SwiftUI

struct RepositoryActionBar: View {
    @EnvironmentObject private var model: AppModel
    let snapshot: RepositorySnapshot
    @State private var newBranch = false
    @State private var branchName = ""
    @State private var stashToPop: GitStash?
    @State private var historyAction: Bool?

    var body: some View {
        ViewThatFits(in: .horizontal) {
            HStack(spacing: 12) {
                selectors
                Spacer(minLength: 8)
                actions
            }
            VStack(alignment: .leading, spacing: 4) {
                selectors
                actions
            }.frame(maxWidth: .infinity, alignment: .leading)
        }
        .padding(.horizontal, 14).padding(.vertical, 6)
        .background(AppPalette.toolbar)
        .disabled(model.isLoading)
        .confirmationDialog(historyAction == true ? "Redo the last NiceGit commit?" : "Undo the last NiceGit commit?", isPresented: Binding(get: { historyAction != nil }, set: { if !$0 { historyAction = nil } })) {
            if let redo = historyAction {
                Button(redo ? "Redo commit" : "Undo commit") { model.moveCommitHistory(redo: redo) }
            }
        } message: {
            Text("Moves the local branch only. The index and working files are preserved. Coordinate before changing history already shared with others. Remote branches are not changed.")
        }
        .alert("Create and checkout branch", isPresented: $newBranch) {
            TextField("Branch name", text: $branchName)
            Button("Cancel", role: .cancel) {}
            Button("Create branch") { model.createBranch(named: branchName) { branchName = "" } }
                .disabled(branchName.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
        }
        .confirmationDialog("Apply and remove this stash?", isPresented: Binding(get: { stashToPop != nil }, set: { if !$0 { stashToPop = nil } })) {
            if let stash = stashToPop {
                Button("Pop stash") { model.popStash(stash) }
            }
        } message: {
            Text("The stash is removed only after its changes apply successfully. If applying fails or conflicts, it is kept.")
        }
    }

    private var selectors: some View {
        HStack(spacing: 16) {
            VStack(alignment: .leading, spacing: 2) {
                Text("Repository").font(.caption).foregroundStyle(.secondary)
                Menu {
                    ForEach(model.openRepositories) { repository in
                        Button(repository.name) { model.loadRepository(at: URL(fileURLWithPath: repository.path)) }
                    }
                    Divider()
                    Button("Open repository...") { model.openRepository() }
                    Button("Clone repository...") { model.showingClone = true }
                } label: { Text(snapshot.name).lineLimit(1) }
                    .menuStyle(.borderlessButton).font(.system(size: 14, weight: .semibold))
                    .help(snapshot.rootPath)
            }.frame(width: 156, alignment: .leading)
            VStack(alignment: .leading, spacing: 2) {
                Text("Branch").font(.caption).foregroundStyle(.secondary)
                Menu {
                    ForEach(snapshot.branches.filter { !$0.isRemote }) { branch in
                        Button(branch.name) { model.checkout(branch: branch) }.disabled(branch.isCurrent)
                    }
                } label: { Text(snapshot.currentBranch).lineLimit(1).truncationMode(.middle) }
                    .menuStyle(.borderlessButton).font(.system(size: 14, weight: .medium))
                    .disabled(snapshot.operation != nil)
            }.frame(width: 156, alignment: .leading)
        }.frame(height: 44)
    }

    private var actions: some View {
        HStack(spacing: 4) {
            action("Undo", icon: "arrow.uturn.backward", help: "Undo the last NiceGit commit; keep file changes") { historyAction = false }
                .disabled(!model.canUndoCommit)
            action("Redo", icon: "arrow.uturn.forward", help: "Restore the commit undone in this session") { historyAction = true }
                .disabled(!model.canRedoCommit)
            HStack(spacing: 0) {
                action("Fetch", icon: "arrow.down.to.line", help: "Fetch all remotes") { model.fetch() }
                    .disabled(snapshot.remotes.isEmpty)
                Menu {
                    Button("Fetch all remotes") { model.fetch() }.disabled(snapshot.remotes.isEmpty)
                    Button("Pull (fast-forward only)") { model.pull() }
                        .disabled(snapshot.upstream == nil || snapshot.operation != nil)
                } label: { Image(systemName: "chevron.down").font(.system(size: 9)).frame(width: 18, height: 40) }
                    .menuStyle(.borderlessButton).menuIndicator(.hidden)
                    .help("Fetch and pull options").accessibilityLabel("Fetch and pull options")
            }
            action("Push", icon: "arrow.up.to.line", help: "Push or publish current branch") { model.push() }
                .disabled(snapshot.remotes.isEmpty)
            action("Branch", icon: "arrow.triangle.branch", help: "Create a branch") { newBranch = true }
                .disabled(snapshot.operation != nil)
            action("Stash", icon: "archivebox", help: "Save or manage stashes") { model.showingStashes = true }
            action("Pop", icon: "tray.and.arrow.up", help: "Apply and remove the latest stash") { stashToPop = snapshot.stashes.first }
                .disabled(snapshot.stashes.isEmpty || snapshot.operation != nil)
            Divider().frame(height: 34).padding(.horizontal, 4)
            action("Terminal", icon: "terminal", help: "Show the embedded terminal") { model.openTerminal() }
            action("Refresh", icon: "arrow.clockwise", help: "Refresh repository") { model.refresh() }
        }.fixedSize(horizontal: true, vertical: false)
    }

    private func action(_ title: String, icon: String, help: String, perform: @escaping () -> Void) -> some View {
        Button(action: perform) {
            VStack(spacing: 4) {
                Text(title).font(.system(size: 11))
                Image(systemName: icon).font(.system(size: 18, weight: .medium))
            }.frame(width: 48, height: 44).contentShape(Rectangle())
        }.buttonStyle(.plain).help(help).accessibilityLabel(title)
    }
}
