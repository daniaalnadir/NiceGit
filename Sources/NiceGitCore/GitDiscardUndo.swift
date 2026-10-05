import Foundation

/// What a discarded path looked like before and after, so the discard can be reversed while
/// nothing else has touched the path. File contents are kept as Git objects, byte for byte.
public struct GitDiscardUndo: Equatable, Sendable {
    public struct Version: Equatable, Sendable {
        let mode: String
        let blob: String
    }
    public let path: String
    let indexBefore: Version?
    let workingBefore: Version?
    let indexAfter: Version?
    let workingAfter: Version?
}

extension GitClient {
    /// Discards `entry` like `discard`, first saving what is needed to undo it. Returns nil (and
    /// still discards) for paths that cannot be restored exactly: renames, copies, conflicts,
    /// submodules, and folders.
    public func discardKeepingUndo(_ entry: GitStatusEntry, in url: URL) throws -> GitDiscardUndo? {
        try discardKeepingUndoWithStatus(entry, in: url).undo
    }

    /// Discards like `discardKeepingUndo` and also returns the status read to verify it, with
    /// branch headers and operation, so the caller need not read the status again.
    public func discardKeepingUndoWithStatus(_ entry: GitStatusEntry, in url: URL) throws
        -> (undo: GitDiscardUndo?, status: (entries: [GitStatusEntry], branch: String?, headHash: String?, isComplete: Bool), operation: GitOperation?) {
        let supported = entry.originalPath == nil && entry.kind != .conflicted && entry.kind != .renamed && !entry.path.hasSuffix("/")
        // Validate, check for HEAD, and save both versions of the file at the same time.
        var status: Result<[GitStatusEntry], Error> = .success([])
        var hasHead = false
        var indexBefore: Result<GitDiscardUndo.Version?, Error> = .success(nil)
        var workingBefore: Result<GitDiscardUndo.Version?, Error> = .success(nil)
        inParallel(
            { status = Result { try self.loadStatus(in: url, paths: [entry.path] + [entry.originalPath].compactMap { $0 }) } },
            { hasHead = (try? self.run(["rev-parse", "--verify", "HEAD"], in: url)) != nil },
            { if supported { indexBefore = Result { try self.indexVersion(entry.path, in: url) } } },
            { if supported { workingBefore = Result { try self.workingVersion(entry.path, in: url, store: true) } } })
        try discard(entry, statusBefore: status.get(), hasHead: hasHead, in: url)

        var statusAfter: Result<(status: (entries: [GitStatusEntry], branch: String?, headHash: String?, isComplete: Bool), operation: GitOperation?), Error> = .success(((entries: [], branch: nil, headHash: nil, isComplete: false), nil))
        var indexAfter: Result<GitDiscardUndo.Version?, Error> = .success(nil)
        var workingAfter: Result<GitDiscardUndo.Version?, Error> = .success(nil)
        inParallel(
            { statusAfter = Result { try self.loadStatusWithCheckoutAndOperation(in: url) } },
            { if supported { indexAfter = Result { try self.indexVersion(entry.path, in: url) } } },
            { if supported { workingAfter = Result { try self.workingVersion(entry.path, in: url, store: false) } } })
        let after = try statusAfter.get()
        try verifyDiscard(entry, statusAfter: after.status.entries)

        // Paths whose versions could not be saved, such as folders, are discarded without undo.
        guard supported, let before = try? (indexBefore.get(), workingBefore.get()),
              before.0?.mode != "160000", before.0 != nil || before.1 != nil else { return (nil, after.status, after.operation) }
        return (GitDiscardUndo(path: entry.path, indexBefore: before.0, workingBefore: before.1,
                               indexAfter: try indexAfter.get(), workingAfter: try workingAfter.get()), after.status, after.operation)
    }

    /// Puts back the staged and working versions a discard removed. Refuses if the path has
    /// changed since the discard, so newer work is never overwritten.
    public func undoDiscard(_ undo: GitDiscardUndo, in url: URL) throws {
        guard try indexVersion(undo.path, in: url) == undo.indexAfter, try workingVersion(undo.path, in: url, store: false) == undo.workingAfter else {
            throw GitClientError.commandFailed(command: "undo discard", message: "\(undo.path) changed after it was discarded, so the discard cannot be undone safely.")
        }
        let file = url.appendingPathComponent(undo.path)
        if let working = undo.workingBefore {
            let data = try runData(["cat-file", "blob", working.blob], in: url)
            try FileManager.default.createDirectory(at: file.deletingLastPathComponent(), withIntermediateDirectories: true)
            if (try? FileManager.default.attributesOfItem(atPath: file.path)) != nil { try FileManager.default.removeItem(at: file) }
            if working.mode == "120000" {
                try FileManager.default.createSymbolicLink(atPath: file.path, withDestinationPath: String(decoding: data, as: UTF8.self))
            } else {
                try data.write(to: file)
                try FileManager.default.setAttributes([.posixPermissions: working.mode == "100755" ? 0o755 : 0o644], ofItemAtPath: file.path)
            }
        } else if (try? FileManager.default.attributesOfItem(atPath: file.path)) != nil {
            try FileManager.default.removeItem(at: file)
        }
        if let index = undo.indexBefore {
            try run(["update-index", "--add", "--cacheinfo", index.mode, index.blob, undo.path], in: url)
        } else if undo.indexAfter != nil {
            try run(["rm", "--cached", "--quiet", "--", undo.path], in: url)
        }
        guard try indexVersion(undo.path, in: url) == undo.indexBefore, try workingVersion(undo.path, in: url, store: false) == undo.workingBefore else {
            throw GitClientError.commandFailed(command: "undo discard", message: "\(undo.path) was restored, but it does not exactly match its state before the discard. Review it.")
        }
    }

    private func indexVersion(_ path: String, in url: URL) throws -> GitDiscardUndo.Version? {
        try run(["ls-files", "-z", "--stage", "--", path], in: url).split(separator: "\0").compactMap { record -> GitDiscardUndo.Version? in
            let parts = record.split(separator: "\t", maxSplits: 1)
            let fields = parts.first?.split(separator: " ") ?? []
            guard parts.count == 2, parts[1] == Substring(path), fields.count == 3, fields[2] == "0" else { return nil }
            return GitDiscardUndo.Version(mode: String(fields[0]), blob: String(fields[1]))
        }.first
    }

    /// The file on disk as Git would record it; `store` also writes its contents to the object database.
    private func workingVersion(_ path: String, in url: URL, store: Bool) throws -> GitDiscardUndo.Version? {
        let file = url.appendingPathComponent(path)
        guard let attributes = try? FileManager.default.attributesOfItem(atPath: file.path) else { return nil }
        let write = store ? ["-w"] : []
        switch attributes[.type] as? FileAttributeType {
        case .typeSymbolicLink?:
            let target = try FileManager.default.destinationOfSymbolicLink(atPath: file.path)
            let temporary = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
            try Data(target.utf8).write(to: temporary)
            defer { try? FileManager.default.removeItem(at: temporary) }
            return .init(mode: "120000", blob: try run(["hash-object", "--no-filters"] + write + ["--", temporary.path], in: url).trimmingCharacters(in: .whitespacesAndNewlines))
        case .typeRegular?:
            let executable = ((attributes[.posixPermissions] as? Int) ?? 0) & 0o111 != 0
            return .init(mode: executable ? "100755" : "100644",
                         blob: try run(["hash-object", "--no-filters"] + write + ["--", file.path], in: url).trimmingCharacters(in: .whitespacesAndNewlines))
        default:
            throw GitClientError.commandFailed(command: "discard", message: "Only files can be restored after a discard.")
        }
    }
}
