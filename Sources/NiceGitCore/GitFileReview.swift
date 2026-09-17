import Foundation

public struct GitFileReview: Sendable {
    public let path: String
    public let staged: Bool
    public let untracked: Bool
    public let patch: String
    public let lines: [GitDiffLine]
    public let lineStagingUnavailable: String?
}

public struct GitEditableFile: Sendable {
    public let path: String
    public let data: Data
    public let text: String
}

extension GitClient {
    public func fileReview(path: String, staged: Bool, in repository: URL) throws -> GitFileReview {
        let entry = try loadStatus(in: repository).first { $0.path == path }
        let untracked = entry?.kind == .untracked
        let options = ["--no-ext-diff", "--no-textconv", "--no-color", "--no-renames", "--unified=1000000"]
        let patch: String
        if untracked && !staged {
            patch = try run(["diff", "--no-index"] + options + ["--", "/dev/null", path], in: repository, acceptedStatuses: [0, 1])
        } else {
            patch = try run(["diff"] + options + (staged ? ["--cached"] : []) + ["--", path], in: repository)
        }
        let lines = GitDiffLine.parse(patch)
        let reason: String?
        if entry?.kind == .conflicted { reason = "Resolve conflicts before staging individual lines." }
        else if entry?.originalPath != nil { reason = "Stage or unstage renamed files as a whole." }
        else if lines.contains(where: { $0.kind == .metadata && ($0.text.hasSuffix("120000") || $0.text.hasSuffix("160000") || $0.text.hasPrefix("Binary files ")) }) {
            reason = "Binary files, symbolic links, and submodules require whole-file staging."
        } else if patch.utf8.count > 4_000_000 { reason = "This diff is too large for line staging." }
        else { reason = nil }
        return GitFileReview(path: path, staged: staged, untracked: untracked, patch: patch, lines: lines, lineStagingUnavailable: reason)
    }

    public func stageLines(_ selected: Set<Int>, from review: GitFileReview, in repository: URL) throws {
        if let reason = review.lineStagingUnavailable { throw reviewError(reason) }
        guard !selected.isEmpty, selected.allSatisfy({ review.lines.indices.contains($0) && [.addition, .deletion].contains(review.lines[$0].kind) }) else {
            throw reviewError("Select added or removed lines first.")
        }
        let fresh = try fileReview(path: review.path, staged: review.staged, in: repository)
        guard fresh.patch == review.patch, fresh.lineStagingUnavailable == nil else {
            throw reviewError("The file or index changed. Reload the diff before staging lines.")
        }
        // Build an exact full-file index patch; Git validates the baseline and locks the index.
        // Unselected deletions remain context in the target, and unselected additions are omitted.
        var baseline = ""
        var target = ""
        var removed: [(Int, String)] = []
        var added: [(Int, String)] = []
        func appendTarget(_ text: String) {
            if !target.isEmpty && !target.hasSuffix("\n") { target += "\n" }
            target += text
        }
        func flushChanges() {
            // Pair adjacent replacements so selecting one pair does not move it past
            // an unselected neighboring replacement in the index.
            for offset in 0..<max(removed.count, added.count) {
                if removed.indices.contains(offset) {
                    let (index, text) = removed[offset]
                    if review.staged ? selected.contains(index) : !selected.contains(index) { appendTarget(text) }
                }
                if added.indices.contains(offset) {
                    let (index, text) = added[offset]
                    if review.staged ? !selected.contains(index) : selected.contains(index) { appendTarget(text) }
                }
            }
            removed = []
            added = []
        }
        for (index, line) in review.lines.enumerated() {
            guard [.context, .addition, .deletion].contains(line.kind) else { continue }
            let noNewline = review.lines.indices.contains(index + 1) && review.lines[index + 1].text == "\\ No newline at end of file"
            let text = String(line.text.dropFirst()) + (noNewline ? "" : "\n")
            let isOld = line.kind == .context || line.kind == .deletion
            let isNew = line.kind == .context || line.kind == .addition
            if review.staged ? isNew : isOld { baseline += text }
            if line.kind == .context { flushChanges(); appendTarget(text) }
            else if line.kind == .deletion { removed.append((index, text)) }
            else { added.append((index, text)) }
        }
        flushChanges()
        guard baseline != target else { throw reviewError("The selected lines do not change the index.") }
        func quoted(_ path: String) throws -> String {
            let data = try JSONSerialization.data(withJSONObject: path, options: [.fragmentsAllowed, .withoutEscapingSlashes])
            return String(decoding: data, as: UTF8.self)
        }
        func body(_ text: String, prefix: String) -> (Int, String) {
            guard !text.isEmpty else { return (0, "") }
            var parts = text.components(separatedBy: "\n")
            if text.hasSuffix("\n") { parts.removeLast() }
            var result = parts.map { prefix + $0 + "\n" }.joined()
            if !text.hasSuffix("\n") { result += "\\ No newline at end of file\n" }
            return (parts.count, result)
        }
        let old = body(baseline, prefix: "-")
        let new = body(target, prefix: "+")
        let createsFile = review.staged ? review.patch.contains("+++ /dev/null\n") : review.patch.contains("--- /dev/null\n")
        let removesFile = target.isEmpty && (review.staged ? review.patch.contains("--- /dev/null\n") : review.patch.contains("+++ /dev/null\n"))
        let a = try quoted("a/" + review.path)
        let b = try quoted("b/" + review.path)
        var patch = "diff --git \(a) \(b)\n"
        if createsFile {
            let mode = review.patch.components(separatedBy: "\n").first { $0.hasPrefix(review.staged ? "deleted file mode " : "new file mode ") }?.split(separator: " ").last ?? "100644"
            patch += "new file mode \(mode)\n"
        }
        if removesFile {
            let mode = review.patch.components(separatedBy: "\n").first { $0.hasPrefix(review.staged ? "new file mode " : "deleted file mode ") }?.split(separator: " ").last ?? "100644"
            patch += "deleted file mode \(mode)\n"
        }
        patch += "--- \(createsFile ? "/dev/null" : a)\n+++ \(removesFile ? "/dev/null" : b)\n"
        patch += "@@ -\(old.0 == 0 ? 0 : 1),\(old.0) +\(new.0 == 0 ? 0 : 1),\(new.0) @@\n" + old.1 + new.1
        let temporary = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try Data(patch.utf8).write(to: temporary, options: .atomic)
        defer { try? FileManager.default.removeItem(at: temporary) }
        try run(["apply", "--cached", "--whitespace=nowarn", "--", temporary.path], in: repository)
    }

    public func editableFile(path: String, in repository: URL) throws -> GitEditableFile {
        let file = try reviewFile(path: path, in: repository)
        let attributes = try FileManager.default.attributesOfItem(atPath: file.path)
        guard (attributes[.size] as? NSNumber)?.intValue ?? 0 <= 2_000_000 else { throw reviewError("This file is too large for the built-in editor.") }
        let data = try Data(contentsOf: file)
        guard !data.contains(0), let text = String(data: data, encoding: .utf8) else { throw reviewError("Only UTF-8 text files can be edited here.") }
        return GitEditableFile(path: path, data: data, text: text)
    }

    public func saveFile(_ document: GitEditableFile, text: String, in repository: URL) throws {
        let file = try reviewFile(path: document.path, in: repository)
        guard try Data(contentsOf: file) == document.data else { throw reviewError("The file changed outside this editor. Reload before saving; your edits have not been written.") }
        let attributes = try FileManager.default.attributesOfItem(atPath: file.path)
        try Data(text.utf8).write(to: file, options: .atomic)
        if let mode = attributes[.posixPermissions] { try FileManager.default.setAttributes([.posixPermissions: mode], ofItemAtPath: file.path) }
    }

    private func reviewFile(path: String, in repository: URL) throws -> URL {
        let root = repository.resolvingSymlinksInPath().standardizedFileURL
        let file = root.appendingPathComponent(path).standardizedFileURL
        guard !path.hasPrefix("/"), !path.split(separator: "/").contains(".."),
              !path.split(separator: "/").contains(where: { $0.lowercased() == ".git" }),
              file.resolvingSymlinksInPath().path.hasPrefix(root.path + "/"),
              try FileManager.default.attributesOfItem(atPath: file.path)[.type] as? FileAttributeType == .typeRegular else {
            throw reviewError("Only regular working files inside this repository can be edited.")
        }
        return file
    }

    private func reviewError(_ message: String) -> GitClientError {
        .commandFailed(command: "file review", message: message)
    }
}
