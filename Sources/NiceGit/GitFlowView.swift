import NiceGitCore
import SwiftUI

/// Sets up GitFlow, starts feature, release, and hotfix branches, and finishes the current one.
struct GitFlowView: View {
    @EnvironmentObject private var model: AppModel
    @Environment(\.dismiss) private var dismiss
    @State private var configuration: GitFlowConfiguration?
    @State private var draft = GitFlowConfiguration()
    @State private var kind: GitFlowKind = .feature
    @State private var name = ""
    @State private var tagMessage = ""
    @State private var loaded = false

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            HStack {
                Text("GitFlow").font(.title2.bold())
                Spacer()
                Button { dismiss() } label: { Image(systemName: "xmark") }.help("Close")
            }
            if !loaded {
                ProgressView()
            } else if let configuration {
                start(configuration)
                Divider()
                finish(configuration)
            } else {
                setup
            }
            if let error = model.errorMessage { Text(error).foregroundStyle(.red).textSelection(.enabled) }
        }
        .padding(24).frame(width: 480)
        .textFieldStyle(.roundedBorder)
        .disabled(model.isLoading)
        .task(id: model.snapshot?.lastUpdated) { await load() }
    }

    private var setup: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text("Branches for releases, ongoing development, and short-lived work, compatible with the git-flow tool.")
                .font(.callout).foregroundStyle(.secondary)
            Form {
                TextField("Production branch", text: $draft.mainBranch)
                TextField("Development branch", text: $draft.developBranch)
                TextField("Feature prefix", text: $draft.featurePrefix)
                TextField("Release prefix", text: $draft.releasePrefix)
                TextField("Hotfix prefix", text: $draft.hotfixPrefix)
                TextField("Version tag prefix", text: $draft.versionTagPrefix)
            }
            HStack {
                Spacer()
                Button("Set up GitFlow") { model.initializeGitFlow(draft) }
                    .keyboardShortcut(.defaultAction)
                    .disabled(draft.mainBranch.isEmpty || draft.developBranch.isEmpty)
            }
        }
    }

    private func start(_ configuration: GitFlowConfiguration) -> some View {
        let base = kind == .hotfix ? configuration.mainBranch : configuration.developBranch
        return VStack(alignment: .leading, spacing: 10) {
            Text("Start").font(.headline)
            Picker("Kind", selection: $kind) {
                ForEach(GitFlowKind.allCases, id: \.self) { Text($0.rawValue.capitalized).tag($0) }
            }.pickerStyle(.segmented).labelsHidden()
            TextField(kind == .feature ? "Feature name" : "Version", text: $name)
            HStack {
                Text("Creates \(prefix(kind, configuration))\(name.isEmpty ? "…" : name) from \(base) and checks it out.")
                    .font(.caption).foregroundStyle(.secondary)
                Spacer()
                Button("Start \(kind.rawValue)") {
                    guard let snapshot = model.snapshot else { return }
                    model.startGitFlow(kind, name: name, expectedBranch: snapshot.currentBranch, expectedHead: snapshot.headHash) { name = "" }
                }.disabled(name.trimmingCharacters(in: .whitespaces).isEmpty || model.snapshot?.operation != nil)
            }
        }
    }

    @ViewBuilder
    private func finish(_ configuration: GitFlowConfiguration) -> some View {
        let current = model.snapshot?.currentBranch ?? ""
        let finishing = GitFlowKind.allCases.first { current.hasPrefix(prefix($0, configuration)) && current.count > prefix($0, configuration).count }
        VStack(alignment: .leading, spacing: 10) {
            Text("Finish").font(.headline)
            if let finishing {
                let version = String(current.dropFirst(prefix(finishing, configuration).count))
                let targets = finishing == .feature ? configuration.developBranch : "\(configuration.mainBranch) and \(configuration.developBranch)"
                Text("Merges \(current) into \(targets) without fast-forwarding\(finishing == .feature ? "" : ", tags \(configuration.versionTagPrefix)\(version) on \(configuration.mainBranch),") then deletes \(current). If a merge conflicts, NiceGit stops so you can resolve it, then you can finish again.")
                    .font(.caption).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
                if finishing != .feature {
                    TextField("Tag message (optional)", text: $tagMessage)
                }
                HStack {
                    Spacer()
                    Button("Finish \(current)") {
                        guard let snapshot = model.snapshot else { return }
                        model.finishGitFlow(expectedBranch: snapshot.currentBranch, expectedHead: snapshot.headHash,
                                            tagMessage: tagMessage.isEmpty ? nil : tagMessage) { tagMessage = "" }
                    }.disabled(model.snapshot?.operation != nil || model.snapshot?.status.contains { $0.kind != .untracked } == true)
                }
            } else {
                Text("Check out a \(configuration.featurePrefix)…, \(configuration.releasePrefix)…, or \(configuration.hotfixPrefix)… branch to finish it.")
                    .font(.caption).foregroundStyle(.secondary)
            }
        }
    }

    private func prefix(_ kind: GitFlowKind, _ configuration: GitFlowConfiguration) -> String {
        switch kind {
        case .feature: configuration.featurePrefix
        case .release: configuration.releasePrefix
        case .hotfix: configuration.hotfixPrefix
        }
    }

    private func load() async {
        guard let url = model.repositoryURL else { return }
        let found = await Task.detached { GitClient().gitFlowConfiguration(in: url) }.value
        configuration = found
        if found == nil, let snapshot = model.snapshot {
            // Suggest the repository's existing production branch.
            draft.mainBranch = snapshot.branches.contains { !$0.isRemote && $0.name == "main" } ? "main"
                : snapshot.branches.contains { !$0.isRemote && $0.name == "master" } ? "master" : snapshot.currentBranch
        }
        loaded = true
    }
}
