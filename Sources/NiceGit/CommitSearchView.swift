import NiceGitCore
import SwiftUI

/// Searches every branch's history, not just the loaded graph page, and opens the chosen commit.
struct CommitSearchView: View {
    let repositoryURL: URL
    @State var query: String
    let choose: (GitCommit) -> Void
    @Environment(\.dismiss) private var dismiss
    @State private var field: GitCommitSearchField = .message
    @State private var results: [GitCommit] = []
    @State private var searching = false
    @State private var searched = false
    @State private var error: String?
    @State private var searchID = UUID()
    @FocusState private var focused: Bool
    private let limit = 200

    var body: some View {
        VStack(spacing: 0) {
            HStack(spacing: 10) {
                Image(systemName: "magnifyingglass").foregroundStyle(.secondary)
                TextField(field == .change ? "Text added or removed in a file" : "Search all commits", text: $query)
                    .textFieldStyle(.plain).font(.system(size: 15)).focused($focused)
                    .onSubmit { searchID = UUID() }
                Picker("Search in", selection: $field) {
                    ForEach(GitCommitSearchField.allCases, id: \.self) { Text($0.rawValue).tag($0) }
                }.labelsHidden().frame(width: 140)
                Button("Done") { dismiss() }.keyboardShortcut(.cancelAction)
            }.padding(14)
            Divider()
            Group {
                if searching {
                    ProgressView().frame(maxWidth: .infinity, maxHeight: .infinity)
                } else if let error {
                    Text(error).foregroundStyle(.red).padding().frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
                } else if results.isEmpty {
                    Text(searched ? "No commits found" : "Press Return to search every branch, including history not yet loaded in the graph.")
                        .foregroundStyle(.secondary).multilineTextAlignment(.center).padding()
                        .frame(maxWidth: .infinity, maxHeight: .infinity)
                } else {
                    ScrollView {
                        LazyVStack(alignment: .leading, spacing: 0) {
                            if results.count == limit {
                                Text("Showing the newest \(limit) matches").font(.caption).foregroundStyle(.secondary).padding(.horizontal, 14).padding(.top, 8)
                            }
                            ForEach(results) { commit in
                                Button { choose(commit); dismiss() } label: {
                                    VStack(alignment: .leading, spacing: 3) {
                                        Text(commit.subject).font(.system(size: 13)).lineLimit(1)
                                        Text("\(commit.authorName) · \(commit.relativeDate) · \(commit.shortHash)")
                                            .font(.system(size: 11)).foregroundStyle(.secondary)
                                    }
                                    .padding(.horizontal, 14).padding(.vertical, 8)
                                    .frame(maxWidth: .infinity, alignment: .leading).contentShape(Rectangle())
                                }.buttonStyle(.plain).help("Open \(commit.shortHash) in the inspector")
                            }
                        }
                    }
                }
            }.frame(maxWidth: .infinity, maxHeight: .infinity).background(AppPalette.panel)
        }
        .frame(width: 620, height: 480)
        .onAppear { focused = true; if !query.isEmpty { searchID = UUID() } }
        .onChange(of: field) { if searched { searchID = UUID() } }
        .task(id: searchID) { await search() }
    }

    private func search() async {
        let text = query, field = field, url = repositoryURL, limit = limit
        guard !text.trimmingCharacters(in: .whitespaces).isEmpty else { return }
        searching = true
        error = nil
        let control = GitCommandControl()
        do {
            let found = try await withTaskCancellationHandler {
                try await Task.detached { try GitClient(control: control).searchCommits(text, in: field, limit: limit, in: url) }.value
            } onCancel: { control.cancel() }
            guard !Task.isCancelled else { return }
            results = found
        } catch { if !Task.isCancelled { self.error = error.localizedDescription; results = [] } }
        if !Task.isCancelled { searching = false; searched = true }
    }
}
