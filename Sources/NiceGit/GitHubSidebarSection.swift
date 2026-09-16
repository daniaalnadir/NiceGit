import NiceGitCore
import SwiftUI

struct GitHubSidebarSection: View {
    let kind: GitHubItemKind
    let rootPath: String
    let remotes: [String]
    @State private var items: [GitHubItem]?
    @State private var remote: String?
    @State private var error: String?
    @State private var loading = false
    @State private var revision = 0
    @State private var limit = 100
    @State private var itemState = GitHubItemState.open
    @State private var query = ""
    @State private var control: GitCommandControl?

    var body: some View {
        SidebarSection(title: kind == .pullRequest ? "Pull Requests" : "Issues", icon: kind == .pullRequest ? "arrow.triangle.pull" : "exclamationmark.circle", count: items?.count) {
            VStack(alignment: .leading, spacing: 8) {
                HStack {
                    Menu(remote.map { "GitHub / " + $0 } ?? "Load from GitHub") {
                        ForEach(remotes, id: \.self) { name in
                            Button(name) { remote = name; limit = 100; revision += 1 }
                        }
                    }.disabled(remotes.isEmpty || loading)
                    Spacer(minLength: 0)
                    if loading {
                        Button { control?.cancel() } label: { Image(systemName: "xmark") }
                            .buttonStyle(.plain).help("Cancel GitHub request")
                    } else if remote != nil {
                        Button { revision += 1 } label: { Image(systemName: "arrow.clockwise") }
                            .buttonStyle(.plain).help("Refresh GitHub items")
                    }
                }
                if remote != nil {
                    Picker("State", selection: $itemState) {
                        ForEach(GitHubItemState.allCases.filter { kind == .pullRequest || $0 != .merged }, id: \.self) { state in
                            Text(state.rawValue.capitalized).tag(state)
                        }
                    }.disabled(loading)
                    .onChange(of: itemState) { limit = 100; revision += 1 }
                    TextField("Filter loaded items", text: $query).textFieldStyle(.roundedBorder)
                }
                if loading { ProgressView().controlSize(.small) }
                if remotes.isEmpty { Text("No remotes configured").font(.caption).foregroundStyle(.secondary) }
                if let error { Text(error).font(.caption).foregroundStyle(.red).textSelection(.enabled) }
                if let items {
                    let visible = items.filter { $0.matches(query) }
                    if items.isEmpty { Text("No \(itemState == .all ? "matching" : itemState.rawValue) items").font(.caption).foregroundStyle(.secondary) }
                    else if visible.isEmpty { Text("No matches in loaded items").font(.caption).foregroundStyle(.secondary) }
                    ForEach(visible) { item in
                        Link(destination: item.url) {
                            VStack(alignment: .leading, spacing: 3) {
                                Text(item.title).font(.system(size: 12, weight: .medium)).lineLimit(2).multilineTextAlignment(.leading)
                                Text(metadata(for: item))
                                    .font(.caption).foregroundStyle(.secondary).lineLimit(1)
                            }.frame(maxWidth: .infinity, alignment: .leading).padding(.vertical, 4)
                        }.help(item.url.absoluteString)
                    }
                    if items.count == limit {
                        if limit < 1000 {
                            Button("Load more") { limit += 100; revision += 1 }.disabled(loading)
                        } else {
                            Text("First 1,000 items").font(.caption).foregroundStyle(.secondary)
                        }
                    }
                }
            }.padding(.horizontal, 14).padding(.vertical, 10)
        }
        .task(id: revision) {
            guard revision > 0, let remote else { return }
            let requestControl = GitCommandControl()
            let requestLimit = limit
            let requestState = itemState
            control = requestControl
            loading = true
            items = nil
            error = nil
            defer { if !Task.isCancelled { loading = false; control = nil } }
            do {
                let result = try await withTaskCancellationHandler {
                    try await Task.detached {
                        try GitHubClient().load(kind: kind, remote: remote, in: URL(fileURLWithPath: rootPath), limit: requestLimit, state: requestState, control: requestControl)
                    }.value
                } onCancel: { requestControl.cancel() }
                guard !Task.isCancelled else { return }
                items = result
            } catch {
                guard !Task.isCancelled else { return }
                self.error = requestControl.isCancelled ? "Request cancelled" : error.localizedDescription
            }
        }
    }

    private func metadata(for item: GitHubItem) -> String {
        var parts = ["#\(item.number)"]
        if let state = item.state { parts.append(state.capitalized) }
        if item.isDraft == true { parts.append("Draft") }
        if let author = item.author { parts.append(author.login) }
        return parts.joined(separator: " · ")
    }
}
