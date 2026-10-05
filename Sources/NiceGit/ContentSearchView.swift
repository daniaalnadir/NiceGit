import NiceGitCore
import SwiftUI

struct ContentSearchRequest: Identifiable {
    let id = UUID()
    let repositoryURL: URL
    /// The commit to search, or nil for the working files.
    let revision: String?
    let label: String
}

/// Finds lines containing some text across tracked files, at a commit or in the working files.
/// Selecting a match opens blame at that line.
struct ContentSearchView: View {
    let request: ContentSearchRequest
    @Environment(\.dismiss) private var dismiss
    @State private var query = ""
    @State private var ignoreCase = true
    @State private var matches: [GitContentMatch] = []
    @State private var searching = false
    @State private var searched = false
    @State private var error: String?
    @State private var searchID = UUID()
    @State private var blameRequest: BlameRequest?
    @FocusState private var focused: Bool
    private let limit = 1000

    var body: some View {
        VStack(spacing: 0) {
            HStack(spacing: 10) {
                Image(systemName: "text.magnifyingglass").foregroundStyle(.secondary)
                TextField("Text in files", text: $query).textFieldStyle(.plain).font(.system(size: 15)).focused($focused)
                    .onSubmit { searchID = UUID() }
                Toggle("Match case", isOn: Binding(get: { !ignoreCase }, set: { ignoreCase = !$0 })).toggleStyle(.checkbox)
                Button("Done") { dismiss() }.keyboardShortcut(.cancelAction)
            }.padding(14)
            HStack {
                Text("Searching \(request.label)").font(.caption).foregroundStyle(.secondary)
                Spacer()
                if searched && !searching {
                    Text(matches.count == limit ? "First \(limit) matches" : "\(matches.count) \(matches.count == 1 ? "match" : "matches") in \(Set(matches.map(\.path)).count) \(Set(matches.map(\.path)).count == 1 ? "file" : "files")")
                        .font(.caption.monospaced()).foregroundStyle(.secondary)
                }
            }.padding(.horizontal, 14).padding(.bottom, 8)
            Divider()
            Group {
                if searching {
                    ProgressView().frame(maxWidth: .infinity, maxHeight: .infinity)
                } else if let error {
                    Text(error).foregroundStyle(.red).padding().frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
                } else if matches.isEmpty {
                    Text(searched ? "No matches" : "Press Return to search every tracked text file.")
                        .foregroundStyle(.secondary).frame(maxWidth: .infinity, maxHeight: .infinity)
                } else {
                    ScrollView {
                        LazyVStack(alignment: .leading, spacing: 0, pinnedViews: [.sectionHeaders]) {
                            ForEach(groupedPaths, id: \.self) { path in
                                Section {
                                    ForEach(matches.filter { $0.path == path }) { match in
                                        Button { open(match) } label: {
                                            HStack(alignment: .firstTextBaseline, spacing: 10) {
                                                Text("\(match.line)").font(.system(size: 11, design: .monospaced)).foregroundStyle(.tertiary)
                                                    .frame(width: 44, alignment: .trailing)
                                                Text(match.text.trimmingCharacters(in: .whitespaces)).font(.system(size: 12, design: .monospaced))
                                                    .lineLimit(1).truncationMode(.tail)
                                                Spacer(minLength: 0)
                                            }
                                            .padding(.horizontal, 14).padding(.vertical, 4).contentShape(Rectangle())
                                        }.buttonStyle(.plain).help("Show who last changed this line")
                                    }
                                } header: {
                                    Text(path).font(.system(size: 12, weight: .semibold, design: .monospaced)).lineLimit(1).truncationMode(.middle)
                                        .padding(.horizontal, 14).padding(.vertical, 6)
                                        .frame(maxWidth: .infinity, alignment: .leading).background(AppPalette.toolbar)
                                }
                            }
                        }
                    }
                }
            }.frame(maxWidth: .infinity, maxHeight: .infinity).background(AppPalette.panel)
        }
        .frame(width: 720, height: 520)
        .onAppear { focused = true }
        .onChange(of: ignoreCase) { if searched { searchID = UUID() } }
        .task(id: searchID) { await search() }
        .sheet(item: $blameRequest) { BlameView(request: $0) }
    }

    private var groupedPaths: [String] {
        var seen = Set<String>()
        return matches.map(\.path).filter { seen.insert($0).inserted }
    }

    private func open(_ match: GitContentMatch) {
        blameRequest = BlameRequest(path: match.path, repositoryURL: request.repositoryURL, revision: request.revision,
                                    revisionLabel: request.revision == nil ? nil : request.label, focusLine: match.line)
    }

    private func search() async {
        let text = query, url = request.repositoryURL, revision = request.revision, ignoreCase = ignoreCase, limit = limit
        guard !text.isEmpty else { return }
        searching = true
        error = nil
        let control = GitCommandControl()
        do {
            let found = try await withTaskCancellationHandler {
                try await Task.detached { try GitClient(control: control).searchContents(text, at: revision, ignoreCase: ignoreCase, limit: limit, in: url) }.value
            } onCancel: { control.cancel() }
            guard !Task.isCancelled else { return }
            matches = found
        } catch { if !Task.isCancelled { self.error = error.localizedDescription; matches = [] } }
        if !Task.isCancelled { searching = false; searched = true }
    }
}
