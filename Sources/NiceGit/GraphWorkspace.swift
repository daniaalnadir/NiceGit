import NiceGitCore
import SwiftUI

struct GraphWorkspace: View {
    let snapshot: RepositorySnapshot
    @Binding var selectedCommit: GitCommit?
    @Binding var selectedStash: GitStash?
    @EnvironmentObject private var model: AppModel
    @State private var query = ""
    @State private var hoveredCommitHash: String?
    @State private var commitToRevert: GitCommit?
    @State private var cherryPickRequest: (commit: GitCommit, branch: String, head: String?)?
    @State private var resetRequest: ResetRequest?
    private let referenceWidth: CGFloat = 220
    private let authorWidth: CGFloat = 140
    private let dateWidth: CGFloat = 165
    private let rowHeight: CGFloat = 36

    private func matches(_ commit: GitCommit) -> Bool {
        query.isEmpty || [commit.subject, commit.hash, commit.authorName, commit.refs.joined(separator: " ")]
            .contains { $0.localizedCaseInsensitiveContains(query) }
    }

    var body: some View {
        let hasChanges = !snapshot.status.isEmpty
        let rows = hasChanges
            ? GitGraph.layoutWithWorkingTree(snapshot.commits, headHash: snapshot.headHash)
            : GitGraph.layout(snapshot.commits)
        let railWidth = max(92, CGFloat(rows.map(\.laneCount).max() ?? 1) * 22 + 26)
        VStack(spacing: 0) {
            HStack(spacing: 12) {
                Text("Repository graph").font(.system(size: 14, weight: .semibold))
                Spacer()
                Text("\(snapshot.commits.count) commits").font(.caption.monospaced()).foregroundStyle(.secondary)
            }.padding(.horizontal, 16).padding(.top, 14).padding(.bottom, 10)
            TextField("Filter loaded commits", text: $query)
                .textFieldStyle(.roundedBorder).padding(.horizontal, 16).padding(.bottom, 12)
            GeometryReader { geometry in
                let messageWidth = max(260, geometry.size.width - referenceWidth - railWidth - authorWidth - dateWidth - 12)
                ScrollView([.horizontal, .vertical]) {
                    VStack(alignment: .leading, spacing: 0) {
                        HStack(spacing: 0) {
                            Text("Branch/Tag").frame(width: referenceWidth, alignment: .leading)
                            Text("Graph").frame(width: railWidth, alignment: .leading)
                            Text("Commit Message").frame(width: messageWidth, alignment: .leading)
                            Text("Author").frame(width: authorWidth, alignment: .leading)
                            Text("Commit Date").frame(width: dateWidth, alignment: .leading)
                        }
                        .font(.system(size: 10, weight: .semibold, design: .monospaced))
                        .foregroundStyle(.secondary).padding(.leading, 12).frame(height: 30)
                        .background(AppPalette.toolbar)

                        ForEach(snapshot.stashes.filter { query.isEmpty || $0.message.localizedCaseInsensitiveContains(query) || $0.reference.localizedCaseInsensitiveContains(query) }) { stash in
                            Button {
                                selectedCommit = nil
                                selectedStash = stash
                            } label: {
                                HStack(spacing: 0) {
                                    Text(stash.reference).font(.system(size: 11, weight: .semibold, design: .monospaced))
                                        .foregroundStyle(.orange).frame(width: referenceWidth, alignment: .leading)
                                    Image(systemName: "archivebox.fill")
                                        .font(.system(size: 15)).foregroundStyle(.orange)
                                        .frame(width: 36, height: rowHeight)
                                        .frame(width: railWidth, alignment: .leading)
                                    Text(stash.message).font(.system(size: 13)).lineLimit(1)
                                        .frame(width: messageWidth, alignment: .leading)
                                    Color.clear.frame(width: authorWidth + dateWidth)
                                }.padding(.leading, 12).frame(height: rowHeight)
                                    .background(Color.orange.opacity(selectedStash?.hash == stash.hash ? 0.22 : 0.07)).contentShape(Rectangle())
                            }.buttonStyle(.plain).help("Inspect \(stash.reference): \(stash.message)")
                        }

                        if hasChanges {
                        Button { selectedCommit = nil; selectedStash = nil } label: {
                            HStack(spacing: 0) {
                                HStack(spacing: 5) {
                                    Image(systemName: "folder")
                                    Text("WORKING TREE").font(.system(size: 10, weight: .bold, design: .monospaced))
                                }.foregroundStyle(AppPalette.signal)
                                    .frame(width: referenceWidth, alignment: .leading)
                                GraphRail(row: rows[0], workingTree: true, connected: query.isEmpty)
                                    .frame(width: railWidth, height: rowHeight)
                                HStack(spacing: 12) {
                                    Text(snapshot.status.isEmpty ? "Working tree clean" : "// WIP").foregroundStyle(.secondary)
                                    Label("\(snapshot.status.filter { $0.kind == .modified || $0.kind == .renamed }.count)", systemImage: "pencil").foregroundStyle(.yellow)
                                    Label("\(snapshot.status.filter { $0.kind == .added || $0.kind == .untracked }.count)", systemImage: "plus").foregroundStyle(AppPalette.signal)
                                    if snapshot.status.contains(where: { $0.kind == .deleted }) {
                                        Label("\(snapshot.status.filter { $0.kind == .deleted }.count)", systemImage: "minus").foregroundStyle(AppPalette.conflict)
                                    }
                                }.font(.system(size: 12)).frame(width: messageWidth, alignment: .leading)
                                Color.clear.frame(width: authorWidth + dateWidth)
                            }.padding(.leading, 12).frame(height: rowHeight)
                                .background(selectedCommit == nil ? AppPalette.signal.opacity(0.12) : AppPalette.signal.opacity(0.035))
                        }.buttonStyle(.plain).help("Show working-tree files and staging")
                        }

                        LazyVStack(spacing: 0) {
                            ForEach(Array(snapshot.commits.enumerated()), id: \.element.hash) { index, commit in
                                if matches(commit) {
                                    HStack(spacing: 0) {
                                            CommitReferences(refs: commit.refs, snapshot: snapshot,
                                                color: AppPalette.laneColors[rows[index + (hasChanges ? 1 : 0)].lane % AppPalette.laneColors.count],
                                                showingReferences: Binding(
                                                get: { hoveredCommitHash == commit.hash },
                                                set: { visible in
                                                    if visible { hoveredCommitHash = commit.hash }
                                                    else if hoveredCommitHash == commit.hash { hoveredCommitHash = nil }
                                                }
                                            ))
                                                .frame(width: referenceWidth, alignment: .leading)
                                        Button { selectedCommit = commit; selectedStash = nil } label: {
                                        HStack(spacing: 0) {
                                            GraphRail(row: rows[index + (hasChanges ? 1 : 0)], workingTree: false, connected: query.isEmpty)
                                                .frame(width: railWidth, height: rowHeight)
                                            Text(commit.subject).font(.system(size: 13)).lineLimit(1)
                                                .frame(width: messageWidth, alignment: .leading).help(commit.subject)
                                            Text(commit.authorName).font(.system(size: 12)).foregroundStyle(.secondary).lineLimit(1)
                                                .frame(width: authorWidth, alignment: .leading).help(commit.authorName)
                                            Text(commit.commitDate.map { $0.formatted(date: .numeric, time: .shortened) } ?? commit.relativeDate)
                                                .font(.system(size: 11)).foregroundStyle(.secondary).lineLimit(1)
                                                .frame(width: dateWidth, alignment: .leading)
                                        }
                                        .frame(height: rowHeight)
                                        .contentShape(Rectangle())
                                        }.buttonStyle(.plain)
                                    }
                                    .padding(.leading, 12).frame(height: rowHeight)
                                    .background(selectedCommit?.hash == commit.hash ? AppPalette.signal.opacity(0.14) : (index.isMultiple(of: 2) ? Color.clear : AppPalette.rowStripe))
                                    .contextMenu {
                                        Button("View patch") { model.inspect(commit) }
                                        Button("Create patch from commit...") { model.exportPatch(commit) }
                                            .disabled(commit.parents.count > 1)
                                        Button("Cherry-pick commit...") {
                                            cherryPickRequest = (commit, snapshot.currentBranch, snapshot.headHash)
                                        }.disabled(snapshot.operation != nil)
                                        Button("Revert commit...") { commitToRevert = commit }
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
                    .frame(width: referenceWidth + railWidth + messageWidth + authorWidth + dateWidth + 12, alignment: .leading)
                    .frame(minHeight: geometry.size.height, alignment: .topLeading)
                }
            }
            HStack {
                Image(systemName: "arrow.triangle.branch")
                Text(snapshot.currentBranch).lineLimit(1)
                Spacer()
                Text(snapshot.headHash.map { String($0.prefix(7)) } ?? "No commits")
            }.font(.system(size: 11, design: .monospaced)).foregroundStyle(.secondary)
                .padding(10).background(AppPalette.toolbar)
        }.background(AppPalette.canvas)
        .onChange(of: snapshot.commits) { hoveredCommitHash = nil }
        .onChange(of: query) { hoveredCommitHash = nil }
        .modifier(ResetConfirmation(request: $resetRequest))
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
        .confirmationDialog("Revert \(commitToRevert?.shortHash ?? "") on \(snapshot.currentBranch)?", isPresented: Binding(get: { commitToRevert != nil }, set: { if !$0 { commitToRevert = nil } })) {
            if let commit = commitToRevert {
                if commit.parents.count > 1 {
                    ForEach(Array(commit.parents.enumerated()), id: \.offset) { index, parent in
                        Button("Revert relative to parent \(index + 1) (\(parent.prefix(8)))") {
                            model.start(.revert, target: commit.hash, mainline: index + 1)
                        }
                    }
                } else {
                    Button("Create revert commit") { model.start(.revert, target: commit.hash) }
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
            $0.isRemote && ($0.displayName == local.upstream || $0.displayName == "origin/\(local.name)") && $0.tip == local.tip
                && !$0.name.hasSuffix("/HEAD")
        }
    }

    private var ordered: [String] {
        let pairedNames = Set(refs.compactMap { branch($0) }.compactMap { pairedRemote($0)?.name })
        return refs.filter { ref in
            if snapshot.remotes.contains(where: { ref == "\($0)/HEAD" || ref == "remotes/\($0)/HEAD" }) { return false }
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
        guard let branch = branch(ref), !branch.isCurrent, !(branch.isRemote && branch.name.hasSuffix("/HEAD")),
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
                    .padding(.horizontal, 7).frame(height: 27)
                    .background(color.opacity(isBadgeHovered ? 0.42 : 0.30), in: RoundedRectangle(cornerRadius: 3))
                if ordered.count > 1 {
                    Text("+\(ordered.count - 1)").font(.system(size: 10, weight: .semibold)).fixedSize()
                        .padding(.horizontal, 6).frame(height: 27)
                        .background(color.opacity(isBadgeHovered ? 0.34 : 0.22), in: RoundedRectangle(cornerRadius: 3))
                }
            }
            .padding(.trailing, 8)
            .help(ordered.map { displayName($0) }.joined(separator: ", "))
            .contentShape(Rectangle())
            .onHover {
                isBadgeHovered = $0
                if $0 && ordered.count > 1 { showingReferences = true }
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
            Color.clear.frame(height: 27)
                .accessibilityHidden(true)
        }
    }
}

private struct GraphRail: View {
    let row: GitGraphRow
    let workingTree: Bool
    let connected: Bool

    var body: some View {
        Canvas { context, size in
            if connected {
                for edge in row.segments {
                    let start = CGPoint(x: 18 + CGFloat(edge.fromLane) * 22, y: edge.startsAtNode ? size.height / 2 : 0)
                    let end = CGPoint(x: 18 + CGFloat(edge.toLane) * 22, y: edge.endsAtNode ? size.height / 2 : size.height)
                    var path = Path()
                    path.move(to: start)
                    path.addCurve(to: end, control1: CGPoint(x: start.x, y: (start.y + end.y) / 2), control2: CGPoint(x: end.x, y: (start.y + end.y) / 2))
                    context.stroke(path, with: .color(AppPalette.laneColors[edge.fromLane % AppPalette.laneColors.count]), style: StrokeStyle(lineWidth: 2, dash: workingTree ? [3, 3] : []))
                }
            }
            let x = 18 + CGFloat(row.lane) * 22
            let circle = Path(ellipseIn: CGRect(x: x - 5, y: size.height / 2 - 5, width: 10, height: 10))
            let color = AppPalette.laneColors[row.lane % AppPalette.laneColors.count]
            context.fill(circle, with: .color(workingTree ? AppPalette.canvas : color))
            context.stroke(circle, with: .color(color), lineWidth: 2)
            if workingTree {
                context.stroke(Path(ellipseIn: CGRect(x: x - 9, y: size.height / 2 - 9, width: 18, height: 18)), with: .color(color.opacity(0.5)), lineWidth: 1)
            }
        }
    }
}
