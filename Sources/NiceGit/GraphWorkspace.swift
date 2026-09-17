import NiceGitCore
import SwiftUI

struct GraphWorkspace: View {
    let snapshot: RepositorySnapshot
    @Binding var selectedCommit: GitCommit?
    @EnvironmentObject private var model: AppModel
    @State private var query = ""
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
        let rows = GitGraph.layoutWithWorkingTree(snapshot.commits, headHash: snapshot.headHash)
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

                        Button { selectedCommit = nil } label: {
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

                        LazyVStack(spacing: 0) {
                            ForEach(Array(snapshot.commits.enumerated()), id: \.element.hash) { index, commit in
                                if matches(commit) {
                                    Button { selectedCommit = commit } label: {
                                        HStack(spacing: 0) {
                                            CommitReferences(refs: commit.refs)
                                                .frame(width: referenceWidth, alignment: .leading)
                                            GraphRail(row: rows[index + 1], workingTree: false, connected: query.isEmpty)
                                                .frame(width: railWidth, height: rowHeight)
                                            Text(commit.subject).font(.system(size: 13)).lineLimit(1)
                                                .frame(width: messageWidth, alignment: .leading).help(commit.subject)
                                            Text(commit.authorName).font(.system(size: 12)).foregroundStyle(.secondary).lineLimit(1)
                                                .frame(width: authorWidth, alignment: .leading).help(commit.authorName)
                                            Text(commit.commitDate.map { $0.formatted(date: .numeric, time: .shortened) } ?? commit.relativeDate)
                                                .font(.system(size: 11)).foregroundStyle(.secondary).lineLimit(1)
                                                .frame(width: dateWidth, alignment: .leading)
                                        }
                                        .padding(.leading, 12).frame(height: rowHeight)
                                        .background(selectedCommit?.hash == commit.hash ? AppPalette.signal.opacity(0.14) : (index.isMultiple(of: 2) ? Color.clear : AppPalette.rowStripe))
                                        .contentShape(Rectangle())
                                    }
                                    .buttonStyle(.plain)
                                    .help(commit.subject)
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
    @State private var showingReferences = false
    @State private var closeTask: Task<Void, Never>?

    private func hover(_ inside: Bool) {
        closeTask?.cancel()
        if inside {
            if refs.count > 1 { showingReferences = true }
        } else {
            closeTask = Task { @MainActor in
                try? await Task.sleep(for: .milliseconds(250))
                guard !Task.isCancelled else { return }
                showingReferences = false
            }
        }
    }

    private var ordered: [String] {
        refs.sorted { left, right in
            let leftHead = left == "HEAD" || left.hasPrefix("HEAD -> ")
            let rightHead = right == "HEAD" || right.hasPrefix("HEAD -> ")
            if leftHead != rightHead { return leftHead }
            return left.localizedStandardCompare(right) == .orderedAscending
        }
    }

    private func label(_ ref: String) -> some View {
        HStack(spacing: 5) {
            Image(systemName: ref.hasPrefix("HEAD") ? "checkmark" : ref.hasPrefix("tag: ") ? "tag" : "arrow.triangle.branch")
            Text(ref.hasPrefix("HEAD -> ") ? String(ref.dropFirst(8)) : ref)
                .lineLimit(1).truncationMode(.middle)
        }.font(.system(size: 12, weight: .medium))
    }

    var body: some View {
        if let first = ordered.first {
            HStack(spacing: 5) {
                label(first)
                if refs.count > 1 {
                    Text("+\(refs.count - 1)").font(.system(size: 10, weight: .semibold)).fixedSize()
                }
            }
            .padding(.horizontal, 7).frame(height: 27)
            .background(AppPalette.signal.opacity(0.23), in: RoundedRectangle(cornerRadius: 3))
            .padding(.trailing, 8)
            .contentShape(Rectangle())
            .onHover(perform: hover)
            .onDisappear { closeTask?.cancel() }
            .onTapGesture { if refs.count > 1 { showingReferences.toggle() } }
            .help(ordered.joined(separator: "\n"))
            .popover(isPresented: $showingReferences, arrowEdge: .bottom) {
                ScrollView {
                    VStack(alignment: .leading, spacing: 0) {
                        ForEach(ordered, id: \.self) { ref in
                            label(ref).padding(9).frame(maxWidth: .infinity, alignment: .leading).help(ref)
                        }
                    }
                }.frame(width: 340, height: min(CGFloat(refs.count) * 36, 300))
                    .onHover(perform: hover)
            }
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
