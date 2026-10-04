import AppKit
import NiceGitCore
import SwiftUI

/// Before and after versions of a changed image, side by side, with their sizes.
struct ImageComparisonView: View {
    let selection: DiffSelection
    @State private var before: Side?
    @State private var after: Side?
    @State private var loading = true
    @State private var error: String?

    private struct Side {
        let data: Data?
        var image: NSImage? { data.flatMap(NSImage.init(data:)) }
    }

    static let extensions: Set<String> = ["png", "jpg", "jpeg", "gif", "bmp", "tif", "tiff", "heic", "webp", "ico", "icns"]

    static func isImage(_ path: String) -> Bool {
        extensions.contains(URL(fileURLWithPath: path).pathExtension.lowercased())
    }

    /// The two versions a selection compares, matching what its text diff would show.
    static func versions(for selection: DiffSelection) -> (old: (String, GitFileVersion)?, new: (String, GitFileVersion)?) {
        guard let path = selection.path else { return (nil, nil) }
        if let from = selection.compareFrom {
            return ((path, .revision(from)), (path, selection.commitHash.map { .revision($0) } ?? .workingFile))
        }
        if let hash = selection.commitHash { return ((path, .revision(hash + "^1")), (path, .revision(hash))) }
        if selection.untracked { return (nil, (path, .workingFile)) }
        if selection.staged { return ((selection.originalPath ?? path, .revision("HEAD")), (path, .index)) }
        return ((path, .index), (path, .workingFile))
    }

    var body: some View {
        Group {
            if loading {
                ProgressView().frame(maxWidth: .infinity, maxHeight: .infinity)
            } else if let error {
                ContentUnavailableView("Unable to load image", systemImage: "photo", description: Text(error))
            } else {
                HStack(spacing: 0) {
                    pane("Before", before, missing: "Added in this change")
                    AppPalette.line.frame(width: 1)
                    pane("After", after, missing: "Deleted in this change")
                }
            }
        }
        .task(id: selection.id) { await load() }
    }

    private func pane(_ title: String, _ side: Side?, missing: String) -> some View {
        VStack(spacing: 8) {
            HStack {
                Text(title).font(.system(size: 12, weight: .semibold))
                Spacer()
                if let data = side?.data {
                    Text(details(data, side?.image)).font(.caption.monospaced()).foregroundStyle(.secondary)
                }
            }
            Group {
                if let image = side?.image {
                    // Never enlarge past natural size, so a change in dimensions stays visible.
                    Image(nsImage: image).resizable().interpolation(.high).scaledToFit()
                        .frame(maxWidth: image.size.width, maxHeight: image.size.height)
                        .background(Color.primary.opacity(0.04))
                        .accessibilityLabel("\(title) image")
                } else {
                    Text(side?.data == nil ? missing : "This format cannot be previewed")
                        .foregroundStyle(.secondary)
                }
            }.frame(maxWidth: .infinity, maxHeight: .infinity)
        }.padding(14).frame(maxWidth: .infinity, maxHeight: .infinity)
    }

    private func details(_ data: Data, _ image: NSImage?) -> String {
        let size = ByteCountFormatter.string(fromByteCount: Int64(data.count), countStyle: .file)
        guard let rep = image?.representations.first, rep.pixelsWide > 0 else { return size }
        return "\(rep.pixelsWide) × \(rep.pixelsHigh) · \(size)"
    }

    private func load() async {
        let versions = Self.versions(for: selection), url = selection.repositoryURL
        do {
            let loaded = try await Task.detached { () -> (Data?, Data?) in
                let git = GitClient()
                return (try versions.old.flatMap { try git.fileData(path: $0.0, at: $0.1, in: url) },
                        try versions.new.flatMap { try git.fileData(path: $0.0, at: $0.1, in: url) })
            }.value
            before = Side(data: loaded.0)
            after = Side(data: loaded.1)
        } catch { self.error = error.localizedDescription }
        loading = false
    }
}
