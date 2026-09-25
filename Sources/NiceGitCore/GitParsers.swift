import Foundation

public enum GitStatusParser {
    public static func parseWithCheckout(_ output: String) -> (entries: [GitStatusEntry], branch: String?, headHash: String?, isComplete: Bool) {
        let records = output.split(separator: "\0", omittingEmptySubsequences: true)
        var entries: [GitStatusEntry] = []
        var branch: String?
        var headHash: String?
        var sawOID = false
        var complete = true
        var index = 0
        while index < records.count {
            let record = String(records[index])
            index += 1
            if record.hasPrefix("# branch.head ") {
                branch = String(record.dropFirst("# branch.head ".count))
                continue
            }
            if record.hasPrefix("# branch.oid ") {
                sawOID = true
                let oid = String(record.dropFirst("# branch.oid ".count))
                headHash = oid == "(initial)" ? nil : oid
                continue
            }
            if record.hasPrefix("? ") {
                let path = String(record.dropFirst(2))
                guard !path.isEmpty else { complete = false; continue }
                entries.append(entry(path: path, original: nil, x: "?", y: "?"))
                continue
            }
            if record.hasPrefix("# ") { continue }
            let isRename = record.hasPrefix("2 ")
            let isConflict = record.hasPrefix("u ")
            guard record.hasPrefix("1 ") || isRename || isConflict,
                  record.count >= 4,
                  let path = path(after: isConflict ? 10 : isRename ? 9 : 8, in: record), !path.isEmpty else {
                complete = false
                continue
            }
            let xy = record.dropFirst(2)
            let x = xy.first == "." ? " " : xy.first ?? " "
            let y = xy.dropFirst().first == "." ? " " : xy.dropFirst().first ?? " "
            let original: String?
            if isRename {
                guard index < records.count, !records[index].isEmpty else { complete = false; continue }
                original = String(records[index])
                index += 1
            } else { original = nil }
            entries.append(entry(path: path, original: original, x: x, y: y, conflicted: isConflict))
        }
        return (entries, branch, headHash, complete && sawOID && branch != nil)
    }

    private static func path(after fields: Int, in record: String) -> String? {
        var start = record.startIndex
        for _ in 0..<fields {
            guard let space = record[start...].firstIndex(of: " ") else { return nil }
            start = record.index(after: space)
        }
        return String(record[start...])
    }

    private static func entry(path: String, original: String?, x: Character, y: Character, conflicted: Bool = false) -> GitStatusEntry {
        let kind: GitStatusKind
        if conflicted { kind = .conflicted }
        else if x == "?" { kind = .untracked }
        else if x == "R" || y == "R" { kind = .renamed }
        else if x == "A" || y == "A" { kind = .added }
        else if x == "D" || y == "D" { kind = .deleted }
        else { kind = .modified }
        return GitStatusEntry(path: path, originalPath: original, kind: kind, indexStatus: x, workTreeStatus: y)
    }

    public static func parseNullTerminated(_ output: String) -> [GitStatusEntry] {
        let records = output.split(separator: "\0", omittingEmptySubsequences: false)
        var entries: [GitStatusEntry] = []
        var index = 0
        while index < records.count {
            let record = String(records[index])
            index += 1
            guard record.count >= 4 else { continue }
            let x = record[record.startIndex]
            let y = record[record.index(after: record.startIndex)]
            let path = String(record.dropFirst(3))
            var original: String?
            if x == "R" || x == "C" || y == "R" || y == "C" {
                guard index < records.count else { break }
                original = String(records[index])
                index += 1
            }
            let conflict = ["DD", "AU", "UD", "UA", "DU", "AA", "UU"].contains(String([x, y]))
            entries.append(entry(path: path, original: original, x: x, y: y, conflicted: conflict))
        }
        return entries
    }

    public static func parse(_ output: String) -> [GitStatusEntry] {
        output
            .split(separator: "\n", omittingEmptySubsequences: true)
            .compactMap { parseLine(String($0)) }
    }

    private static func parseLine(_ line: String) -> GitStatusEntry? {
        guard line.count >= 3 else {
            return nil
        }

        let indexStatus = line[line.startIndex]
        let workTreeStatus = line[line.index(after: line.startIndex)]
        let pathStart = line.index(line.startIndex, offsetBy: 3)
        let rawPath = String(line[pathStart...])

        let parts = rawPath.components(separatedBy: " -> ")
        let originalPath = parts.count > 1 ? parts.first : nil
        let path = parts.last ?? rawPath

        let kind: GitStatusKind
        if indexStatus == "?" && workTreeStatus == "?" {
            kind = .untracked
        } else if indexStatus == "U" || workTreeStatus == "U" {
            kind = .conflicted
        } else if indexStatus == "R" {
            kind = .renamed
        } else if indexStatus == "A" || workTreeStatus == "A" {
            kind = .added
        } else if indexStatus == "D" || workTreeStatus == "D" {
            kind = .deleted
        } else {
            kind = .modified
        }

        return GitStatusEntry(
            path: path,
            originalPath: originalPath,
            kind: kind,
            indexStatus: indexStatus,
            workTreeStatus: workTreeStatus
        )
    }
}

public enum GitBranchParser {
    public static let format = "%(refname)%09%(HEAD)%09%(objectname)%09%(contents:subject)%09%(upstream)%09%(symref)"

    public static func parse(_ output: String, includesSymref: Bool = false) -> [GitBranch] {
        output
            .split(separator: "\n", omittingEmptySubsequences: true)
            .compactMap { line in
                let fields = String(line).components(separatedBy: "\t")
                guard fields.count >= (includesSymref ? 6 : 4) else {
                    return nil
                }

                let name = fields[0]
                let isRemote = name.hasPrefix("refs/remotes/") || name.hasPrefix("remotes/")
                let normalizedName = name.hasPrefix("refs/heads/") ? String(name.dropFirst(11)) :
                    (name.hasPrefix("refs/remotes/") ? String(name.dropFirst(5)) : name)
                if isRemote && (includesSymref ? !fields[fields.count - 1].isEmpty : normalizedName.split(separator: "/").count == 3 && normalizedName.hasSuffix("/HEAD")) { return nil }
                let subjectEnd = fields.count - (includesSymref ? 2 : (fields.count > 4 ? 1 : 0))
                return GitBranch(
                    name: normalizedName,
                    isCurrent: fields[1] == "*",
                    isRemote: isRemote,
                    tip: fields[2],
                    subject: fields[3..<subjectEnd].joined(separator: "\t"),
                    upstream: fields.count > 4 && fields[includesSymref ? fields.count - 2 : fields.count - 1] != ""
                        ? fields[includesSymref ? fields.count - 2 : fields.count - 1] : nil
                )
            }
    }
}

public enum GitLogParser {
    public static func parse(_ output: String) -> [GitCommit] {
        output
            .components(separatedBy: "\u{1e}")
            .compactMap { record in
                let cleanRecord = record.trimmingCharacters(in: .whitespacesAndNewlines)
                guard !cleanRecord.isEmpty else {
                    return nil
                }

                let fields = cleanRecord.components(separatedBy: "\u{1f}")
                guard fields.count >= 8 else {
                    return nil
                }

                let refs = fields[3]
                    .split(separator: ",")
                    .map { $0.trimmingCharacters(in: .whitespacesAndNewlines) }
                    .filter { !$0.isEmpty }

                return GitCommit(
                    hash: fields[0],
                    shortHash: fields[1],
                    parents: fields[2].split(separator: " ").map(String.init),
                    refs: refs,
                    subject: fields[4],
                    authorName: fields[5],
                    authorEmail: fields[6],
                    relativeDate: fields[7],
                    commitDate: fields.count > 8 ? TimeInterval(fields[8]).map { Date(timeIntervalSince1970: $0) } : nil
                )
            }
    }
}

public enum GitRemoteParser {
    public static func allAddresses(_ output: String, direction: String) -> [String: [String]] {
        var result: [String: [String]] = [:]
        let suffix = " (" + direction + ")"
        for line in output.split(separator: "\n") where line.hasSuffix(suffix) {
            let fields = line.dropLast(suffix.count).split(separator: "\t", maxSplits: 1)
            if fields.count == 2 { result[String(fields[0]), default: []].append(String(fields[1])) }
        }
        return result
    }

    public static func addresses(_ output: String) -> [String: String] {
        var result: [String: String] = [:]
        for line in output.split(separator: "\n") where line.hasSuffix(" (fetch)") {
            let fields = line.dropLast(8).split(separator: "\t", maxSplits: 1)
            if fields.count == 2 { result[String(fields[0])] = String(fields[1]) }
        }
        return result
    }

    public static func parse(_ output: String) -> [String] {
        let names = output
            .split(separator: "\n", omittingEmptySubsequences: true)
            .compactMap { line -> String? in
                String(line).split(separator: "\t").first.map(String.init)
            }

        return Array(Set(names)).sorted()
    }
}
