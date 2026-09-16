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
    private let referenceWidth: CGFloat = 138
    private let rowHeight: CGFloat = 52

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
                ScrollView([.horizontal, .vertical]) {
                    VStack(alignment: .leading, spacing: 0) {
                        HStack(spacing: 0) {
                            Text("REFERENCES").frame(width: referenceWidth, alignment: .leading)
                            Text("GRAPH").frame(width: railWidth, alignment: .leading)
                            Text("COMMIT")
                            Spacer()
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
                                VStack(alignment: .leading, spacing: 4) {
                                    Text(snapshot.status.isEmpty ? "Working tree clean" : "\(snapshot.status.count) changed files")
                                        .font(.system(size: 13, weight: .semibold))
                                    Text("\(snapshot.stagedCount) staged  ·  \(snapshot.unstagedCount) unstaged")
                                        .font(.system(size: 11, design: .monospaced)).foregroundStyle(.secondary)
                                }
                                Spacer(minLength: 12)
                                Image(systemName: snapshot.status.isEmpty ? "checkmark.circle" : "pencil.circle")
                                    .foregroundStyle(snapshot.status.isEmpty ? AppPalette.signal : AppPalette.merge)
                                    .padding(.trailing, 14)
                            }.padding(.leading, 12).frame(height: rowHeight)
                                .background(selectedCommit == nil ? AppPalette.signal.opacity(0.12) : AppPalette.signal.opacity(0.035))
                        }.buttonStyle(.plain).help("Show working-tree files and staging")

                        LazyVStack(spacing: 0) {
                            ForEach(Array(snapshot.commits.enumerated()), id: \.element.hash) { index, commit in
                                if matches(commit) {
                                    Button { selectedCommit = commit } label: {
                                        HStack(spacing: 0) {
                                            VStack(alignment: .leading, spacing: 3) {
                                                ForEach(commit.refs.prefix(2), id: \.self) { ref in
                                                    Text(ref).font(.system(size: 10, weight: .medium, design: .monospaced))
                                                        .lineLimit(1).padding(.horizontal, 6).padding(.vertical, 3)
                                                        .background(ref.contains("HEAD") ? AppPalette.signal.opacity(0.18) : AppPalette.branchTag)
                                                        .clipShape(RoundedRectangle(cornerRadius: 4)).help(ref)
                                                }
                                            }.frame(width: referenceWidth, alignment: .leading)
                                            GraphRail(row: rows[index + 1], workingTree: false, connected: query.isEmpty)
                                                .frame(width: railWidth, height: rowHeight)
                                            VStack(alignment: .leading, spacing: 4) {
                                                Text(commit.subject).font(.system(size: 13, weight: .medium)).lineLimit(1)
                                                HStack(spacing: 9) {
                                                    Text(commit.shortHash).foregroundStyle(AppPalette.signal)
                                                    Text(commit.authorName).lineLimit(1)
                                                    Text(commit.relativeDate).lineLimit(1)
                                                }.font(.system(size: 10, design: .monospaced)).foregroundStyle(.secondary)
                                            }
                                            Spacer(minLength: 12)
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
                    .frame(width: max(geometry.size.width, referenceWidth + railWidth + 300), alignment: .leading)
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
