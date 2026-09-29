import NiceGitCore
import SwiftUI
private struct RepositorySidebar: View {
    @EnvironmentObject private var model: AppModel
    @State private var newBranchName = ""
    @State private var showingNewBranch = false
    @State private var branchSource: GitBranch?
    @State private var newBranchCheckout: (branch: String, head: String?)?
    @State private var branchToRename: GitBranch?
    @State private var branchToDelete: GitBranch?
    @State private var pushRequest: (branch: GitBranch, remote: String)?
    @State private var integrationRequest: (operation: GitOperation, branch: GitBranch, currentBranch: String, head: String?)?
    /// A branch dropped onto the current branch, awaiting a choice of merge or rebase.
    @State private var droppedBranch: (branch: GitBranch, currentBranch: String, head: String?)?
    @State private var renamedBranchName = ""
    @State private var tagToDelete: (name: String, tip: String)?
    @State private var worktreeToRemove: String?
    @State private var remoteTagToDelete: (name: String, remote: String, tip: String, addresses: [String: [String]])?
    @State private var referenceQuery = ""
    @State private var resetRequest: ResetRequest?

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            Menu {
                Button("Open repository...") { model.openRepository() }
                Button("Clone repository...") { model.showingClone = true }
                Divider()
                ForEach(model.recentRepositories) { repository in
                    Button(repository.name) {
                        model.loadRepository(at: URL(fileURLWithPath: repository.path))
                    }.help(repository.path)
                }
                if model.snapshot != nil {
                    Divider()
                    Button("Repository settings...") { model.showingRepositorySettings = true }
                }
            } label: {
                Label(model.snapshot?.name ?? "Choose repository", systemImage: "shippingbox")
            }
            .menuStyle(.borderlessButton)
            .font(.system(size: 16, weight: .semibold))
            .padding(16)
            .help(model.snapshot?.rootPath ?? "Choose a repository")

            if let snapshot = model.snapshot {
                HStack {
                    Image(systemName: "magnifyingglass").foregroundStyle(.secondary)
                    TextField("Filter references", text: $referenceQuery).textFieldStyle(.plain)
                    if !referenceQuery.isEmpty {
                        Button { referenceQuery = "" } label: { Image(systemName: "xmark.circle.fill") }
                            .buttonStyle(.plain).help("Clear filter")
                    }
                    Button {
                        branchSource = nil
                        newBranchCheckout = (snapshot.currentBranch, snapshot.headHash)
                        showingNewBranch = true
                    } label: { Image(systemName: "plus") }
                        .buttonStyle(.plain).help("Create branch").disabled(snapshot.operation != nil)
                }.padding(.horizontal, 16).padding(.bottom, 12)

                ScrollView {
                    VStack(spacing: 0) {
                        SidebarSection(title: "Local", icon: "laptopcomputer", count: snapshot.branches.filter { !$0.isRemote }.count) {
                            let branches = snapshot.branches.filter { !$0.isRemote && matches($0.displayName) }
                            if branches.isEmpty { empty("No local branches") }
                            ForEach(branches) { branch in branchRow(branch, snapshot: snapshot) }
                        }
                        SidebarSection(title: "Remote", icon: "network", count: snapshot.branches.filter(\.isRemote).count) {
                            let branches = snapshot.branches.filter { $0.isRemote && matches($0.displayName) }
                            ForEach(snapshot.remotes, id: \.self) { remote in
                                DisclosureGroup(remote) {
                                    ForEach(branches.filter { $0.remoteName(among: snapshot.remotes) == remote }) { branch in
                                        branchRow(branch, snapshot: snapshot)
                                    }
                                }.padding(.horizontal, 12).padding(.vertical, 4)
                            }
                            if snapshot.remotes.isEmpty { empty("No remotes configured") }
                        }
                        SubmodulesSidebarSection(snapshot: snapshot)
                        SidebarSection(title: "Worktrees", icon: "square.split.2x2", count: snapshot.worktrees.count) {
                            ForEach(snapshot.worktrees.filter { matches($0.branch ?? "") || matches($0.path) }) { worktree in
                                SidebarButton(
                                    title: worktree.branch ?? (worktree.isBare ? "Bare repository" : "Detached HEAD"),
                                    subtitle: (worktree.isPrunable ? "Unavailable - " : worktree.isLocked ? "Locked - " : "") + worktree.path,
                                    systemImage: worktree.isPrunable ? "exclamationmark.folder" : worktree.isLocked ? "lock" : "folder",
                                    isSelected: worktree.path == snapshot.rootPath
                                ) { model.loadRepository(at: URL(fileURLWithPath: worktree.path)) }
                                .disabled(worktree.isBare || worktree.isPrunable)
                                .help(worktree.path)
                                .contextMenu {
                                    Button("Open worktree") {
                                        model.loadRepository(at: URL(fileURLWithPath: worktree.path))
                                    }
                                    .disabled(worktree.isBare || worktree.isPrunable)
                                    Button("Reveal in Finder") {
                                        NSWorkspace.shared.activateFileViewerSelecting([URL(fileURLWithPath: worktree.path)])
                                    }
                                    .disabled(worktree.isPrunable)
                                    Divider()
                                    Button("Copy worktree path") {
                                        NSPasteboard.general.clearContents()
                                        NSPasteboard.general.setString(worktree.path, forType: .string)
                                    }
                                    Divider()
                                    Button("Remove worktree...", role: .destructive) { worktreeToRemove = worktree.path }
                                        .disabled(worktree.path == snapshot.rootPath || worktree.id == snapshot.worktrees.first?.id || worktree.isLocked || worktree.isBare)
                                    if snapshot.worktrees.contains(where: \.isPrunable) {
                                        Button("Forget missing worktrees") { model.pruneWorktrees() }
                                    }
                                }
                            }
                        }
                        SidebarSection(title: "Tags", icon: "tag", count: snapshot.tags.count) {
                            let tags = snapshot.tags.filter { matches($0) }
                            if tags.isEmpty { empty("No tags") }
                            ForEach(tags, id: \.self) { tag in
                                SidebarButton(title: tag, subtitle: "", systemImage: "tag", isSelected: false) { model.inspectTag(tag) }
                                    .contextMenu {
                                        ForEach(snapshot.remotes, id: \.self) { remote in
                                            Button("Push tag to \(remote)") {
                                                if let tip = snapshot.tagTips[tag] {
                                                    model.pushTag(tag, to: remote, expectedTip: tip, expectedPushAddresses: snapshot.remotePushAddresses)
                                                }
                                            }
                                        }
                                        Divider()
                                        Button("Delete local tag...", role: .destructive) {
                                            if let tip = snapshot.tagTips[tag] { tagToDelete = (tag, tip) }
                                        }
                                        ForEach(snapshot.remotes, id: \.self) { remote in
                                            Button("Delete tag from \(remote)...", role: .destructive) {
                                                if let tip = snapshot.tagTips[tag] { remoteTagToDelete = (tag, remote, tip, snapshot.remotePushAddresses) }
                                            }
                                        }
                                    }
                            }
                        }
                        SidebarSection(title: "Stashes", icon: "archivebox", count: snapshot.stashes.count) {
                            if snapshot.stashes.isEmpty { empty("No stashes") }
                            ForEach(snapshot.stashes.filter { matches($0.message) || matches($0.reference) }) { stash in
                                SidebarButton(title: stash.reference, subtitle: stash.message, systemImage: "archivebox", isSelected: false) {
                                    model.showingStashes = true
                                }
                            }
                            Button("Manage stashes...") { model.showingStashes = true }
                                .buttonStyle(.plain).padding(12)
                        }
                        GitHubSidebarSection(kind: .pullRequest, rootPath: snapshot.rootPath, remotes: snapshot.remotes).id(snapshot.rootPath)
                        GitHubSidebarSection(kind: .issue, rootPath: snapshot.rootPath, remotes: snapshot.remotes).id(snapshot.rootPath)
                    }.padding(.bottom, 16)
                }
            } else {
                Button("Open repository...") { model.openRepository() }.padding(16)
                Spacer()
            }
            Divider()
            HStack {
                Text("NiceGit").font(.caption.weight(.semibold)).foregroundStyle(.secondary)
                Spacer()
                Button { model.showingRepositorySettings = true } label: { Image(systemName: "gearshape") }
                    .buttonStyle(.plain).help("Repository settings").disabled(model.snapshot == nil)
            }.padding(12)
        }
        .background(AppPalette.sidebar)
        .modifier(ResetConfirmation(request: $resetRequest))
        .onChange(of: model.snapshot?.rootPath) { referenceQuery = "" }
        .confirmationDialog(integrationTitle, isPresented: Binding(get: { integrationRequest != nil }, set: { if !$0 { integrationRequest = nil } })) {
            if let request = integrationRequest {
                Button(request.operation == .rebase ? "Rebase branch" : "Merge branch") {
                    model.start(request.operation, target: request.branch.tip, expectedHead: request.head, expectedBranch: request.currentBranch, expectedSourceBranch: request.branch)
                    integrationRequest = nil
                }
            }
        } message: {
            if let request = integrationRequest {
                Text(request.operation == .rebase
                    ? "Replays commits from \(request.currentBranch) onto \(request.branch.displayName) at \(request.branch.tip.prefix(8)). This rewrites commit IDs. Coordinate before rebasing commits already shared with others."
                    : "Merges \(request.branch.displayName) at \(request.branch.tip.prefix(8)) into \(request.currentBranch). Conflicts may need resolving before the merge can finish.")
            }
        }
        .modifier(BranchDropConfirmation(drop: $droppedBranch))
        .confirmationDialog("Push \(pushRequest?.branch.name ?? "")?", isPresented: Binding(get: { pushRequest != nil }, set: { if !$0 { pushRequest = nil } })) {
            if let request = pushRequest {
                Button("Push to \(request.remote)/\(request.branch.name)") { model.push(request.branch, to: request.remote) }
            }
        } message: {
            Text("Updates the same-named branch on the selected remote without force-pushing. Your checkout and upstream settings stay unchanged.")
        }
        .alert(branchSource.map { "Create branch from \($0.displayName)" } ?? "Create branch at HEAD", isPresented: $showingNewBranch) {
            TextField("Branch name", text: $newBranchName)
            Button("Cancel", role: .cancel) {}
            Button(branchSource == nil ? "Create and checkout" : "Create branch") {
                if let branchSource {
                    model.createBranch(named: newBranchName, from: branchSource) { newBranchName = "" }
                } else if let newBranchCheckout {
                    model.createBranch(named: newBranchName, expectedBranch: newBranchCheckout.branch, expectedHead: newBranchCheckout.head) { newBranchName = "" }
                }
            }
                .disabled(newBranchName.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || (branchSource == nil && (newBranchCheckout == nil || model.snapshot?.operation != nil)))
        }


        .confirmationDialog("Delete local tag \(tagToDelete?.name ?? "")?", isPresented: Binding(get: { tagToDelete != nil }, set: { if !$0 { tagToDelete = nil } })) {
            if let tagToDelete {
                Button("Delete local tag", role: .destructive) { model.deleteTag(name: tagToDelete.name, expectedTip: tagToDelete.tip) }
            }
        }
        .confirmationDialog("Remove worktree?", isPresented: Binding(get: { worktreeToRemove != nil }, set: { if !$0 { worktreeToRemove = nil } })) {
            if let path = worktreeToRemove {
                Button("Remove worktree", role: .destructive) { model.removeWorktree(at: path) }
            }
        } message: {
            Text("Deletes the folder \(worktreeToRemove ?? "") and unregisters it. Git refuses if it has uncommitted or untracked files. Its branch and commits are kept.")
        }
        .confirmationDialog("Delete tag \(remoteTagToDelete?.name ?? "") from \(remoteTagToDelete?.remote ?? "")?",
                            isPresented: Binding(get: { remoteTagToDelete != nil }, set: { if !$0 { remoteTagToDelete = nil } })) {
            if let request = remoteTagToDelete {
                Button("Delete from \(request.remote)", role: .destructive) {
                    model.deleteRemoteTag(request.name, from: request.remote, expectedTip: request.tip, expectedPushAddresses: request.addresses)
                }
            }
        } message: {
            Text("The tag is removed from the remote for everyone who fetches from it, but only if it still points where your local tag does. Your local tag is kept.")
        }
        .alert("Rename branch", isPresented: Binding(get: { branchToRename != nil }, set: { if !$0 { branchToRename = nil } })) {
            TextField("Branch name", text: $renamedBranchName)
            Button("Cancel", role: .cancel) {}
            Button("Rename") {
                if let branchToRename { model.renameBranch(branchToRename, to: renamedBranchName) }
            }
        }
        .confirmationDialog("Delete branch \(branchToDelete?.name ?? "")?", isPresented: Binding(get: { branchToDelete != nil }, set: { if !$0 { branchToDelete = nil } })) {
            if let branchToDelete {
                Button("Delete branch", role: .destructive) { model.deleteBranch(branchToDelete) }
            }
        } message: {
            Text("Git will refuse to delete a branch with unmerged changes.")
        }
    }

    private func matches(_ text: String) -> Bool {
        referenceQuery.isEmpty || text.localizedCaseInsensitiveContains(referenceQuery)
    }

    private func empty(_ message: String) -> some View {
        Text(referenceQuery.isEmpty ? message : "No matching items")
            .font(.caption).foregroundStyle(.secondary)
            .frame(maxWidth: .infinity, alignment: .leading).padding(.horizontal, 18).padding(.vertical, 10)
    }

    private func branchRow(_ branch: GitBranch, snapshot: RepositorySnapshot) -> some View {
        SidebarButton(title: branch.displayName, subtitle: "", systemImage: branch.isCurrent ? "checkmark.circle.fill" : "arrow.triangle.branch", isSelected: branch.isCurrent, isDisabled: branch.isCurrent || snapshot.operation != nil) {
            model.checkout(branch: branch)
        }
        .draggable(branchDragIdentity(branch))
        .dropDestination(for: String.self) { items, _ in
            // Dropping another branch on the current one offers to merge or rebase with it.
            guard branch.isCurrent, snapshot.operation == nil, let item = items.first,
                  let dropped = snapshot.branches.first(where: { branchDragIdentity($0) == item }), !dropped.isCurrent else { return false }
            droppedBranch = (dropped, snapshot.currentBranch, snapshot.headHash)
            return true
        } isTargeted: { _ in }
        .contextMenu {
            Button(branch.isRemote ? "Checkout tracking branch" : "Checkout branch") { model.checkout(branch: branch) }
                .disabled(branch.isCurrent || snapshot.operation != nil)
            if branch.isCurrent {
                Divider()
                Button("Pull (fast-forward only)") { model.pull() }
                Button("Push...") { model.push() }
            }
            Button("Fetch all remotes") { model.fetch() }.disabled(snapshot.remotes.isEmpty)
            if !branch.isRemote {
                Menu("Push branch to") {
                    ForEach(snapshot.remotes, id: \.self) { remote in
                        Button("\(remote)/\(branch.name)...") { pushRequest = (branch, remote) }
                    }
                }.disabled(snapshot.remotes.isEmpty)
                Menu("Set upstream") {
                    ForEach(snapshot.branches.filter(\.isRemote)) { remote in
                        Button {
                            model.setUpstream(for: branch, to: remote)
                        } label: {
                            if branch.upstream == "refs/" + remote.name {
                                Label(remote.displayName, systemImage: "checkmark")
                            } else {
                                Text(remote.displayName)
                            }
                        }
                    }
                    Divider()
                    Button("Remove upstream") { model.setUpstream(for: branch, to: nil) }
                        .disabled(branch.upstream == nil)
                }
            }
            Divider()
            Button("Create branch here...") { branchSource = branch; showingNewBranch = true }
            Button("Create tag here...") {
                model.taggingCommit = GitCommit(hash: branch.tip, shortHash: String(branch.tip.prefix(8)), parents: [], refs: [], subject: branch.subject, authorName: "", authorEmail: "", relativeDate: "")
            }
            if !branch.isRemote {
                Button("Create worktree from branch...") { model.createWorktree(for: branch) }
                    .disabled(snapshot.worktrees.contains { $0.branch == branch.name })
            }
            Button("Merge into \(snapshot.currentBranch)...") { integrationRequest = (.merge, branch, snapshot.currentBranch, snapshot.headHash) }
                .disabled(branch.isCurrent || snapshot.operation != nil)
            Button("Rebase \(snapshot.currentBranch) onto this branch...") { integrationRequest = (.rebase, branch, snapshot.currentBranch, snapshot.headHash) }
                .disabled(branch.isCurrent || snapshot.operation != nil)
            Menu("Reset \(snapshot.currentBranch) to \(branch.displayName)") {
                ForEach(GitResetMode.allCases, id: \.self) { mode in
                    Button(mode.title + "...") {
                        if let head = snapshot.headHash {
                            resetRequest = ResetRequest(target: branch.tip, mode: mode, branch: snapshot.currentBranch, head: head)
                        }
                    }
                }
            }.disabled(snapshot.operation != nil || snapshot.headHash == nil)
            Divider()
            if !branch.isRemote {
                Button("Rename \(branch.name)...") { renamedBranchName = branch.name; branchToRename = branch }
                Button("Delete \(branch.name)...", role: .destructive) { branchToDelete = branch }
                    .disabled(snapshot.worktrees.contains { $0.branch == branch.name } || branch.isCurrent)
                Divider()
            }
            Button("Copy branch name") {
                NSPasteboard.general.clearContents()
                NSPasteboard.general.setString(branch.displayName, forType: .string)
            }
            Button("Copy commit SHA") {
                NSPasteboard.general.clearContents()
                NSPasteboard.general.setString(branch.tip, forType: .string)
            }
            if !snapshot.remotes.isEmpty {
                Menu("Copy GitHub commit link") {
                    ForEach(snapshot.remotes, id: \.self) { remote in
                        Button(remote) { model.copyCommitLink(hash: branch.tip, remote: remote) }
                    }
                }
            }
        }
    }

    private var integrationTitle: String {
        guard let request = integrationRequest else { return "Integrate branch?" }
        return request.operation == .rebase
            ? "Rebase \(request.currentBranch) onto \(request.branch.displayName)?"
            : "Merge \(request.branch.displayName) into \(request.currentBranch)?"
    }
}

struct SidebarSection<Content: View>: View {
    let title: String
    let icon: String
    let count: Int?
    @ViewBuilder let content: () -> Content
    @State private var expanded = true

    var body: some View {
        VStack(spacing: 0) {
            Divider()
            Button { expanded.toggle() } label: {
                HStack(spacing: 8) {
                    Image(systemName: expanded ? "chevron.down" : "chevron.right").font(.system(size: 9, weight: .bold)).frame(width: 10)
                    Image(systemName: icon).frame(width: 16)
                    Text(title.uppercased()).font(.system(size: 11, weight: .semibold))
                    Spacer(minLength: 0)
                    if let count { Text(count.formatted()).font(.system(size: 11, design: .monospaced)).foregroundStyle(AppPalette.signal) }
                }.foregroundStyle(.secondary).padding(.horizontal, 12).frame(height: 36).contentShape(Rectangle())
            }.buttonStyle(.plain).accessibilityLabel("\(title), \(expanded ? "expanded" : "collapsed")")
            if expanded { content() }
        }
    }
}
struct ContentView: View {
    @EnvironmentObject private var model: AppModel
    @Environment(\.scenePhase) private var scenePhase
    @AppStorage("NiceGit.repositorySidebarCollapsed") private var repositorySidebarCollapsed = false

    var body: some View {
        HStack(spacing: 0) {
            OpenRepositoryTabs(isCollapsed: $repositorySidebarCollapsed)
            Divider()
        HSplitView {
            RepositorySidebar()
                .frame(minWidth: 220, idealWidth: 240, maxWidth: 280)
            Group {
                if let snapshot = model.snapshot {
                    WorkbenchView(snapshot: snapshot)
                } else {
                    EmptyRepositoryView()
                }
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .background(AppPalette.canvas)
        }
        }
        .frame(minWidth: 1120, minHeight: 720)
        .toolbar(.hidden, for: .windowToolbar)
        .ignoresSafeArea(.container, edges: .top)
        .tint(AppPalette.signal)
        .task { model.restoreSession() }
        .onChange(of: scenePhase) { _, phase in
            if phase == .active { model.refreshOnActivation() }
        }
        .disabled(model.isLoading)
        .overlay {
            if model.isLoading {
                ProgressView()
                    .controlSize(.large)
                    .padding(22)
                    .background(.regularMaterial, in: RoundedRectangle(cornerRadius: 12))
            }
        }
        .sheet(item: $model.diffSelection, onDismiss: model.refreshAfterReview) { selection in
            if selection.conflicted { ConflictView(selection: selection) }
            else { DiffView(selection: selection) }
        }
        .sheet(isPresented: $model.showingClone, onDismiss: model.refreshAfterReview) { CloneRepositoryView() }
        .sheet(isPresented: $model.showingStashes, onDismiss: model.refreshAfterReview) { StashView() }
        .sheet(isPresented: $model.showingPublish, onDismiss: model.refreshAfterReview) { PublishBranchView() }
        .sheet(isPresented: $model.showingRepositorySettings, onDismiss: model.refreshAfterReview) { RepositorySettingsView() }
        .sheet(item: $model.taggingCommit, onDismiss: model.refreshAfterReview) { TagView(commit: $0) }
        .sheet(item: $model.editingCommitMessage, onDismiss: model.refreshAfterReview) { CommitMessageView(commit: $0) }
        .sheet(item: $model.fileHistoryRequest) { FileHistoryView(request: $0) }
        .sheet(item: $model.blameRequest) { BlameView(request: $0) }
        .sheet(item: $model.rebaseRequest, onDismiss: model.refreshAfterReview) { InteractiveRebaseView(request: $0) }
        .sheet(isPresented: $model.showingCommandPalette) { CommandPaletteView() }
        .sheet(item: $model.compareRequest) { CompareView(request: $0) }
        .sheet(isPresented: $model.showingGitFlow, onDismiss: model.refreshAfterReview) { GitFlowView() }
        .sheet(isPresented: $model.showingLFS, onDismiss: model.refreshAfterReview) { LFSView() }
        .sheet(isPresented: $model.showingReflog, onDismiss: model.refreshAfterReview) {
            if let url = model.repositoryURL { ReflogView(repositoryURL: url) }
        }
        .alert(
            model.errorMessage == nil ? "Branch switched" : "Git needs attention",
            isPresented: Binding(
                get: { model.errorMessage != nil || model.noticeMessage != nil },
                set: { if !$0 { model.errorMessage = nil; model.noticeMessage = nil } }
            )
        ) {
            Button("OK", role: .cancel) {}
        } message: {
            Text(model.errorMessage ?? model.noticeMessage ?? "")
        }
    }
}


private struct SidebarHeader: View {
    var title: String

    var body: some View {
        Text(title.uppercased())
            .font(.system(size: 11, weight: .semibold, design: .monospaced))
            .foregroundStyle(.secondary)
            .padding(.horizontal, 18)
            .padding(.bottom, 7)
    }
}

private struct SidebarButton: View {
    var title: String
    var subtitle: String
    var systemImage: String
    var isSelected: Bool
    var isDisabled = false
    var action: () -> Void

    var body: some View {
        Button(action: action) {
            HStack(alignment: .top, spacing: 10) {
                Image(systemName: systemImage)
                    .font(.system(size: 15, weight: .semibold))
                    .foregroundStyle(isSelected ? AppPalette.signal : .secondary)
                    .frame(width: 18)

                VStack(alignment: .leading, spacing: 3) {
                    Text(title)
                        .lineLimit(1)
                        .font(.system(size: 13, weight: .semibold))
                    if !subtitle.isEmpty {
                        Text(subtitle)
                            .lineLimit(1)
                            .font(.system(size: 11))
                            .foregroundStyle(.secondary)
                    }
                }

                Spacer(minLength: 0)
            }
            .padding(.horizontal, 10)
            .padding(.vertical, 8)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(isSelected ? AppPalette.selection : Color.clear)
            .contentShape(Rectangle())
            .clipShape(RoundedRectangle(cornerRadius: 8))
        }
        .buttonStyle(.plain)
        .disabled(isDisabled)
        .padding(.horizontal, 8)
        .help(title)
    }
}

private struct EmptyRepositoryView: View {
    @EnvironmentObject private var model: AppModel

    var body: some View {
        VStack(spacing: 22) {
            ZStack {
                RoundedRectangle(cornerRadius: 8)
                    .strokeBorder(AppPalette.line, lineWidth: 1)
                    .frame(width: 360, height: 180)
                CommitRail(index: 0, parentCount: 2)
                    .frame(width: 62, height: 132)
                    .offset(x: -118)
                VStack(alignment: .leading, spacing: 14) {
                    Capsule()
                        .fill(AppPalette.ink)
                        .frame(width: 156, height: 13)
                    Capsule()
                        .fill(AppPalette.line)
                        .frame(width: 242, height: 10)
                    Capsule()
                        .fill(AppPalette.line)
                        .frame(width: 198, height: 10)
                }
                .offset(x: 42)
            }

            VStack(spacing: 8) {
                Text("Open a repository")
                    .font(.system(size: 32, weight: .bold, design: .rounded))
            }

            Button {
                model.openRepository()
            } label: {
                Label("Choose repository", systemImage: "folder")
            }
            .buttonStyle(.borderedProminent)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .padding(40)
    }
}

private struct WorkbenchView: View {
    @EnvironmentObject private var model: AppModel
    let snapshot: RepositorySnapshot
    @State private var selectedCommit: GitCommit?
    @State private var commitDiff: DiffSelection?
    @State private var selectedStash: GitStash?

    var body: some View {
        VStack(spacing: 0) {
            VSplitView {
            HSplitView {
                VStack(spacing: 0) {
                    RepositoryActionBar(snapshot: snapshot)
                    if let operation = snapshot.operation {
                        OperationBar(operation: operation, hasConflicts: snapshot.status.contains { $0.kind == .conflicted })
                    }
                    Divider()
                Group {
                    if let selection = model.fileReviewSelection {
                        FileReviewView(selection: selection).id(selection.id)
                    } else if let commitDiff {
                        DiffView(selection: commitDiff, onClose: { self.commitDiff = nil }).id(commitDiff.id)
                    } else {
                        GraphWorkspace(snapshot: snapshot, selectedCommit: $selectedCommit, selectedStash: $selectedStash)
                    }
                }
                }.frame(minWidth: 450)
                Group {
                    if let selectedStash, model.fileReviewSelection == nil {
                        StashInspector(stash: selectedStash, repositoryURL: URL(fileURLWithPath: snapshot.rootPath)) {
                            self.selectedStash = nil
                        }
                    } else if let selectedCommit, model.fileReviewSelection == nil {
                        CommitInspector(commit: selectedCommit, repositoryURL: URL(fileURLWithPath: snapshot.rootPath), showFile: { commitDiff = $0 }) {
                            self.selectedCommit = nil
                            commitDiff = nil
                        }
                    } else {
                        ChangesPanel(snapshot: snapshot, commitMessage: Binding(
                            get: { model.commitDraft(for: snapshot.rootPath) },
                            set: { model.setCommitDraft($0, for: snapshot.rootPath) }
                        ))
                    }
                }
                .frame(minWidth: 300, idealWidth: 350, maxWidth: 480)
            }
            .frame(minHeight: 260)
            if model.showingTerminal, let session = model.activeTerminal {
                TerminalPanel(session: session).frame(minHeight: 140, idealHeight: 240)
            }
            }
        }
        .onChange(of: snapshot.rootPath) {
            commitDiff = nil
            model.compareMark = nil
            selectedCommit = nil
            selectedStash = nil
        }
        .onChange(of: snapshot.commits) { previous, commits in
            // A commit opened from history search may lie outside the loaded page; keep it
            // unless it was on the page and has now left it.
            if let selected = selectedCommit {
                selectedCommit = commits.first { $0.hash == selected.hash }
                    ?? (previous.contains { $0.hash == selected.hash } ? nil : selected)
            }
        }
        .onChange(of: selectedCommit?.hash) { commitDiff = nil }
        .onChange(of: snapshot.stashes) { _, stashes in
            if let selected = selectedStash { selectedStash = stashes.first { $0.hash == selected.hash } }
        }
    }
}



private struct CommitRail: View {
    let index: Int
    let parentCount: Int

    var color: Color {
        AppPalette.laneColors[index % AppPalette.laneColors.count]
    }

    var body: some View {
        Canvas { context, size in
            let x = size.width * 0.5
            let center = CGPoint(x: x, y: size.height * 0.5)
            var vertical = Path()
            vertical.move(to: CGPoint(x: x, y: 0))
            vertical.addLine(to: CGPoint(x: x, y: size.height))
            context.stroke(vertical, with: .color(color.opacity(0.55)), lineWidth: 2.5)

            if parentCount > 1 {
                var merge = Path()
                merge.move(to: center)
                merge.addCurve(
                    to: CGPoint(x: size.width * 0.82, y: size.height),
                    control1: CGPoint(x: size.width * 0.76, y: size.height * 0.48),
                    control2: CGPoint(x: size.width * 0.82, y: size.height * 0.72)
                )
                context.stroke(merge, with: .color(AppPalette.merge), lineWidth: 2)
            }

            context.fill(
                Path(ellipseIn: CGRect(x: center.x - 7, y: center.y - 7, width: 14, height: 14)),
                with: .color(color)
            )
            context.stroke(
                Path(ellipseIn: CGRect(x: center.x - 7, y: center.y - 7, width: 14, height: 14)),
                with: .color(AppPalette.canvas),
                lineWidth: 2
            )
        }
    }
}

private struct ChangesPanel: View {
    @EnvironmentObject private var model: AppModel
    let snapshot: RepositorySnapshot
    @Binding var commitMessage: String
    @State private var treeMode = false
    @State private var ascending = true
    /// The checkout captured when amending was switched on; the amend is refused if it moves.
    @State private var amendTarget: (branch: String, head: String, published: Bool)?
    @State private var messageBeforeAmend = ""

    private var summary: Binding<String> {
        Binding(get: { commitMessage.components(separatedBy: "\n").first ?? "" }, set: {
            let body = description.wrappedValue
            commitMessage = $0 + (body.isEmpty ? "" : "\n\n" + body)
        })
    }

    private var description: Binding<String> {
        Binding(get: {
            var lines = commitMessage.components(separatedBy: "\n").dropFirst()
            if lines.first == "" { lines = lines.dropFirst() }
            return lines.joined(separator: "\n")
        }, set: { commitMessage = summary.wrappedValue + ($0.isEmpty ? "" : "\n\n" + $0) })
    }

    /// Switching amend on loads the last commit's message into an empty draft; switching it
    /// off restores the draft that was there before.
    private func setAmending(_ on: Bool) {
        guard on else {
            if amendTarget != nil { commitMessage = messageBeforeAmend }
            amendTarget = nil
            return
        }
        guard let head = snapshot.headHash else { return }
        let branch = snapshot.currentBranch, url = URL(fileURLWithPath: snapshot.rootPath)
        messageBeforeAmend = commitMessage
        amendTarget = (branch, head, false)
        Task {
            let loaded = await Task.detached { () -> (String, Bool)? in
                let git = GitClient()
                guard let message = try? git.commitMessage(hash: head, in: url) else { return nil }
                return (message.trimmingCharacters(in: .whitespacesAndNewlines), (try? git.isPublished(head, in: url)) ?? false)
            }.value
            guard let loaded, amendTarget?.head == head else { return }
            amendTarget = (branch, head, loaded.1)
            if commitMessage.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty { commitMessage = loaded.0 }
        }
    }

    private func sorted(_ entries: [GitStatusEntry]) -> [GitStatusEntry] {
        entries.sorted { ascending ? $0.path.localizedStandardCompare($1.path) == .orderedAscending : $0.path.localizedStandardCompare($1.path) == .orderedDescending }
    }

    private var staged: [GitStatusEntry] {
        sorted(snapshot.status.filter(\.isStaged))
    }

    private var unstaged: [GitStatusEntry] {
        sorted(snapshot.status.filter(\.isUnstaged))
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack {
                Text("\(snapshot.status.count) file changes on")
                    .font(.system(size: 12, weight: .medium))
                    .fixedSize()
                Text(snapshot.currentBranch)
                    .font(.system(size: 12, weight: .semibold))
                    .lineLimit(1).truncationMode(.middle)
                    .padding(.horizontal, 6).padding(.vertical, 4)
                    .background(AppPalette.branchTag, in: RoundedRectangle(cornerRadius: 3))
                    .help(snapshot.currentBranch)
                Spacer(minLength: 0)
            }
            .padding(.horizontal, 18)
            .padding(.vertical, 14)

            Divider()
            HStack {
                Button { ascending.toggle() } label: {
                    Image(systemName: ascending ? "arrow.down" : "arrow.up")
                }.buttonStyle(.plain).help(ascending ? "Sort paths Z to A" : "Sort paths A to Z")
                    .disabled(treeMode)
                Spacer()
                Picker("File view", selection: $treeMode) {
                    Label("Path", systemImage: "list.bullet").tag(false)
                    Label("Tree", systemImage: "folder").tag(true)
                }.pickerStyle(.segmented).frame(width: 180)
                Spacer()
            }
            .padding(.horizontal, 14).padding(.vertical, 10)

            if let undo = model.latestDiscardUndo {
                HStack(spacing: 8) {
                    Image(systemName: "arrow.uturn.backward.circle").foregroundStyle(.secondary)
                    Text("Discarded changes to \(undo.path)").font(.system(size: 12)).lineLimit(1).truncationMode(.middle)
                    Spacer(minLength: 4)
                    Button("Undo") { model.undoLatestDiscard() }.disabled(model.isLoading)
                        .help("Put back the staged and unstaged changes, if the file has not changed since")
                    Button { model.forgetDiscardUndos() } label: { Image(systemName: "xmark") }.buttonStyle(.plain)
                        .help("Dismiss").accessibilityLabel("Dismiss undo")
                }
                .padding(.horizontal, 14).padding(.vertical, 6)
                .background(AppPalette.signal.opacity(0.10))
            }
            VSplitView {
                    ChangeSection(title: "Unstaged Files", entries: unstaged, emptyText: "No local changes", treeMode: treeMode, actionTitle: "Stage All Changes", actionColor: AppPalette.signal, action: { model.stageAll() }) { entry in
                        FileChangeRow(entry: entry, treeMode: treeMode, primarySystemImage: "plus.circle", primaryHelp: "Stage file") {
                            model.stage(entry)
                        } discard: {
                            model.discard(entry)
                        }
                    }.frame(minHeight: 110, maxHeight: .infinity)
                    ChangeSection(title: "Staged Files", entries: staged, emptyText: "Nothing staged", treeMode: treeMode, actionTitle: "Unstage All Changes", actionColor: AppPalette.conflict, action: { model.unstageAll() }) { entry in
                        FileChangeRow(entry: entry, treeMode: treeMode, primarySystemImage: "minus.circle", primaryHelp: "Unstage file") {
                            model.unstage(entry)
                        } discard: {
                            model.discard(entry)
                        }
                    }.frame(minHeight: 110, maxHeight: .infinity)
            }

            Divider()

            VStack(alignment: .leading, spacing: 10) {
                HStack {
                    Label("Commit", systemImage: "point.topleft.down.to.point.bottomright.curvepath")
                        .font(.system(size: 14, weight: .semibold))
                    Spacer()
                }
                VStack(alignment: .leading, spacing: 10) {
                    HStack(alignment: .top) {
                        TextField("Commit summary", text: summary, axis: .vertical)
                            .font(.system(size: 14)).lineLimit(1...3)
                        Text("\(72 - summary.wrappedValue.count)")
                            .font(.system(size: 12, design: .monospaced))
                            .foregroundStyle(summary.wrappedValue.count > 72 ? Color.orange : Color.secondary)
                            .help("Characters remaining in the recommended 72-character summary")
                    }
                    TextField("Description", text: description, axis: .vertical)
                        .font(.system(size: 13)).lineLimit(4...6)
                }
                .textFieldStyle(.plain).padding(12)
                .background(AppPalette.canvas, in: RoundedRectangle(cornerRadius: 5))
                .overlay(RoundedRectangle(cornerRadius: 5).stroke(AppPalette.line))

                Toggle("Amend last commit", isOn: Binding(get: { amendTarget != nil }, set: { setAmending($0) }))
                    .toggleStyle(.checkbox).font(.system(size: 12))
                    .disabled(snapshot.headHash == nil || snapshot.operation != nil || model.isLoading)
                    .help("Replace the last commit with one that also includes the staged changes")
                if let amendTarget, amendTarget.published {
                    Label("The last commit is already on a remote. Amending it means you will need to force-push.", systemImage: "exclamationmark.triangle.fill")
                        .font(.caption).foregroundStyle(.orange)
                }
                Button {
                    if let amendTarget {
                        model.amendCommit(message: commitMessage, expectedBranch: amendTarget.branch, expectedHead: amendTarget.head) {
                            self.amendTarget = nil
                            commitMessage = ""
                        }
                    } else {
                        model.commit(message: commitMessage) { commitMessage = "" }
                    }
                } label: {
                    Label(summary.wrappedValue.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty ? "Type a Message to Commit"
                          : amendTarget != nil ? "Amend Last Commit" : "Commit Changes", systemImage: "checkmark.circle")
                        .frame(maxWidth: .infinity).padding(.vertical, 7)
                }
                .buttonStyle(.borderedProminent).tint(amendTarget != nil ? .orange : AppPalette.signal)
                .disabled((staged.isEmpty && amendTarget == nil) || summary.wrappedValue.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
            }
            .padding(14)
            .background(AppPalette.toolbar)
        }
        .background(AppPalette.panel)
        .onChange(of: snapshot.headHash) { _, head in
            if let amendTarget, amendTarget.head != head { setAmending(false) }
        }
    }
}

private struct ChangeSection<Content: View>: View {
    var title: String
    var entries: [GitStatusEntry]
    var emptyText: String
    var treeMode: Bool
    var actionTitle: String
    var actionColor: Color
    var action: () -> Void
    @State private var expanded = true
    @ViewBuilder var row: (GitStatusEntry) -> Content

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack(spacing: 6) {
                Button { expanded.toggle() } label: {
                    HStack(spacing: 5) {
                        Image(systemName: expanded ? "chevron.down" : "chevron.right").font(.system(size: 9, weight: .bold))
                        Text("\(title) (\(entries.count))").font(.system(size: 12, weight: .medium))
                    }
                }.buttonStyle(.plain).help(expanded ? "Collapse \(title)" : "Expand \(title)")
                Spacer()
                Button(actionTitle, action: action)
                    .font(.system(size: 11, weight: .semibold))
                    .buttonStyle(.plain).padding(.horizontal, 7).padding(.vertical, 5)
                    .background(actionColor.opacity(0.1))
                    .overlay(RoundedRectangle(cornerRadius: 3).stroke(actionColor.opacity(entries.isEmpty ? 0.3 : 0.8)))
                    .disabled(entries.isEmpty)
            }.padding(.horizontal, 14).padding(.vertical, 10)
            Divider()
            if expanded {
            ScrollView {
            if entries.isEmpty {
                Text(emptyText)
                    .font(.system(size: 13))
                    .foregroundStyle(.secondary)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(14)
            } else {
                LazyVStack(spacing: 0) {
                    if treeMode {
                        FileChangeTree(entries: entries, row: row)
                    } else {
                    ForEach(entries) { entry in
                        row(entry)
                    }
                    }
                }.padding(.horizontal, 8).padding(.vertical, 5)
            }
            }.frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
            } else { Spacer(minLength: 0) }
        }
    }
}

private struct FileChangeRow: View {
    @EnvironmentObject private var model: AppModel
    @State private var confirmingDiscard = false
    var entry: GitStatusEntry
    var treeMode: Bool
    var primarySystemImage: String
    var primaryHelp: String
    var primaryAction: () -> Void
    var discard: () -> Void

    var body: some View {
        HStack(spacing: 9) {
            Image(systemName: statusSymbol)
                .font(.system(size: 14, weight: .semibold))
                .foregroundStyle(statusColor)
                .frame(width: 18, height: 18)
                .accessibilityLabel(statusKind.rawValue)
                .help(statusKind.rawValue)

            Button { model.inspect(entry, staged: primaryHelp == "Unstage file") } label: {
                Text(treeMode ? entry.fileName : entry.path)
                    .font(.system(size: 13)).lineLimit(1).truncationMode(.middle)
                    .frame(maxWidth: .infinity, alignment: .leading).contentShape(Rectangle())
            }.buttonStyle(.plain)
                .help(entry.path)

            Button(action: primaryAction) {
                Image(systemName: primarySystemImage)
            }
            .buttonStyle(.plain)
            .help(primaryHelp)

            Button(role: .destructive) { confirmingDiscard = true } label: {
                Image(systemName: "trash")
            }
            .buttonStyle(.plain)
            .help("Discard all changes to this file")
        }
        .padding(.horizontal, 6).padding(.vertical, 7)
        .frame(minHeight: 34)
        .contextMenu {
            // A staged rename has no history under its new name until it is committed.
            let historyPath = entry.kind == .renamed ? entry.originalPath ?? entry.path : entry.path
            Button("Show file history") {
                if let url = model.repositoryURL { model.fileHistoryRequest = FileHistoryRequest(path: historyPath, repositoryURL: url) }
            }.disabled(entry.kind == .untracked || entry.kind == .added)
            Button("Blame") {
                if let url = model.repositoryURL { model.blameRequest = BlameRequest(path: entry.path, repositoryURL: url) }
            }.disabled(entry.kind == .untracked || entry.kind == .added || entry.kind == .deleted || entry.kind == .conflicted)
            if entry.kind == .untracked {
                Divider()
                Button("Ignore in .gitignore") { model.ignore(entry, rule: .path, scope: .shared) }
                    .disabled(GitClient.ignorePattern(for: entry.path, rule: .path) == nil)
                if let pattern = GitClient.ignorePattern(for: entry.path, rule: .fileExtension) {
                    Button("Ignore all \(pattern) files in .gitignore") { model.ignore(entry, rule: .fileExtension, scope: .shared) }
                }
                Button("Ignore on this computer only") { model.ignore(entry, rule: .path, scope: .local) }
                    .disabled(GitClient.ignorePattern(for: entry.path, rule: .path) == nil)
                    .help("Adds the rule to this repository's info/exclude file, which is not committed")
            }
            Divider()
            Button("Copy path") {
                NSPasteboard.general.clearContents()
                NSPasteboard.general.setString(entry.path, forType: .string)
            }
        }
        .confirmationDialog("Discard changes to \(entry.fileName)?", isPresented: $confirmingDiscard) {
            Button("Discard changes", role: .destructive, action: discard)
        } message: {
            let undoable = entry.originalPath == nil && entry.kind != .renamed && entry.kind != .conflicted && !entry.path.hasSuffix("/")
            Text((entry.kind == .untracked
                ? "This untracked file will be deleted."
                : "All staged and unstaged changes to this file will be discarded. Newly added files will be deleted.")
                + (undoable ? " You can undo this from the Changes panel until the file changes again." : " This cannot be undone."))
        }
    }

    private var statusKind: GitStatusKind {
        if entry.kind == .conflicted || entry.kind == .untracked { return entry.kind }
        let status = primaryHelp == "Unstage file" ? entry.indexStatus : entry.workTreeStatus
        switch status {
        case "A", "C": return .added
        case "D": return .deleted
        case "M", "T": return .modified
        case "R": return .renamed
        default: return entry.kind
        }
    }

    private var statusSymbol: String {
        switch statusKind {
        case .added, .untracked: "plus"
        case .modified, .renamed: "pencil"
        case .deleted: "minus"
        case .conflicted: "exclamationmark.triangle"
        }
    }

    private var statusColor: Color {
        switch statusKind {
        case .added, .untracked:
            AppPalette.signal
        case .modified, .renamed:
            .yellow
        case .deleted, .conflicted:
            AppPalette.conflict
        }
    }
}

enum AppPalette {
    static let canvas = Color(nsColor: .textBackgroundColor)
    static let sidebar = Color(nsColor: .windowBackgroundColor)
    static let toolbar = Color(nsColor: .controlBackgroundColor)
    static let panel = Color(nsColor: .windowBackgroundColor)
    static let ink = Color(red: 0.125, green: 0.145, blue: 0.169)
    static let line = Color(nsColor: .separatorColor)
    static let signal = Color(red: 0.349, green: 0.82, blue: 0.549)
    static let merge = Color(red: 0.969, green: 0.725, blue: 0.333)
    static let conflict = Color(red: 0.937, green: 0.435, blue: 0.424)
    static let selection = Color.primary.opacity(0.07)
    static let rowStripe = Color.primary.opacity(0.025)
    static let branchTag = Color.green.opacity(0.16)
    static let changeRow = Color(nsColor: .controlBackgroundColor)

    /// Graph line colours from the chosen palette. The first belongs to the checkout's line;
    /// yellow and red stay reserved for change and conflict states.
    static var laneColors: [Color] { LanePalette.current.colors }
}

enum LanePalette: String, CaseIterable, Identifiable {
    case standard, colorBlindSafe, muted
    static let storageKey = "NiceGit.lanePalette"
    var id: String { rawValue }

    static var current: LanePalette {
        UserDefaults.standard.string(forKey: storageKey).flatMap(LanePalette.init(rawValue:)) ?? .standard
    }

    var title: String {
        switch self {
        case .standard: "Standard"
        case .colorBlindSafe: "Colour-blind safe"
        case .muted: "Muted"
        }
    }

    var colors: [Color] {
        switch self {
        case .standard:
            [AppPalette.signal, Color(nsColor: .systemBlue), Color(nsColor: .systemPurple), Color(nsColor: .systemTeal),
             Color(nsColor: .systemPink), Color(nsColor: .systemIndigo), Color(nsColor: .systemBrown)]
        case .colorBlindSafe:
            // Okabe-Ito colours, distinguishable with the common forms of colour blindness.
            [Color(red: 0, green: 0.447, blue: 0.698), Color(red: 0.902, green: 0.624, blue: 0), Color(red: 0.337, green: 0.706, blue: 0.914),
             Color(red: 0, green: 0.620, blue: 0.451), Color(red: 0.800, green: 0.475, blue: 0.655), Color(red: 0.835, green: 0.369, blue: 0),
             Color(red: 0.6, green: 0.6, blue: 0.6)]
        case .muted:
            [Color(red: 0.42, green: 0.66, blue: 0.52), Color(red: 0.45, green: 0.56, blue: 0.72), Color(red: 0.62, green: 0.52, blue: 0.70),
             Color(red: 0.44, green: 0.64, blue: 0.66), Color(red: 0.74, green: 0.52, blue: 0.60), Color(red: 0.52, green: 0.52, blue: 0.68),
             Color(red: 0.62, green: 0.54, blue: 0.46)]
        }
    }
}

enum AppearanceSetting: String, CaseIterable, Identifiable {
    case system, light, dark
    static let storageKey = "NiceGit.appearance"
    var id: String { rawValue }
    var title: String { rawValue.capitalized }
    var colorScheme: ColorScheme? {
        switch self {
        case .system: nil
        case .light: .light
        case .dark: .dark
        }
    }
}

struct SettingsView: View {
    @AppStorage(AppearanceSetting.storageKey) private var appearance = AppearanceSetting.system
    @AppStorage(LanePalette.storageKey) private var palette = LanePalette.standard
    @AppStorage("NiceGit.splitDiff") private var splitDiff = false
    @AppStorage("NiceGit.diffIgnoresWhitespace") private var ignoreWhitespace = false

    var body: some View {
        Form {
            Picker("Appearance", selection: $appearance) {
                ForEach(AppearanceSetting.allCases) { Text($0.title).tag($0) }
            }
            Picker("Graph colours", selection: $palette) {
                ForEach(LanePalette.allCases) { Text($0.title).tag($0) }
            }
            HStack(spacing: 6) {
                ForEach(Array(palette.colors.enumerated()), id: \.offset) { _, color in
                    Circle().fill(color).frame(width: 14, height: 14)
                }
            }.accessibilityHidden(true)
            Toggle("Show diffs side by side", isOn: $splitDiff)
            Toggle("Hide whitespace-only changes in diffs", isOn: $ignoreWhitespace)
        }
        .formStyle(.grouped)
        .frame(width: 420)
        .padding()
    }
}

/// Submodules load on their own when the repository refreshes, so repositories without any
/// pay nothing and those with many do not slow every refresh.
private struct SubmodulesSidebarSection: View {
    let snapshot: RepositorySnapshot
    @EnvironmentObject private var model: AppModel
    @State private var submodules: [GitSubmodule] = []
    @State private var pendingUpdate: GitSubmodule?

    var body: some View {
        Group {
            if !submodules.isEmpty {
                SidebarSection(title: "Submodules", icon: "shippingbox", count: submodules.count) {
                    ForEach(submodules) { submodule in
                        SidebarButton(title: submodule.path, subtitle: summary(submodule), systemImage: icon(submodule), isSelected: false) {
                            open(submodule)
                        }
                        .help(submodule.path + " · recorded " + String(submodule.recordedCommit.prefix(7)))
                        .contextMenu {
                            Button("Open submodule") { open(submodule) }.disabled(submodule.state == .uninitialized)
                            Button(submodule.state == .uninitialized ? "Initialize and check out" : "Check out recorded commit...") {
                                if case .differentCommit = submodule.state { pendingUpdate = submodule } else { model.updateSubmodule(submodule.path) }
                            }.disabled(submodule.state == .upToDate || model.isLoading || snapshot.operation != nil)
                        }
                    }
                }
            }
        }
        .task(id: snapshot.lastUpdated) {
            let url = URL(fileURLWithPath: snapshot.rootPath)
            submodules = (try? await Task.detached { try GitClient().submodules(in: url) }.value) ?? []
        }
        .confirmationDialog("Check out the recorded commit in \(pendingUpdate?.path ?? "")?", isPresented: Binding(get: { pendingUpdate != nil }, set: { if !$0 { pendingUpdate = nil } })) {
            if let submodule = pendingUpdate {
                Button("Check out \(submodule.recordedCommit.prefix(7))") { model.updateSubmodule(submodule.path) }
            }
        } message: {
            Text("The submodule moves from its current commit to the one this repository records, leaving its HEAD detached. Commits on its branches are kept; Git refuses if uncommitted changes would be overwritten.")
        }
    }

    private func open(_ submodule: GitSubmodule) {
        guard submodule.state != .uninitialized, !model.isLoading, model.confirmDiscardFileEdits() else { return }
        model.loadRepository(at: URL(fileURLWithPath: snapshot.rootPath).appendingPathComponent(submodule.path))
    }

    private func summary(_ submodule: GitSubmodule) -> String {
        let state = switch submodule.state {
        case .uninitialized: "Not checked out"
        case .upToDate: "At recorded commit"
        case let .differentCommit(head): "At \(head.prefix(7)), recorded \(submodule.recordedCommit.prefix(7))"
        }
        return state + (submodule.hasLocalChanges ? " · uncommitted changes" : "")
    }

    private func icon(_ submodule: GitSubmodule) -> String {
        switch submodule.state {
        case .uninitialized: "shippingbox"
        case .upToDate: submodule.hasLocalChanges ? "shippingbox.circle" : "shippingbox.fill"
        case .differentCommit: "exclamationmark.triangle"
        }
    }
}

/// Confirms merging or rebasing after a branch is dropped onto the current branch, using the
/// branch and HEAD captured at the drop.
private struct BranchDropConfirmation: ViewModifier {
    @Binding var drop: (branch: GitBranch, currentBranch: String, head: String?)?
    @EnvironmentObject private var model: AppModel

    func body(content: Content) -> some View {
        content.confirmationDialog("Combine \(drop?.branch.displayName ?? "") with \(drop?.currentBranch ?? "")?",
                                   isPresented: Binding(get: { drop != nil }, set: { if !$0 { drop = nil } })) {
            if let drop {
                Button("Merge \(drop.branch.displayName) into \(drop.currentBranch)") {
                    model.start(.merge, target: drop.branch.tip, expectedHead: drop.head, expectedBranch: drop.currentBranch, expectedSourceBranch: drop.branch)
                }
                Button("Rebase \(drop.currentBranch) onto \(drop.branch.displayName)") {
                    model.start(.rebase, target: drop.branch.tip, expectedHead: drop.head, expectedBranch: drop.currentBranch, expectedSourceBranch: drop.branch)
                }
            }
        } message: {
            if let drop {
                Text("Merge adds \(drop.branch.displayName) at \(drop.branch.tip.prefix(8)) to \(drop.currentBranch), with a merge commit when needed. Rebase replays \(drop.currentBranch)'s commits on top of it and rewrites their IDs. Either may stop for conflicts.")
            }
        }
    }
}

/// Identifies a dragged branch unambiguously; local and remote names can look alike.
func branchDragIdentity(_ branch: GitBranch) -> String {
    "nicegit-branch\0" + (branch.isRemote ? "remote" : "local") + "\0" + branch.name
}

