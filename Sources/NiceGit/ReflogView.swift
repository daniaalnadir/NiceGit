import NiceGitCore
import SwiftUI

/// Where HEAD has been, newest first, for recovering commits left behind by a reset, rebase,
/// amend, or deleted branch. Entries on no branch are marked; a branch can be created at any.
struct ReflogView: View {
    let repositoryURL: URL
    @EnvironmentObject private var model: AppModel
    @Environment(\.dismiss) private var dismiss
    @State private var entries: [GitReflogEntry] = []
    @State private var unreachable: Set<String> = []
    @State private var loading = true
    @State private var error: String?
    @State private var selected: GitReflogEntry?
    @State private var branchTarget: GitReflogEntry?
    @State private var branchName = ""

    var body: some View {
        VStack(spacing: 0) {
            HStack {
                VStack(alignment: .leading, spacing: 3) {
                    Text("Recover lost work").font(.headline)
                    Text("Every position HEAD has had, including commits no branch points to any more.")
                        .font(.caption).foregroundStyle(.secondary)
                }
                Spacer()
                Button("Done") { dismiss() }.keyboardShortcut(.defaultAction)
            }.padding()
            Divider()
            HSplitView {
                ScrollView {
                    LazyVStack(alignment: .leading, spacing: 0) {
                        if loading { ProgressView().frame(maxWidth: .infinity).padding() }
                        if let error { Text(error).foregroundStyle(.red).padding() }
                        if !loading && error == nil && entries.isEmpty { Text("No history yet").foregroundStyle(.secondary).padding() }
                        ForEach(entries) { entry in row(entry) }
                    }
                }.frame(minWidth: 340, idealWidth: 420, maxWidth: 560).background(AppPalette.panel)
                Group {
                    if let selected {
                        DiffView(selection: DiffSelection(title: selected.subject, repositoryURL: repositoryURL, commitHash: selected.hash),
                                 onClose: { self.selected = nil })
                            .id(selected.id)
                    } else {
                        Text("Select an entry to see its commit").foregroundStyle(.secondary).frame(maxWidth: .infinity, maxHeight: .infinity)
                    }
                }.frame(minWidth: 420, maxWidth: .infinity, maxHeight: .infinity)
            }
        }
        .frame(minWidth: 960, minHeight: 580)
        .alert("Create branch at \(branchTarget?.shortHash ?? "")", isPresented: Binding(get: { branchTarget != nil }, set: { if !$0 { branchTarget = nil } })) {
            TextField("Branch name", text: $branchName)
            Button("Cancel", role: .cancel) {}
            Button("Create branch") {
                if let branchTarget {
                    model.createBranch(named: branchName, at: branchTarget.hash)
                    unreachable.remove(branchTarget.hash)
                }
            }.disabled(branchName.trimmingCharacters(in: .whitespaces).isEmpty)
        } message: {
            Text("The branch keeps this commit and its history. Your current checkout does not change.")
        }
        .task { await load() }
    }

    private func row(_ entry: GitReflogEntry) -> some View {
        Button { selected = entry } label: {
            VStack(alignment: .leading, spacing: 3) {
                HStack(spacing: 6) {
                    Text(entry.subject).font(.system(size: 13)).lineLimit(1)
                    Spacer(minLength: 4)
                    if unreachable.contains(entry.hash) {
                        Text("Not on any branch").font(.system(size: 10, weight: .semibold)).foregroundStyle(.orange)
                            .help("Only the reflog keeps this commit; Git may delete it after the reflog expires")
                    }
                }
                Text(entry.action).font(.system(size: 11)).foregroundStyle(.secondary).lineLimit(1)
                HStack(spacing: 6) {
                    Text(entry.selector).font(.system(size: 10, design: .monospaced))
                    Text(entry.shortHash).font(.system(size: 10, design: .monospaced))
                    if let date = entry.date { Text(date, format: .relative(presentation: .named)).font(.system(size: 10)) }
                }.foregroundStyle(.tertiary)
            }
            .padding(.horizontal, 14).padding(.vertical, 8)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(AppPalette.signal.opacity(selected?.id == entry.id ? 0.20 : 0))
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .contextMenu {
            Button("Create branch here...") { branchName = ""; branchTarget = entry }
            Button("Copy commit hash") {
                NSPasteboard.general.clearContents()
                NSPasteboard.general.setString(entry.hash, forType: .string)
            }
        }
        .accessibilityAction(named: "Create branch here") { branchName = ""; branchTarget = entry }
    }

    private func load() async {
        let url = repositoryURL
        do {
            let loaded = try await Task.detached { () -> ([GitReflogEntry], Set<String>) in
                let git = GitClient()
                let entries = try git.reflog(in: url)
                return (entries, try git.unreachableCommits(entries.map(\.hash), in: url))
            }.value
            entries = loaded.0
            unreachable = loaded.1
            selected = loaded.0.first { loaded.1.contains($0.hash) }
        } catch { self.error = error.localizedDescription }
        loading = false
    }
}
