import NiceGitCore
import SwiftUI

struct InteractiveRebaseRequest: Identifiable {
    let id = UUID()
    /// The oldest commit to edit; everything from it through HEAD is listed.
    let oldest: String
    let branch: String
    let head: String
    let repositoryURL: URL
}

/// Edits the commits from a chosen commit through HEAD: drag to reorder, and choose to keep,
/// reword, squash, fix up, or drop each one. Newest commits are listed first, as in the graph.
struct InteractiveRebaseView: View {
    let request: InteractiveRebaseRequest
    @EnvironmentObject private var model: AppModel
    @Environment(\.dismiss) private var dismiss
    @State private var plan: GitRebasePlan?
    @State private var entries: [Entry] = []
    @State private var error: String?

    fileprivate enum Choice: String, CaseIterable, Identifiable {
        case pick = "Pick", reword = "Reword", squash = "Squash", fixup = "Fixup", drop = "Drop"
        var id: String { rawValue }
        var help: String {
            switch self {
            case .pick: "Keep this commit as it is"
            case .reword: "Keep this commit with a new message"
            case .squash: "Combine into the commit below, keeping both messages"
            case .fixup: "Combine into the commit below, discarding this message"
            case .drop: "Remove this commit and its changes"
            }
        }
    }

    fileprivate struct Entry: Identifiable, Equatable {
        let commit: GitCommit
        var choice: Choice = .pick
        var message: String
        var id: String { commit.hash }
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack(alignment: .firstTextBaseline) {
                VStack(alignment: .leading, spacing: 3) {
                    Text("Interactive rebase").font(.headline)
                    Text("Rewrite \(entries.count) \(entries.count == 1 ? "commit" : "commits") on \(request.branch). Drag to reorder; the top is newest.")
                        .font(.caption).foregroundStyle(.secondary)
                }
                Spacer()
                Button("Reset") { reset() }.disabled(plan == nil || entries == original)
            }.padding()
            if let published = publishedCount, published > 0 {
                Label("\(published) of these commits are already on a remote. Rewriting them means you will need to force-push, and anyone who has them must reconcile their copies.",
                      systemImage: "exclamationmark.triangle.fill")
                    .font(.caption).foregroundStyle(.orange)
                    .padding(.horizontal).padding(.bottom, 8)
            }
            Divider()
            if let error {
                Text(error).foregroundStyle(.red).padding().frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
            } else if plan == nil {
                ProgressView().frame(maxWidth: .infinity, maxHeight: .infinity)
            } else {
                List {
                    ForEach($entries) { $entry in
                        row($entry)
                    }
                    .onMove { entries.move(fromOffsets: $0, toOffset: $1) }
                }
            }
            Divider()
            HStack {
                if let problem = validationProblem {
                    Label(problem, systemImage: "exclamationmark.circle").font(.caption).foregroundStyle(.red)
                } else if plan != nil {
                    Text(summary).font(.caption).foregroundStyle(.secondary)
                }
                Spacer()
                Button("Cancel") { dismiss() }.keyboardShortcut(.cancelAction)
                Button("Rewrite commits") { start() }
                    .keyboardShortcut(.defaultAction)
                    .disabled(plan == nil || validationProblem != nil || entries == original || model.isLoading)
            }.padding()
        }
        .frame(minWidth: 760, minHeight: 540)
        .task(id: request.id) { await load() }
    }

    private func row(_ entry: Binding<Entry>) -> some View {
        let value = entry.wrappedValue
        let index = entries.firstIndex { $0.id == value.id } ?? 0
        return VStack(alignment: .leading, spacing: 6) {
            HStack(spacing: 10) {
                Image(systemName: "line.3.horizontal").foregroundStyle(.tertiary).help("Drag to reorder")
                Picker("Action for \(value.commit.shortHash)", selection: entry.choice) {
                    ForEach(Choice.allCases) { Text($0.rawValue).tag($0).help($0.help) }
                }.labelsHidden().frame(width: 100)
                    .help(value.choice.help)
                Text(value.commit.subject).lineLimit(1)
                    .strikethrough(value.choice == .drop)
                    .foregroundStyle(value.choice == .drop ? .secondary : .primary)
                if value.choice == .squash || value.choice == .fixup {
                    Image(systemName: "arrow.turn.right.down").foregroundStyle(.secondary).help("Combined into the commit below")
                }
                Spacer(minLength: 8)
                Text("\(value.commit.authorName) · \(value.commit.shortHash)")
                    .font(.system(size: 11)).foregroundStyle(.secondary).lineLimit(1)
                if plan?.publishedCommits.contains(value.commit.hash) == true {
                    Image(systemName: "network").foregroundStyle(.orange).help("Already on a remote")
                }
                Button { move(index, by: -1) } label: { Image(systemName: "chevron.up") }
                    .buttonStyle(.plain).disabled(index == 0).help("Move newer")
                Button { move(index, by: 1) } label: { Image(systemName: "chevron.down") }
                    .buttonStyle(.plain).disabled(index == entries.count - 1).help("Move older")
            }
            if value.choice == .reword {
                TextEditor(text: entry.message)
                    .font(.system(size: 12, design: .monospaced))
                    .frame(minHeight: 60, maxHeight: 120)
                    .overlay(RoundedRectangle(cornerRadius: 4).stroke(Color.secondary.opacity(0.3)))
                    .padding(.leading, 128)
            }
        }.padding(.vertical, 3)
    }

    private var original: [Entry] {
        (plan?.commits ?? []).reversed().map { Entry(commit: $0, message: plan?.messages[$0.hash] ?? $0.subject) }
    }

    private var publishedCount: Int? { plan.map { plan in entries.filter { plan.publishedCommits.contains($0.commit.hash) }.count } }

    /// Git applies oldest first, so squash and fixup fold into the nearest kept commit below.
    private var validationProblem: String? {
        let oldestFirst = entries.reversed()
        if let first = oldestFirst.first(where: { $0.choice != .drop }), first.choice == .squash || first.choice == .fixup {
            return "The oldest kept commit has nothing below it to combine into."
        }
        if entries.contains(where: { $0.choice == .reword && $0.message.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty }) {
            return "A reworded commit needs a message."
        }
        return nil
    }

    private var summary: String {
        let kept = entries.filter { $0.choice == .pick || $0.choice == .reword }.count
        let dropped = entries.filter { $0.choice == .drop }.count
        return "Result: \(kept) \(kept == 1 ? "commit" : "commits")" + (dropped > 0 ? ", \(dropped) dropped" : "")
    }

    private func move(_ index: Int, by offset: Int) {
        let target = index + offset
        guard entries.indices.contains(index), entries.indices.contains(target) else { return }
        entries.swapAt(index, target)
    }

    private func reset() { entries = original }

    private func start() {
        guard let plan, validationProblem == nil else { return }
        let steps = entries.reversed().map { entry -> GitRebaseStep in
            switch entry.choice {
            case .pick: GitRebaseStep(commit: entry.commit, action: .pick)
            case .reword: GitRebaseStep(commit: entry.commit, action: .reword(entry.message))
            case .squash: GitRebaseStep(commit: entry.commit, action: .squash)
            case .fixup: GitRebaseStep(commit: entry.commit, action: .fixup)
            case .drop: GitRebaseStep(commit: entry.commit, action: .drop)
            }
        }
        model.interactiveRebase(steps, plan: plan, expectedBranch: request.branch, expectedHead: request.head)
        dismiss()
    }

    private func load() async {
        let oldest = request.oldest, url = request.repositoryURL
        do {
            let loaded = try await Task.detached { try GitClient().interactiveRebasePlan(from: oldest, in: url) }.value
            guard loaded.commits.last?.hash == request.head else {
                error = "The branch changed since this was opened. Close and try again."
                return
            }
            plan = loaded
            entries = original
        } catch { self.error = error.localizedDescription }
    }
}
