import NiceGitCore
import SwiftUI

struct GraphWorkspace: View {
    let snapshot: RepositorySnapshot
    @Binding var selectedCommit: GitCommit?
    @Binding var selectedStash: GitStash?
    @EnvironmentObject private var model: AppModel
    @State private var query = ""
    @State private var hoveredCommitHash: String?
    @State private var revertRequest: (commit: GitCommit, branch: String, head: String?)?
    @State private var cherryPickRequest: (commit: GitCommit, branch: String, head: String?)?
    @State private var resetRequest: ResetRequest?
    @State private var searchingHistory = false
    /// Observed so the graph redraws when the palette changes in Settings.
    @AppStorage(LanePalette.storageKey) private var lanePalette = LanePalette.standard
    /// Commits gathered with Command-click for actions on several commits at once.
    @State private var multiSelection: Set<String> = []
    @State private var multiPickRequest: (hashes: [String], branch: String, head: String?)?

    private func moveSelection(by offset: Int, proxy: ScrollViewProxy) -> KeyPress.Result {
        let listed = snapshot.commits.filter(matches)
        guard !listed.isEmpty else { return .ignored }
        let current = selectedCommit.flatMap { selected in listed.firstIndex { $0.hash == selected.hash } }
        let next = current.map { min(max($0 + offset, 0), listed.count - 1) } ?? (offset > 0 ? 0 : listed.count - 1)
        multiSelection = []
        selectedStash = nil
        selectedCommit = listed[next]
        // Scroll vertically only; centring the row sideways would hide the branch column.
        proxy.scrollTo(listed[next].hash, anchor: UnitPoint(x: 0, y: 0.5))
        return .handled
    }

    /// Command-click adds or removes a commit from the multi-selection; a plain click selects one.
    private func select(_ commit: GitCommit) {
        if NSEvent.modifierFlags.contains(.command) {
            if multiSelection.isEmpty, let current = selectedCommit { multiSelection.insert(current.hash) }
            if multiSelection.contains(commit.hash) { multiSelection.remove(commit.hash) } else { multiSelection.insert(commit.hash) }
        } else {
            multiSelection = []
        }
        selectedCommit = commit
        selectedStash = nil
    }
    private let rowHeight: CGFloat = 36
    /// The branch/tag column narrows in smaller windows so commit messages keep room.
    private static func referenceWidth(for width: CGFloat) -> CGFloat { min(212, max(120, width * 0.2)) }

    private func matches(_ commit: GitCommit) -> Bool {
        query.isEmpty || [commit.subject, commit.hash, commit.authorName, commit.refs.joined(separator: " ")]
            .contains { $0.localizedCaseInsensitiveContains(query) }
    }

    var body: some View {
        let _ = lanePalette
        let hasChanges = !snapshot.status.isEmpty
        let rows = hasChanges
            ? GitGraph.layoutWithWorkingTree(snapshot.commits, headHash: snapshot.headHash, colorCount: AppPalette.laneColors.count)
            : GitGraph.layout(snapshot.commits, pinning: snapshot.headHash, colorCount: AppPalette.laneColors.count)
        // Emphasise the selected commit's line; with nothing selected every line stays at full strength.
        let focusLine = selectedCommit.flatMap { selected in
            snapshot.commits.firstIndex(where: { $0.hash == selected.hash }).map { rows[$0 + (hasChanges ? 1 : 0)].line }
        }
        let railWidth = GraphRail.width(lanes: rows.map(\.laneCount).max() ?? 1)
        let laneColor = { (row: GitGraphRow) in AppPalette.laneColors[row.color % AppPalette.laneColors.count] }
        VStack(spacing: 0) {
            HStack(spacing: 12) {
                Text("Commit history").font(.system(size: 14, weight: .semibold))
                Spacer()
                Text("\(snapshot.commits.count) commits").font(.caption.monospaced()).foregroundStyle(.secondary)
            }.padding(.horizontal, 16).padding(.top, 14).padding(.bottom, 10)
            HStack(spacing: 8) {
                TextField("Filter loaded commits", text: $query)
                    .textFieldStyle(.roundedBorder)
                Button { searchingHistory = true } label: { Label("Search all history", systemImage: "magnifyingglass") }
                    .help("Search every branch by message, author, or code change (⇧⌘F)")
                    .keyboardShortcut("f", modifiers: [.command, .shift])
            }.padding(.horizontal, 16).padding(.bottom, 12)
            GeometryReader { geometry in
                let referenceWidth = Self.referenceWidth(for: geometry.size.width)
                let messageWidth = max(240, geometry.size.width - referenceWidth - railWidth)
                // In tight rows the date alone is clearer than a name and date both cut short,
                // and the hash stays available in the inspector.
                let showsAuthor = messageWidth >= 560
                let showsHash = messageWidth >= 400
                ScrollViewReader { proxy in
                ScrollView([.horizontal, .vertical]) {
                    VStack(alignment: .leading, spacing: 0) {
                        HStack(spacing: 0) {
                            Text("Branch / tag").frame(width: referenceWidth - 12, alignment: .leading)
                            Text("Graph").frame(width: railWidth, alignment: .leading)
                            Text(query.isEmpty ? "Commit message" : "Commit message · connections hidden while filtering")
                                .frame(width: messageWidth, alignment: .leading)
                        }
                        .font(.system(size: 11, weight: .medium))
                        .foregroundStyle(.secondary).padding(.leading, 12).frame(height: 28)
                        .background(AppPalette.toolbar)

                        ForEach(snapshot.stashes.filter { query.isEmpty || $0.message.localizedCaseInsensitiveContains(query) || $0.reference.localizedCaseInsensitiveContains(query) }) { stash in
                            Button {
                                selectedCommit = nil
                                selectedStash = stash
                            } label: {
                                HStack(spacing: 0) {
                                    Text(stash.reference).font(.system(size: 11, weight: .semibold, design: .monospaced))
                                        .foregroundStyle(.orange).lineLimit(1)
                                        .frame(width: referenceWidth - 12, alignment: .leading)
                                    Image(systemName: "archivebox.fill")
                                        .font(.system(size: 13)).foregroundStyle(.orange)
                                        .frame(width: GraphRail.nodeX(lane: 0) * 2)
                                        .frame(width: railWidth, alignment: .leading)
                                    GraphMessage(accent: .orange, tint: selectedStash?.hash == stash.hash ? 0.28 : 0.08) {
                                        Text(stash.message).font(.system(size: 13)).lineLimit(1)
                                        Spacer(minLength: 8)
                                        Text("Saved changes").font(.system(size: 11)).foregroundStyle(.secondary)
                                    }.frame(width: messageWidth, height: rowHeight)
                                }.padding(.leading, 12).frame(height: rowHeight).contentShape(Rectangle())
                            }.buttonStyle(.plain).help("Inspect \(stash.reference): \(stash.message)")
                        }

                        if hasChanges && query.isEmpty {
                            let isSelected = selectedCommit == nil && selectedStash == nil
                            Button { selectedCommit = nil; selectedStash = nil } label: {
                                HStack(spacing: 0) {
                                    HStack(spacing: 5) {
                                        Image(systemName: "pencil")
                                        Text("Working tree").font(.system(size: 12, weight: .semibold))
                                    }.foregroundStyle(laneColor(rows[0]))
                                        .frame(width: referenceWidth - 12, alignment: .leading)
                                    GraphRail(row: rows[0], workingTree: true, connected: true, focusLine: focusLine,
                                              tint: isSelected ? 0.28 : 0.10)
                                        .frame(width: railWidth, height: rowHeight)
                                    GraphMessage(accent: laneColor(rows[0]), tint: isSelected ? 0.28 : 0.10) {
                                        Text("\(snapshot.status.count) uncommitted \(snapshot.status.count == 1 ? "file" : "files")")
                                            .font(.system(size: 13, weight: .semibold)).lineLimit(1)
                                        Spacer(minLength: 8)
                                        Text("Not committed yet").font(.system(size: 11)).foregroundStyle(.secondary)
                                    }.frame(width: messageWidth, height: rowHeight)
                                }.padding(.leading, 12).frame(height: rowHeight).contentShape(Rectangle())
                            }.buttonStyle(.plain).help("Show working-tree files and staging")
                        }

                        LazyVStack(spacing: 0) {
                            ForEach(Array(snapshot.commits.enumerated()), id: \.element.hash) { index, commit in
                                if matches(commit) {
                                    let row = rows[index + (hasChanges ? 1 : 0)]
                                    let isSelected = selectedCommit?.hash == commit.hash || multiSelection.contains(commit.hash)
                                    let isHead = commit.hash == snapshot.headHash
                                    HStack(spacing: 0) {
                                        // Pills sit in their own column; a thin line in the lane colour
                                        // leads from the pill to the commit it names.
                                        HStack(spacing: 0) {
                                            if !commit.refs.isEmpty {
                                                CommitReferences(refs: commit.refs, snapshot: snapshot, color: laneColor(row),
                                                    showingReferences: Binding(
                                                        get: { hoveredCommitHash == commit.hash },
                                                        set: { visible in
                                                            if visible { hoveredCommitHash = commit.hash }
                                                            else if hoveredCommitHash == commit.hash { hoveredCommitHash = nil }
                                                        }
                                                    ))
                                                    .layoutPriority(1)
                                                laneColor(row).opacity(0.6).frame(height: 1)
                                            }
                                        }.frame(width: referenceWidth - 12, alignment: .leading)
                                        Button { select(commit) } label: {
                                            GraphRail(row: row, workingTree: false, connected: query.isEmpty,
                                                      focusLine: focusLine, tint: isSelected ? 0.28 : 0.10,
                                                      initials: GraphRail.initials(commit.authorName),
                                                      showsConnector: !commit.refs.isEmpty,
                                                      isMerge: commit.parents.count > 1, isHead: isHead)
                                                .frame(width: railWidth, height: rowHeight)
                                                .contentShape(Rectangle())
                                        }.buttonStyle(.plain).accessibilityHidden(true)
                                        Button { select(commit) } label: {
                                            GraphMessage(accent: laneColor(row), tint: isSelected ? 0.28 : 0.10) {
                                                Text(commit.subject).font(.system(size: 13, weight: isHead ? .semibold : .regular)).lineLimit(1)
                                                if isHead {
                                                    Text("You are here")
                                                        .font(.system(size: 10, weight: .semibold)).fixedSize()
                                                        .foregroundStyle(AppPalette.signal)
                                                }
                                                Spacer(minLength: 8)
                                                Text(showsAuthor ? "\(commit.authorName) · \(commit.relativeDate)" : commit.relativeDate)
                                                    .font(.system(size: 11)).foregroundStyle(.secondary)
                                                    .lineLimit(1).truncationMode(.middle)
                                                    .frame(maxWidth: 220, alignment: .trailing)
                                                    .help("\(commit.authorName) · \(commit.commitDate.map { $0.formatted(date: .complete, time: .shortened) } ?? commit.relativeDate)")
                                                if showsHash {
                                                    Text(commit.shortHash).font(.system(size: 10, design: .monospaced))
                                                        .foregroundStyle(.secondary).fixedSize()
                                                }
                                            }.frame(width: messageWidth, height: rowHeight).contentShape(Rectangle())
                                        }.buttonStyle(.plain)
                                            .help(commit.subject)
                                            .accessibilityLabel("\(commit.parents.count > 1 ? "Merge commit" : "Commit"): \(commit.subject)\(isHead ? ", current checkout" : "")")
                                    }
                                    .padding(.leading, 12).frame(height: rowHeight)
                                    .background(index.isMultiple(of: 2) ? Color.clear : AppPalette.rowStripe)
                                    .contextMenu {
                                        Button("View patch") { model.inspect(commit) }
                                        Button("Create patch from commit...") { model.exportPatch(commit) }
                                            .disabled(commit.parents.count > 1)
                                        if multiSelection.count > 1 && multiSelection.contains(commit.hash) {
                                            Button("Cherry-pick \(multiSelection.count) selected commits...") {
                                                multiPickRequest = (Array(multiSelection), snapshot.currentBranch, snapshot.headHash)
                                            }.disabled(snapshot.operation != nil || !snapshot.status.isEmpty)
                                        }
                                        Button("Cherry-pick commit...") {
                                            cherryPickRequest = (commit, snapshot.currentBranch, snapshot.headHash)
                                        }.disabled(snapshot.operation != nil)
                                        Menu("Compare") {
                                            Button("With working files") {
                                                model.compareRequest = CompareRequest(repositoryURL: URL(fileURLWithPath: snapshot.rootPath), older: commit, newer: nil)
                                            }
                                            if let marked = model.compareMark, marked.hash != commit.hash {
                                                Button("With marked commit \(marked.shortHash)") {
                                                    // Older side first, judged by commit time; equal times keep the marked one first.
                                                    let markedIsOlder = (marked.commitDate ?? .distantPast) <= (commit.commitDate ?? .distantPast)
                                                    model.compareRequest = CompareRequest(repositoryURL: URL(fileURLWithPath: snapshot.rootPath),
                                                                                          older: markedIsOlder ? marked : commit, newer: markedIsOlder ? commit : marked)
                                                }
                                            }
                                            Divider()
                                            Button(model.compareMark?.hash == commit.hash ? "Clear comparison mark" : "Mark for comparison") {
                                                model.compareMark = model.compareMark?.hash == commit.hash ? nil : commit
                                            }
                                        }
                                        Button("Interactive rebase from this commit...") {
                                            if let head = snapshot.headHash {
                                                model.rebaseRequest = InteractiveRebaseRequest(oldest: commit.hash, branch: snapshot.currentBranch,
                                                                                               head: head, repositoryURL: URL(fileURLWithPath: snapshot.rootPath))
                                            }
                                        }.disabled(snapshot.operation != nil || snapshot.headHash == nil || snapshot.currentBranch.hasPrefix("Detached HEAD"))
                                        Button("Revert commit...") {
                                            revertRequest = (commit, snapshot.currentBranch, snapshot.headHash)
                                        }
                                        Button("Create tag...") { model.taggingCommit = commit }
                                        Menu("Reset \(snapshot.currentBranch) to this commit") {
                                            ForEach(GitResetMode.allCases, id: \.self) { mode in
                                                Button(mode.title + "...") {
                                                    if let head = snapshot.headHash {
                                                        resetRequest = ResetRequest(target: commit.hash, mode: mode, branch: snapshot.currentBranch, head: head)
                                                    }
                                                }
                                            }
                                        }.disabled(snapshot.operation != nil || snapshot.headHash == nil)
                                        Button("Edit commit message...") { model.editingCommitMessage = commit }
                                            .disabled(commit.hash != snapshot.headHash || snapshot.operation != nil)
                                    }
                                }
                            }
                        }
                        if snapshot.hasMoreCommits {
                            Button("Load older commits") { model.loadOlderCommits() }.padding(16)
                        }
                    }
                    // Rows are the reference column (including its leading padding), the rail and the message.
                    .frame(width: referenceWidth + railWidth + messageWidth, alignment: .leading)
                    .frame(minHeight: geometry.size.height, alignment: .topLeading)
                }
                // Arrow keys move through the listed commits; Escape clears the selection.
                .focusable()
                .focusEffectDisabled()
                .onKeyPress(.downArrow) { moveSelection(by: 1, proxy: proxy) }
                .onKeyPress(.upArrow) { moveSelection(by: -1, proxy: proxy) }
                .onKeyPress(.escape) {
                    guard selectedCommit != nil || !multiSelection.isEmpty else { return .ignored }
                    selectedCommit = nil
                    multiSelection = []
                    return .handled
                }
                }
            }
            HStack {
                Image(systemName: "arrow.triangle.branch")
                Text("Current checkout: \(snapshot.currentBranch)").lineLimit(1)
                Spacer()
                Text(snapshot.headHash.map { String($0.prefix(7)) } ?? "No commits")
            }.font(.system(size: 11, design: .monospaced)).foregroundStyle(.secondary)
                .padding(10).background(AppPalette.toolbar)
        }.background(AppPalette.canvas)
        .onChange(of: snapshot.commits) { hoveredCommitHash = nil }
        .onChange(of: query) { hoveredCommitHash = nil }
        .modifier(ResetConfirmation(request: $resetRequest))
        .onChange(of: snapshot.commits) { _, commits in
            let loaded = Set(commits.map(\.hash))
            multiSelection.formIntersection(loaded)
        }
        .confirmationDialog("Cherry-pick \(multiPickRequest?.hashes.count ?? 0) commits onto \(multiPickRequest?.branch ?? "")?",
                            isPresented: Binding(get: { multiPickRequest != nil }, set: { if !$0 { multiPickRequest = nil } })) {
            if let request = multiPickRequest {
                Button("Cherry-pick commits") {
                    model.cherryPick(request.hashes, expectedHead: request.head, expectedBranch: request.branch)
                    multiSelection = []
                }
            }
        } message: {
            Text("Copies each selected commit onto the current branch as a new commit, oldest first. If one conflicts, NiceGit stops so you can resolve it and continue, or abort to return to where you started. Merge commits must be picked on their own.")
        }
        .sheet(isPresented: $searchingHistory) {
            CommitSearchView(repositoryURL: URL(fileURLWithPath: snapshot.rootPath), query: query) { commit in
                selectedStash = nil
                selectedCommit = commit
            }
        }
        .confirmationDialog("Cherry-pick \(cherryPickRequest?.commit.shortHash ?? "") onto \(cherryPickRequest?.branch ?? "")?", isPresented: Binding(get: { cherryPickRequest != nil }, set: { if !$0 { cherryPickRequest = nil } })) {
            if let request = cherryPickRequest {
                if request.commit.parents.count > 1 {
                    ForEach(Array(request.commit.parents.enumerated()), id: \.offset) { index, parent in
                        Button("Apply relative to parent \(index + 1) (\(parent.prefix(8)))") {
                            model.start(.cherryPick, target: request.commit.hash, mainline: index + 1, expectedHead: request.head, expectedBranch: request.branch)
                        }
                    }
                } else {
                    Button("Cherry-pick commit") {
                        model.start(.cherryPick, target: request.commit.hash, expectedHead: request.head, expectedBranch: request.branch)
                    }
                }
            }
        } message: {
            Text("Copies this change into a new commit on the current branch. Conflicts may need resolving. For a merge, choose the parent to use as the baseline for the copied changes.")
        }
        .confirmationDialog("Revert \(revertRequest?.commit.shortHash ?? "") on \(revertRequest?.branch ?? "")?", isPresented: Binding(get: { revertRequest != nil }, set: { if !$0 { revertRequest = nil } })) {
            if let request = revertRequest {
                if request.commit.parents.count > 1 {
                    ForEach(Array(request.commit.parents.enumerated()), id: \.offset) { index, parent in
                        Button("Revert relative to parent \(index + 1) (\(parent.prefix(8)))") {
                            model.start(.revert, target: request.commit.hash, mainline: index + 1,
                                        expectedHead: request.head, expectedBranch: request.branch)
                        }
                    }
                } else {
                    Button("Create revert commit") {
                        model.start(.revert, target: request.commit.hash,
                                    expectedHead: request.head, expectedBranch: request.branch)
                    }
                }
            }
        } message: {
            Text("Creates a new commit reversing this change. Original history is retained. For a merge, select the parent whose side should be kept.")
        }
    }
}

private struct CommitReferences: View {
    let refs: [String]
    let snapshot: RepositorySnapshot
    let color: Color
    @EnvironmentObject private var model: AppModel
    @Environment(\.colorScheme) private var colorScheme
    @Binding var showingReferences: Bool
    @State private var isBadgeHovered = false
    @State private var hoveredReference: String?

    private func pairedRemote(_ local: GitBranch) -> GitBranch? {
        guard !local.isRemote else { return nil }
        return refs.compactMap { branch($0) }.first {
            $0.isRemote && ("refs/" + $0.name == local.upstream || $0.displayName == "origin/\(local.name)") && $0.tip == local.tip
        }
    }

    private var ordered: [String] {
        let pairedNames = Set(refs.compactMap { branch($0) }.compactMap { pairedRemote($0)?.name })
        return refs.filter { ref in
            if branch(ref) == nil && snapshot.remotes.contains(where: { ref == "\($0)/HEAD" || ref == "remotes/\($0)/HEAD" }) { return false }
            if ref == "HEAD" { return !snapshot.branches.contains(where: { $0.isCurrent }) }
            guard let branch = branch(ref) else { return true }
            return !pairedNames.contains(branch.name)
        }.sorted { left, right in
            let leftHead = left == "HEAD" || left.hasPrefix("HEAD -> ")
            let rightHead = right == "HEAD" || right.hasPrefix("HEAD -> ")
            if leftHead != rightHead { return leftHead }
            return left.localizedStandardCompare(right) == .orderedAscending
        }
    }

    private func branch(_ ref: String) -> GitBranch? {
        guard !ref.hasPrefix("tag: "), ref != "HEAD" else { return nil }
        let name = ref.hasPrefix("HEAD -> ") ? String(ref.dropFirst(8)) : ref
        return snapshot.branches.first { !$0.isRemote && $0.name == name }
            ?? snapshot.branches.first { $0.isRemote && ($0.name == name || $0.displayName == name) }
    }

    private func isGitHub(_ branch: GitBranch) -> Bool {
        guard branch.isRemote,
              let remote = snapshot.remotes.sorted(by: { $0.count > $1.count }).first(where: { branch.displayName.hasPrefix($0 + "/") }),
              let address = snapshot.remoteAddresses[remote] else { return false }
        return (try? GitHubRepository(remoteAddress: address)) != nil
    }

    private func checkout(_ ref: String) {
        guard let branch = branch(ref), !branch.isCurrent,
              snapshot.operation == nil, !model.isLoading,
              model.confirmDiscardFileEdits() else { return }
        showingReferences = false
        model.checkout(branch: branch)
    }

    @ViewBuilder
    private func branchIcon(_ branch: GitBranch) -> some View {
        if isGitHub(branch), let url = Bundle.module.url(forResource: colorScheme == .dark ? "GitHub_Invertocat_White" : "GitHub_Invertocat_Black", withExtension: "png"),
           let image = NSImage(contentsOf: url) {
            Image(nsImage: image)
                .resizable().scaledToFit().frame(width: 16, height: 16)
                .accessibilityLabel("GitHub remote branch")
        } else {
            Image(systemName: branch.isRemote ? "network" : "laptopcomputer")
                .frame(width: 16, height: 16)
                .accessibilityLabel(branch.isRemote ? "Remote branch" : "Local branch")
        }
    }

    private func displayName(_ ref: String) -> String {
        if ref == "HEAD" { return "Detached HEAD \(snapshot.headHash.map { String($0.prefix(7)) } ?? "")" }
        guard let branch = branch(ref) else { return ref }
        guard branch.isRemote else { return branch.name }
        guard let remote = snapshot.remotes.sorted(by: { $0.count > $1.count }).first(where: {
            branch.displayName.hasPrefix($0 + "/")
        }) else { return branch.displayName }
        return String(branch.displayName.dropFirst(remote.count + 1))
    }

    private func label(_ ref: String) -> some View {
        HStack(spacing: 5) {
            if ref == "HEAD" || branch(ref)?.isCurrent == true { Image(systemName: "checkmark") }
            Text(displayName(ref))
                .lineLimit(1).truncationMode(.middle)
            if let branch = branch(ref) {
                branchIcon(branch)
                if let remote = pairedRemote(branch) {
                    branchIcon(remote).help(remote.displayName)
                }
            } else { Image(systemName: ref.hasPrefix("tag: ") ? "tag" : "arrow.triangle.branch") }
        }.font(.system(size: 12, weight: .medium))
    }

    var body: some View {
        if let first = ordered.first {
            HStack(spacing: 8) {
                label(first)
                    .padding(.horizontal, 7).frame(height: 24)
                    .background(color.opacity(isBadgeHovered ? 0.42 : 0.30), in: RoundedRectangle(cornerRadius: 3))
                if ordered.count > 1 {
                    Text("+\(ordered.count - 1)").font(.system(size: 10, weight: .semibold)).fixedSize()
                        .padding(.horizontal, 6).frame(height: 24)
                        .background(color.opacity(isBadgeHovered ? 0.34 : 0.22), in: RoundedRectangle(cornerRadius: 3))
                }
            }
            .padding(.trailing, 8)
            .help(ordered.map { displayName($0) }.joined(separator: ", "))
            .contentShape(Rectangle())
            .onHover {
                isBadgeHovered = $0
                // Open the reference list on click so tracing a line cannot obscure the graph.
            }
            .onDisappear {
                isBadgeHovered = false
                hoveredReference = nil
            }
            .onChange(of: showingReferences) {
                if !showingReferences { hoveredReference = nil }
            }
            .gesture(TapGesture(count: 2).exclusively(before: TapGesture()).onEnded { gesture in
                switch gesture {
                case .first: checkout(first)
                case .second: if ordered.count > 1 { showingReferences = true }
                }
            })
            .accessibilityAction(named: "Switch to branch") { checkout(first) }
            // Drag a branch label onto the current branch in the sidebar to merge or rebase.
            .draggable(branch(first).map(branchDragIdentity) ?? "")
            .accessibilityLabel(ordered.map { displayName($0) }.joined(separator: ", "))
            .popover(isPresented: $showingReferences, arrowEdge: .bottom) {
                ScrollView {
                    VStack(alignment: .leading, spacing: 0) {
                        ForEach(ordered, id: \.self) { ref in
                            let isCurrent = branch(ref)?.isCurrent == true
                            label(ref).padding(9).frame(maxWidth: .infinity, alignment: .leading)
                                .background(color.opacity(isCurrent ? (hoveredReference == ref ? 0.46 : 0.36) : (hoveredReference == ref ? 0.24 : 0.12)))
                                .overlay(alignment: .leading) {
                                    if isCurrent { color.frame(width: 3).allowsHitTesting(false) }
                                }
                                .accessibilityAddTraits(isCurrent ? .isSelected : [])
                                .contentShape(Rectangle())
                                .onHover { inside in
                                    if inside { hoveredReference = ref }
                                    else if hoveredReference == ref { hoveredReference = nil }
                                }
                                .onTapGesture(count: 2) { checkout(ref) }
                                .accessibilityAction(named: "Switch to branch") { checkout(ref) }
                                .help(ref + (branch(ref).map { $0.isRemote ? " (remote)" : " (local)" } ?? ""))
                        }
                    }
                }.frame(width: 340, height: min(CGFloat(ordered.count) * 36, 300))
            }
        } else {
            Color.clear.frame(height: 24)
                .accessibilityHidden(true)
        }
    }
}

/// The message column of a graph row: a lane-coloured accent bar over a tint that
/// continues the row's tint from the graph.
private struct GraphMessage<Content: View>: View {
    let accent: Color
    let tint: Double
    @ViewBuilder let content: Content

    var body: some View {
        HStack(spacing: 8) {
            accent.frame(width: 3)
            content
        }
        .padding(.trailing, 12)
        .background(accent.opacity(tint))
    }
}

private struct GraphRail: View {
    let row: GitGraphRow
    let workingTree: Bool
    let connected: Bool
    var focusLine: Int?
    /// Opacity of the lane-coloured band from the node to the message column.
    var tint: Double = 0
    var initials = ""
    var showsConnector = false
    var isMerge = false
    var isHead = false

    static let laneSpacing: CGFloat = 24
    static let nodeRadius: CGFloat = 10
    static func nodeX(lane: Int) -> CGFloat { 14 + CGFloat(lane) * laneSpacing }
    static func width(lanes: Int) -> CGFloat { max(56, nodeX(lane: max(lanes, 1) - 1) + 20) }

    static func initials(_ name: String) -> String {
        let words = name.split(whereSeparator: { $0.isWhitespace || $0 == "." || $0 == "-" || $0 == "_" })
        return String((words.count > 1 ? [words[0], words[words.count - 1]] : Array(words.prefix(1))).compactMap(\.first)).uppercased()
    }

    /// Lines away from the focused one fade so the selected path reads first.
    private func opacity(_ matches: Bool) -> Double { focusLine == nil || matches ? 1 : 0.3 }
    private func isFocused(_ edge: GitGraphSegment) -> Bool { edge.line == focusLine || edge.fromLine == focusLine }

    /// Lane changes run as right angles with rounded corners. A new branch leaves its
    /// node sideways before turning down; a line joining another lane runs down first.
    private static func connection(from start: CGPoint, to end: CGPoint, turnAt turn: CGFloat) -> Path {
        var path = Path()
        path.move(to: start)
        guard abs(end.x - start.x) > 0.5 else {
            path.addLine(to: end)
            return path
        }
        let direction: CGFloat = end.x > start.x ? 1 : -1
        let before = turn - start.y, after = end.y - turn
        let radius = min(6, abs(end.x - start.x) / 2)
        if before > 0.5 {
            let r = min(radius, before)
            path.addLine(to: CGPoint(x: start.x, y: turn - r))
            path.addQuadCurve(to: CGPoint(x: start.x + direction * r, y: turn), control: CGPoint(x: start.x, y: turn))
        }
        if after > 0.5 {
            let r = min(radius, after)
            path.addLine(to: CGPoint(x: end.x - direction * r, y: turn))
            path.addQuadCurve(to: CGPoint(x: end.x, y: turn + r), control: CGPoint(x: end.x, y: turn))
        }
        path.addLine(to: end)
        return path
    }

    var body: some View {
        Canvas { context, size in
            let x = Self.nodeX(lane: row.lane)
            let y = size.height / 2
            let nodeColor = AppPalette.laneColors[row.color % AppPalette.laneColors.count]
            let tintRect = CGRect(x: x, y: 0, width: max(0, size.width - x), height: size.height)
            if tint > 0 {
                context.fill(Path(tintRect), with: .color(nodeColor.opacity(tint)))
            }
            if showsConnector {
                var connector = Path()
                connector.move(to: CGPoint(x: 0, y: y))
                connector.addLine(to: CGPoint(x: x, y: y))
                context.stroke(connector, with: .color(nodeColor.opacity(0.6)), lineWidth: 1)
            }
            if connected {
                // Draw straight continuations first; a background stroke gives crossing
                // connections a small bridge so crossings cannot look like junctions.
                // Focused lines go last so a faded crossing never cuts through them.
                let order = { (edge: GitGraphSegment) in (isFocused(edge) ? 1 : 0, edge.startsAtNode ? 1 : 0) }
                for edge in row.segments.sorted(by: { order($0) < order($1) }) {
                    let start = CGPoint(x: Self.nodeX(lane: edge.fromLane), y: edge.startsAtNode ? y : 0)
                    let end = CGPoint(x: Self.nodeX(lane: edge.toLane), y: edge.endsAtNode ? y : size.height)
                    let turn = edge.startsAtNode && edge.toLane > edge.fromLane ? start.y
                        : edge.endsAtNode ? end.y : (start.y + end.y) / 2
                    let path = Self.connection(from: start, to: end, turnAt: turn)
                    context.stroke(path, with: .color(AppPalette.canvas), lineWidth: 6)
                    if tint > 0 {
                        // Keep the bridge inside the tinted band the same colour as the band.
                        var tinted = context
                        tinted.clip(to: Path(tintRect))
                        tinted.stroke(path, with: .color(nodeColor.opacity(tint)), lineWidth: 6)
                    }
                    let focused = isFocused(edge)
                    context.stroke(path, with: .color(AppPalette.laneColors[edge.color % AppPalette.laneColors.count].opacity(opacity(focused))),
                                   style: StrokeStyle(lineWidth: focused && focusLine != nil ? 2.5 : 2, lineCap: .round, lineJoin: .round, dash: workingTree ? [3, 3] : []))
                }
            }
            let color = nodeColor.opacity(opacity(row.line == focusLine))
            // Merges are small dots so the commits that carry work stand out.
            let radius = isMerge ? 5 : Self.nodeRadius
            let node = Path(ellipseIn: CGRect(x: x - radius, y: y - radius, width: radius * 2, height: radius * 2))
            context.stroke(node, with: .color(AppPalette.canvas), lineWidth: 4)
            if workingTree {
                context.fill(node, with: .color(AppPalette.canvas))
                context.stroke(node, with: .color(color), style: StrokeStyle(lineWidth: 2, dash: [3, 2]))
            } else {
                // A solid base keeps lines from showing through a faded node.
                context.fill(node, with: .color(AppPalette.canvas))
                context.fill(node, with: .color(color))
                if !isMerge && !initials.isEmpty {
                    context.draw(Text(initials).font(.system(size: 8, weight: .bold)).foregroundStyle(.white.opacity(opacity(row.line == focusLine))),
                                 at: CGPoint(x: x, y: y))
                }
            }
            if workingTree || isHead {
                context.stroke(Path(ellipseIn: CGRect(x: x - radius - 3, y: y - radius - 3, width: (radius + 3) * 2, height: (radius + 3) * 2)),
                               with: .color(color.opacity(0.6)), lineWidth: 1.5)
            }
        }
    }
}
