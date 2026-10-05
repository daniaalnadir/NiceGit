import CoreServices
import Foundation

/// Reports file-system changes in a repository's working tree and Git directories, so NiceGit
/// can refresh after edits, staging, commits, or fetches made outside it.
final class RepositoryWatcher {
    struct Change: Equatable {
        /// Branches, tags, HEAD, or the operation state changed, so history may have moved.
        var refsChanged: Bool
    }

    private var stream: FSEventStreamRef?
    private let gitDirectories: [String]
    private let onChange: (Change) -> Void

    /// Git rewrites these constantly without changing anything NiceGit shows.
    private static let ignoredGitEntries: Set<String> = ["objects", "logs", "lfs", "rr-cache", "fsmonitor--daemon"]

    init?(root: String, gitDirectories: [String], onChange: @escaping (Change) -> Void) {
        self.gitDirectories = gitDirectories
        self.onChange = onChange
        var context = FSEventStreamContext(version: 0, info: Unmanaged.passUnretained(self).toOpaque(), retain: nil, release: nil, copyDescription: nil)
        let callback: FSEventStreamCallback = { _, info, count, paths, _, _ in
            guard let info else { return }
            let watcher = Unmanaged<RepositoryWatcher>.fromOpaque(info).takeUnretainedValue()
            let changed = (unsafeBitCast(paths, to: NSArray.self) as? [String]) ?? []
            if let change = RepositoryWatcher.classify(Array(changed.prefix(count)), gitDirectories: watcher.gitDirectories) { watcher.onChange(change) }
        }
        guard let stream = FSEventStreamCreate(kCFAllocatorDefault, callback, &context, ([root] + gitDirectories) as CFArray,
                                               FSEventStreamEventId(kFSEventStreamEventIdSinceNow), 0.5,
                                               FSEventStreamCreateFlags(kFSEventStreamCreateFlagUseCFTypes | kFSEventStreamCreateFlagFileEvents)) else { return nil }
        self.stream = stream
        FSEventStreamSetDispatchQueue(stream, .main)
        FSEventStreamStart(stream)
    }

    deinit {
        guard let stream else { return }
        FSEventStreamStop(stream)
        FSEventStreamInvalidate(stream)
        FSEventStreamRelease(stream)
    }

    /// Decides whether changed paths matter. Inside a Git directory, the index means staging
    /// changed and anything else except object storage, logs, and lock files means refs or
    /// operation state changed. Elsewhere, files of nested repositories are ignored.
    static func classify(_ paths: [String], gitDirectories: [String]) -> Change? {
        let directories = gitDirectories.map { $0.hasSuffix("/") ? $0 : $0 + "/" }
        var relevant = false
        var refsChanged = false
        for path in paths {
            if let directory = directories.first(where: { path.hasPrefix($0) || path + "/" == $0 }) {
                let entry = String(path.dropFirst(min(directory.count, path.count))).split(separator: "/").first.map(String.init) ?? ""
                if ignoredGitEntries.contains(entry) || entry.hasSuffix(".lock") { continue }
                relevant = true
                if entry != "index" { refsChanged = true }
            } else if !path.split(separator: "/").contains(".git") {
                relevant = true
            }
        }
        return relevant ? Change(refsChanged: refsChanged) : nil
    }
}
