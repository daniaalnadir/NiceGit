import NiceGitCore
import SwiftUI

/// Lists local branches merged into the current branch, or inactive for a while, and deletes the
/// chosen ones as a single step that Undo can reverse.
struct BranchCleanupView: View {
    @EnvironmentObject private var model: AppModel
    @Environment(\.dismiss) private var dismiss
    @State private var candidates: [GitCleanupCandidate] = []
    @State private var selected: Set<String> = []
    @State private var inactiveDays = 90
    @State private var loading = true
    @State private var error: String?
    @State private var confirming = false

    private var selectedCandidates: [GitCleanupCandidate] { candidates.filter { selected.contains($0.name) } }
    private var selectedUnmerged: Int { selectedCandidates.filter { !$0.isMerged }.count }

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            HStack {
                VStack(alignment: .leading, spacing: 3) {
                    Text("Clean up branches").font(.title2.bold())
                    Text("Local branches merged into \(model.snapshot?.currentBranch ?? "the current branch"), or with no commits recently.")
                        .font(.caption).foregroundStyle(.secondary)
                }
                Spacer()
                Button { dismiss() } label: { Image(systemName: "xmark") }.help("Close")
            }
            Stepper("Include unmerged branches inactive for \(inactiveDays) days", value: $inactiveDays, in: 7...730, step: 30)
                .font(.callout)
            Divider()
            if loading {
                ProgressView().frame(maxWidth: .infinity)
            } else if let error {
                Text(error).foregroundStyle(.red)
            } else if candidates.isEmpty {
                Text("No branches to clean up.").foregroundStyle(.secondary)
            } else {
                ScrollView {
                    VStack(alignment: .leading, spacing: 6) {
                        ForEach(candidates) { candidate in
                            Toggle(isOn: Binding(get: { selected.contains(candidate.name) },
                                                 set: { if $0 { selected.insert(candidate.name) } else { selected.remove(candidate.name) } })) {
                                HStack(spacing: 8) {
                                    Text(candidate.name).font(.body.monospaced()).lineLimit(1).truncationMode(.middle)
                                    Text(candidate.isMerged ? "Merged" : "Not merged")
                                        .font(.caption.weight(.semibold))
                                        .foregroundStyle(candidate.isMerged ? Color.green : Color.orange)
                                    Spacer(minLength: 8)
                                    if let date = candidate.lastCommitDate {
                                        Text(date, format: .relative(presentation: .named)).font(.caption).foregroundStyle(.secondary)
                                    }
                                }
                            }
                            .toggleStyle(.checkbox)
                            .help(candidate.subject)
                        }
                    }.frame(maxWidth: .infinity, alignment: .leading)
                }.frame(maxHeight: 280)
            }
            HStack {
                if selectedUnmerged > 0 {
                    Label("\(selectedUnmerged) selected \(selectedUnmerged == 1 ? "branch has" : "branches have") commits on no other branch.", systemImage: "exclamationmark.triangle")
                        .font(.caption).foregroundStyle(.orange)
                }
                Spacer()
                Button("Delete \(selected.count) \(selected.count == 1 ? "branch" : "branches")...") { confirming = true }
                    .disabled(selected.isEmpty || model.isLoading)
            }
        }
        .padding(24).frame(width: 560)
        .task(id: "\(inactiveDays)-\(model.snapshot?.lastUpdated.timeIntervalSince1970 ?? 0)") { await load() }
        .confirmationDialog("Delete \(selected.count) local \(selected.count == 1 ? "branch" : "branches")?", isPresented: $confirming) {
            Button(selectedUnmerged > 0 ? "Delete, including unmerged" : "Delete branches", role: .destructive) {
                model.deleteBranches(selectedCandidates, includeUnmerged: selectedUnmerged > 0) { selected = [] }
            }
        } message: {
            Text("Remote branches are not changed. "
                 + (selectedUnmerged > 0 ? "Unmerged branches hold commits found on no other branch. " : "")
                 + "You can restore all of them with Undo until you take another undoable action.")
        }
    }

    private func load() async {
        guard let url = model.repositoryURL else { return }
        let days = inactiveDays
        do {
            let found = try await Task.detached { try GitClient().branchCleanupCandidates(inactiveDays: days, in: url) }.value
            // Merged branches start selected; unmerged ones need a deliberate choice.
            let previous = selected
            candidates = found
            selected = Set(found.filter { $0.isMerged || previous.contains($0.name) }.map(\.name))
            error = nil
        } catch { self.error = error.localizedDescription }
        loading = false
    }
}
