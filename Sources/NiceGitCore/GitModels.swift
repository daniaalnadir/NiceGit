import Foundation

public struct GitCommitFileChange: Identifiable, Sendable {
    public let path: String
    public let status: String
    public var id: String { path }
    public init(path: String, status: String) { self.path = path; self.status = status }
}

public struct RepositorySnapshot: Equatable, Sendable {
    public var rootPath: String
    public var name: String
    public var currentBranch: String
    public var status: [GitStatusEntry]
    public var branches: [GitBranch]
    public var commits: [GitCommit]
    public var remotes: [String]
    public var remoteAddresses: [String: String] = [:]
    public var remoteFetchAddresses: [String: [String]] = [:]
    public var remotePushAddresses: [String: [String]] = [:]
    public var lastUpdated: Date
    public var stashes: [GitStash] = []
    public var operation: GitOperation?
    public var hasMoreCommits = false
    public var tags: [String] = []
    public var tagTips: [String: String] = [:]
    public var upstream: String?
    public var ahead: Int?
    public var behind: Int?
    public var headHash: String?
    public var worktrees: [GitWorktree] = []

    public init(
        rootPath: String,
        name: String,
        currentBranch: String,
        status: [GitStatusEntry],
        branches: [GitBranch],
        commits: [GitCommit],
        remotes: [String],
        lastUpdated: Date = Date()
    ) {
        self.rootPath = rootPath
        self.name = name
        self.currentBranch = currentBranch
        self.status = status
        self.branches = branches
        self.commits = commits
        self.remotes = remotes
        self.lastUpdated = lastUpdated
    }

    public var stagedCount: Int {
        status.filter(\.isStaged).count
    }

    public var unstagedCount: Int {
        status.filter(\.isUnstaged).count
    }
}

public enum GitResetMode: String, CaseIterable, Sendable {
    case soft, mixed, hard

    public var title: String {
        switch self {
        case .soft: "Soft: keep changes staged"
        case .mixed: "Mixed: keep changes unstaged"
        case .hard: "Hard: discard local changes"
        }
    }

    public var warning: String {
        switch self {
        case .soft: "Moves HEAD without changing the index or working files. Changes from removed commits remain staged."
        case .mixed: "Moves HEAD and resets the index. Working files are kept, and changes become unstaged."
        case .hard: "Moves HEAD and replaces the index and tracked files. Uncommitted tracked changes are lost. Untracked files obstructing restored paths may also be deleted."
        }
    }
}

public enum GitOperation: String, Sendable {
    case merge
    case rebase
    case cherryPick = "cherry-pick"
    case revert
}

public struct GitStash: Identifiable, Equatable, Sendable {
    public let hash: String
    public let reference: String
    public let message: String
    public var id: String { hash }

    public init(hash: String, reference: String, message: String) {
        self.hash = hash
        self.reference = reference
        self.message = message
    }
}

public enum GitStatusKind: String, Equatable, Sendable {
    case added = "Added"
    case modified = "Modified"
    case deleted = "Deleted"
    case renamed = "Renamed"
    case conflicted = "Conflict"
    case untracked = "Untracked"
}

public struct GitStatusEntry: Identifiable, Equatable, Sendable {
    public var path: String
    public var originalPath: String?
    public var kind: GitStatusKind
    public var indexStatus: Character
    public var workTreeStatus: Character

    public init(
        path: String,
        originalPath: String? = nil,
        kind: GitStatusKind,
        indexStatus: Character,
        workTreeStatus: Character
    ) {
        self.path = path
        self.originalPath = originalPath
        self.kind = kind
        self.indexStatus = indexStatus
        self.workTreeStatus = workTreeStatus
    }

    public var id: String {
        "\(originalPath ?? "")\0\(path)\0\(indexStatus)\(workTreeStatus)"
    }

    public var fileName: String {
        URL(fileURLWithPath: path).lastPathComponent
    }

    public var isStaged: Bool {
        kind != .conflicted && indexStatus != " " && indexStatus != "?"
    }

    public var isUnstaged: Bool {
        kind == .conflicted || workTreeStatus != " "
    }
}

public struct GitBranch: Identifiable, Equatable, Sendable {
    public var name: String
    public var isCurrent: Bool
    public var isRemote: Bool
    public var tip: String
    public var subject: String
    public var upstream: String?

    public init(name: String, isCurrent: Bool, isRemote: Bool, tip: String, subject: String, upstream: String? = nil) {
        self.name = name
        self.isCurrent = isCurrent
        self.isRemote = isRemote
        self.tip = tip
        self.subject = subject
        self.upstream = upstream
    }

    public var id: String {
        "\(isRemote ? "remote" : "local")\0\(name)"
    }

    public var displayName: String {
        name.hasPrefix("remotes/") ? String(name.dropFirst("remotes/".count)) : name
    }
}

public struct GitCommit: Identifiable, Equatable, Sendable {
    public var hash: String
    public var shortHash: String
    public var parents: [String]
    public var refs: [String]
    public var subject: String
    public var authorName: String
    public var authorEmail: String
    public var relativeDate: String
    public var commitDate: Date?

    public init(
        hash: String,
        shortHash: String,
        parents: [String],
        refs: [String],
        subject: String,
        authorName: String,
        authorEmail: String,
        relativeDate: String,
        commitDate: Date? = nil
    ) {
        self.hash = hash
        self.shortHash = shortHash
        self.parents = parents
        self.refs = refs
        self.subject = subject
        self.authorName = authorName
        self.authorEmail = authorEmail
        self.relativeDate = relativeDate
        self.commitDate = commitDate
    }

    public var id: String {
        hash
    }
}

public enum GitClientError: LocalizedError, Sendable {
    case commandFailed(command: String, message: String)
    case emptyBranchName
    case emptyCommitMessage

    public var errorDescription: String? {
        switch self {
        case let .commandFailed(command, message):
            "Git could not run `\(command)`: \(message)"
        case .emptyBranchName:
            "Branch name cannot be empty."
        case .emptyCommitMessage:
            "Commit message cannot be empty."
        }
    }
}
