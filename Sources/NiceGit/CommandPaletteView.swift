import NiceGitCore
import SwiftUI

/// A keyboard-driven list of repository actions, branches, and recent repositories.
/// Commands call the same model actions as buttons, so their safety checks still apply.
struct CommandPaletteView: View {
    @EnvironmentObject private var model: AppModel
    @Environment(\.dismiss) private var dismiss
    @State private var query = ""
    @State private var selection = 0
    @FocusState private var focused: Bool

    struct Command: Identifiable {
        let id: String
        let title: String
        let detail: String
        let systemImage: String
        var enabled = true
        let run: () -> Void
    }

    var body: some View {
        let matches = Self.ranked(commands, query: query)
        VStack(spacing: 0) {
            HStack(spacing: 10) {
                Image(systemName: "command").foregroundStyle(.secondary)
                TextField("Type a command, branch, or repository", text: $query)
                    .textFieldStyle(.plain).font(.system(size: 16)).focused($focused)
                    .onSubmit { run(matches) }
                    .onKeyPress(.downArrow) { selection = min(selection + 1, max(matches.count - 1, 0)); return .handled }
                    .onKeyPress(.upArrow) { selection = max(selection - 1, 0); return .handled }
                    .onKeyPress(.escape) { dismiss(); return .handled }
            }.padding(14)
            Divider()
            ScrollViewReader { proxy in
                ScrollView {
                    LazyVStack(spacing: 0) {
                        if matches.isEmpty {
                            Text("No matching commands").foregroundStyle(.secondary).padding(20)
                        }
                        ForEach(Array(matches.enumerated()), id: \.element.id) { index, command in
                            Button { selection = index; run(matches) } label: {
                                HStack(spacing: 10) {
                                    Image(systemName: command.systemImage).frame(width: 18).foregroundStyle(.secondary)
                                    Text(command.title).lineLimit(1)
                                    Spacer(minLength: 8)
                                    Text(command.detail).font(.caption).foregroundStyle(.secondary).lineLimit(1).truncationMode(.middle)
                                }
                                .padding(.horizontal, 14).padding(.vertical, 8)
                                .background(AppPalette.signal.opacity(index == selection ? 0.22 : 0))
                                .contentShape(Rectangle())
                                .opacity(command.enabled ? 1 : 0.45)
                            }
                            .buttonStyle(.plain).disabled(!command.enabled).id(index)
                        }
                    }
                }.onChange(of: selection) { proxy.scrollTo(selection) }
            }
        }
        .frame(width: 560, height: 400)
        .onAppear { focused = true }
        .onChange(of: query) { selection = 0 }
    }

    private func run(_ matches: [Command]) {
        guard matches.indices.contains(selection), matches[selection].enabled else { return }
        let command = matches[selection]
        dismiss()
        // Let the sheet close first so commands that present their own sheet or alert can.
        DispatchQueue.main.async { command.run() }
    }

    private var commands: [Command] {
        let snapshot = model.snapshot
        let hasRepository = snapshot != nil
        let idle = !model.isLoading
        let clean = snapshot?.operation == nil
        var list: [Command] = [
            Command(id: "fetch", title: "Fetch", detail: "Download from all remotes", systemImage: "arrow.down.circle",
                    enabled: hasRepository && idle) { model.fetch() },
            Command(id: "pull", title: "Pull", detail: "Fetch and integrate the upstream branch", systemImage: "arrow.down.to.line",
                    enabled: hasRepository && idle && clean) { model.pull() },
            Command(id: "push", title: "Push", detail: "Send the current branch to its upstream", systemImage: "arrow.up.to.line",
                    enabled: hasRepository && idle) { model.push() },
            Command(id: "refresh", title: "Refresh", detail: "⌘R", systemImage: "arrow.clockwise",
                    enabled: hasRepository && idle) { model.refresh() },
            Command(id: "stage-all", title: "Stage all changes", detail: "", systemImage: "plus.circle",
                    enabled: hasRepository && idle && snapshot?.status.isEmpty == false) { model.stageAll() },
            Command(id: "unstage-all", title: "Unstage all changes", detail: "", systemImage: "minus.circle",
                    enabled: hasRepository && idle && snapshot?.stagedCount ?? 0 > 0) { model.unstageAll() },
            Command(id: "stashes", title: "Stashes", detail: "Save or apply stashed changes", systemImage: "archivebox",
                    enabled: hasRepository && clean) { model.showingStashes = true },
            Command(id: "publish", title: "Publish branch", detail: "", systemImage: "arrow.up.circle",
                    enabled: snapshot?.remotes.isEmpty == false && idle) { model.showingPublish = true },
            Command(id: "terminal", title: model.showingTerminal ? "Hide terminal" : "Show terminal", detail: "⌃`", systemImage: "terminal",
                    enabled: hasRepository && idle) { model.toggleTerminal() },
            Command(id: "settings", title: "Repository settings", detail: "Identity and remotes", systemImage: "gearshape",
                    enabled: hasRepository) { model.showingRepositorySettings = true },
            Command(id: "open", title: "Open repository...", detail: "⌘O", systemImage: "folder") { model.openRepository() },
            Command(id: "clone", title: "Clone repository...", detail: "", systemImage: "square.and.arrow.down") { model.showingClone = true },
            Command(id: "content-search", title: "Search file contents", detail: "⌥⌘F · text in tracked files", systemImage: "text.magnifyingglass",
                    enabled: hasRepository) { model.searchWorkingFiles() },
            Command(id: "cleanup", title: "Clean up branches", detail: "Delete merged or inactive local branches", systemImage: "scissors",
                    enabled: hasRepository && idle) { model.showingBranchCleanup = true },
            Command(id: "gitflow", title: "GitFlow", detail: "Start or finish feature, release, and hotfix branches", systemImage: "arrow.triangle.branch",
                    enabled: hasRepository) { model.showingGitFlow = true },
            Command(id: "lfs", title: "Git LFS", detail: "Tracked patterns and large files", systemImage: "externaldrive.badge.plus",
                    enabled: hasRepository) { model.showingLFS = true },
            Command(id: "reflog", title: "Recover lost work", detail: "Commits from resets, rebases, and deleted branches", systemImage: "clock.arrow.circlepath",
                    enabled: hasRepository) { model.showingReflog = true },
            Command(id: "patch", title: "Apply patch...", detail: "", systemImage: "doc.badge.plus",
                    enabled: hasRepository && idle && clean) { model.importPatch() },
        ]
        if snapshot?.worktrees.contains(where: \.isPrunable) == true {
            list.append(Command(id: "prune-worktrees", title: "Forget missing worktrees", detail: "Worktree folders deleted outside Git",
                                systemImage: "folder.badge.minus", enabled: idle) { model.pruneWorktrees() })
        }
        if model.canUndo, let undo = model.historyDescription(redo: false) {
            list.append(Command(id: "undo", title: undo.title, detail: undo.detail, systemImage: "arrow.uturn.backward") { model.moveHistory(redo: false) })
        }
        if model.canRedo, let redo = model.historyDescription(redo: true) {
            list.append(Command(id: "redo", title: redo.title, detail: redo.detail, systemImage: "arrow.uturn.forward") { model.moveHistory(redo: true) })
        }
        if let bisect = snapshot?.bisect {
            if bisect.firstBad == nil {
                list.append(Command(id: "bisect-good", title: "Bisect: mark good", detail: "This commit does not have the problem", systemImage: "checkmark.circle", enabled: idle) { model.markBisect(.good) })
                list.append(Command(id: "bisect-bad", title: "Bisect: mark bad", detail: "This commit has the problem", systemImage: "xmark.circle", enabled: idle) { model.markBisect(.bad) })
                list.append(Command(id: "bisect-skip", title: "Bisect: skip", detail: "This commit cannot be tested", systemImage: "forward", enabled: idle) { model.markBisect(.skip) })
            }
            list.append(Command(id: "bisect-end", title: "End bisect", detail: "Return to \(bisect.originalCheckout)", systemImage: "scope", enabled: idle) { model.endBisect() })
        }
        if let operation = snapshot?.operation {
            list.append(Command(id: "continue", title: "Continue \(operation.rawValue)", detail: "After resolving conflicts", systemImage: "play",
                                enabled: idle) { model.continueOperation() })
            list.append(Command(id: "abort", title: "Abort \(operation.rawValue)", detail: "Return to the state before it started", systemImage: "xmark.octagon",
                                enabled: idle) { model.abortOperation() })
        }
        for branch in snapshot?.branches ?? [] where !branch.isCurrent {
            list.append(Command(id: (branch.isRemote ? "remote:" : "local:") + branch.name,
                                title: "Switch to \(branch.isRemote ? branch.displayName : branch.name)",
                                detail: branch.isRemote ? "Remote branch" : "Local branch",
                                systemImage: branch.isRemote ? "network" : "arrow.triangle.branch",
                                enabled: idle && clean) { model.checkout(branch: branch) })
        }
        for profile in model.identityProfiles {
            list.append(Command(id: "profile:" + profile.id.uuidString, title: "Use identity \(profile.name)",
                                detail: profile.email + (profile.signingKey == nil ? "" : " · signs commits"), systemImage: "person.crop.circle",
                                enabled: hasRepository && idle) { model.applyIdentityProfile(profile) })
        }
        for repository in model.recentRepositories where repository.path != snapshot?.rootPath {
            list.append(Command(id: "repository:" + repository.path,
                                title: "Open \(URL(fileURLWithPath: repository.path).lastPathComponent)",
                                detail: repository.path, systemImage: "externaldrive",
                                enabled: idle) { model.loadRepository(at: URL(fileURLWithPath: repository.path)) })
        }
        return list
    }

    /// Orders commands by how well `query` matches their title: every query character must
    /// appear in order; earlier, word-starting, and consecutive matches rank higher.
    static func ranked(_ commands: [Command], query: String) -> [Command] {
        let needle = Array(query.lowercased().filter { !$0.isWhitespace })
        guard !needle.isEmpty else { return commands }
        return commands.compactMap { command in score(Array(command.title.lowercased()), needle).map { (command, $0) } }
            .sorted { $0.1 > $1.1 }.map(\.0)
    }

    static func score(_ title: [Character], _ needle: [Character]) -> Int? {
        var score = 0, position = 0, previous = -2
        for character in needle {
            guard let found = title[position...].firstIndex(of: character) else { return nil }
            let wordStart = found == 0 || !title[found - 1].isLetter
            score += (wordStart ? 8 : 1) + (found == previous + 1 ? 8 : 0) - min(found, 20) / 4
            previous = found
            position = found + 1
        }
        return score
    }
}
