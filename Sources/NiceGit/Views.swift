import NiceGitCore
import SwiftUI
private struct RepositorySidebar: View {
    @EnvironmentObject private var model: AppModel
    @State private var newBranchName = ""
    @State private var showingNewBranch = false
    @State private var branchSource: GitBranch?
    @State private var branchToRename: GitBranch?
    @State private var branchToDelete: GitBranch?
    @State private var pushRequest: (branch: GitBranch, remote: String)?
    @State private var integrationRequest: (operation: GitOperation, branch: GitBranch, currentBranch: String, head: String?)?
    @State private var renamedBranchName = ""
    @State private var tagToDelete: (name: String, tip: String)?
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
                    Button { branchSource = nil; showingNewBranch = true } label: { Image(systemName: "plus") }
                        .buttonStyle(.plain).help("Create branch")
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
                                    ForEach(branches.filter { $0.name.hasPrefix("remotes/" + remote + "/") }) { branch in
                                        branchRow(branch, snapshot: snapshot)
                                    }
                                }.padding(.horizontal, 12).padding(.vertical, 4)
                            }
                            if snapshot.remotes.isEmpty { empty("No remotes configured") }
                        }
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
                                }
                            }
                        }
                        SidebarSection(title: "Tags", icon: "tag", count: snapshot.tags.count) {
                            let tags = snapshot.tags.filter { matches($0) }
                            if tags.isEmpty { empty("No tags") }
                            ForEach(tags, id: \.self) { tag in
                                SidebarButton(title: tag, subtitle: "", systemImage: "tag", isSelected: false) { model.inspectTag(tag) }
                                    .contextMenu {
                                        Button("Delete local tag...", role: .destructive) {
                                            if let tip = snapshot.tagTips[tag] { tagToDelete = (tag, tip) }
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
                    model.start(request.operation, target: request.branch.tip, expectedHead: request.head, expectedBranch: request.currentBranch)
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
                } else {
                    model.createBranch(named: newBranchName) { newBranchName = "" }
                }
            }
                .disabled(newBranchName.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
        }


        .confirmationDialog("Delete local tag \(tagToDelete?.name ?? "")?", isPresented: Binding(get: { tagToDelete != nil }, set: { if !$0 { tagToDelete = nil } })) {
            if let tagToDelete {
                Button("Delete local tag", role: .destructive) { model.deleteTag(name: tagToDelete.name, expectedTip: tagToDelete.tip) }
            }
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
        SidebarButton(title: branch.displayName, subtitle: "", systemImage: branch.isCurrent ? "checkmark.circle.fill" : "arrow.triangle.branch", isSelected: branch.isCurrent) {
            model.checkout(branch: branch)
        }.contextMenu {
            Button(branch.isRemote ? "Checkout tracking branch" : "Checkout branch") { model.checkout(branch: branch) }
                .disabled(branch.isCurrent)
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
            selectedCommit = nil
            selectedStash = nil
            if model.showingTerminal { model.openTerminal() }
        }
        .onChange(of: snapshot.commits) { _, commits in
            if let selected = selectedCommit {
                selectedCommit = commits.first { $0.hash == selected.hash }
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

                Button {
                    model.commit(message: commitMessage) { commitMessage = "" }
                } label: {
                    Label(summary.wrappedValue.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty ? "Type a Message to Commit" : "Commit Changes", systemImage: "checkmark.circle")
                        .frame(maxWidth: .infinity).padding(.vertical, 7)
                }
                .buttonStyle(.borderedProminent).tint(AppPalette.signal)
                .disabled(staged.isEmpty || summary.wrappedValue.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
            }
            .padding(14)
            .background(AppPalette.toolbar)
        }
        .background(AppPalette.panel)
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
            .disabled(entry.kind == .untracked || primaryHelp == "Unstage file")
            .help("Discard local change")
        }
        .padding(.horizontal, 6).padding(.vertical, 7)
        .frame(minHeight: 34)
        .confirmationDialog("Discard changes to \(entry.fileName)?", isPresented: $confirmingDiscard) {
            Button("Discard changes", role: .destructive, action: discard)
        } message: {
            Text("Unstaged changes to this file will be lost.")
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

    static let laneColors: [Color] = [
        signal,
        Color(red: 0.157, green: 0.478, blue: 0.812),
        merge,
        conflict
    ]
}
