import Foundation

public enum GitHubItemKind: String, Sendable {
    case pullRequest = "pr"
    case issue
}

public enum GitHubItemState: String, CaseIterable, Sendable {
    case open, closed, merged, all
}

public struct GitHubRepository: Equatable, Sendable {
    public let owner: String
    public let name: String
    public var slug: String { owner + "/" + name }

    public func commitURL(hash: String) throws -> URL {
        guard [40, 64].contains(hash.count), hash.allSatisfy({ $0.isASCII && $0.isHexDigit }),
              let url = URL(string: "https://github.com/\(slug)/commit/\(hash)") else {
            throw GitHubClientError.message("A full commit hash is required to copy a commit link.")
        }
        return url
    }

    public init(remoteAddress: String) throws {
        let address = remoteAddress.hasPrefix("git@github.com:")
            ? "https://github.com/" + remoteAddress.dropFirst("git@github.com:".count)
            : remoteAddress
        guard let url = URLComponents(string: address),
              ["https", "ssh"].contains(url.scheme), url.host?.lowercased() == "github.com",
              url.password == nil, url.port == nil, url.query == nil, url.fragment == nil else {
            throw GitHubClientError.message("This remote is not a supported github.com repository.")
        }
        var parts = url.path.split(separator: "/", omittingEmptySubsequences: true).map(String.init)
        guard parts.count == 2 else { throw GitHubClientError.message("The GitHub remote must identify an owner and repository.") }
        if parts[1].hasSuffix(".git") { parts[1].removeLast(4) }
        let allowed = CharacterSet(charactersIn: "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789-_.")
        guard parts.allSatisfy({ !$0.isEmpty && $0 != "." && $0 != ".." && $0.unicodeScalars.allSatisfy(allowed.contains) }) else {
            throw GitHubClientError.message("The GitHub repository address is invalid.")
        }
        owner = parts[0]
        name = parts[1]
    }
}

public struct GitHubItem: Decodable, Identifiable, Equatable, Sendable {
    public struct Author: Decodable, Equatable, Sendable { public let login: String }
    public let number: Int
    public let title: String
    public let url: URL
    public let author: Author?
    public let isDraft: Bool?
    public let state: String?
    public var id: Int { number }

    public func matches(_ query: String) -> Bool {
        let query = query.trimmingCharacters(in: .whitespacesAndNewlines)
        return query.isEmpty || title.localizedCaseInsensitiveContains(query)
            || (author?.login.localizedCaseInsensitiveContains(query) ?? false)
            || String(number) == (query.hasPrefix("#") ? String(query.dropFirst()) : query)
    }
}

public enum GitHubClientError: LocalizedError {
    case message(String)
    public var errorDescription: String? { if case .message(let text) = self { return text }; return nil }
}

public struct GitHubClient: Sendable {
    public init() {}

    public static func decode(_ data: Data, repository: GitHubRepository, kind: GitHubItemKind) throws -> [GitHubItem] {
        let items = try JSONDecoder().decode([GitHubItem].self, from: data)
        let segment = kind == .pullRequest ? "pull" : "issues"
        guard Set(items.map(\.number)).count == items.count,
              items.allSatisfy({ item in
                  item.number > 0 && item.url.scheme == "https" && item.url.host == "github.com"
                    && item.url.user == nil && item.url.password == nil && item.url.port == nil
                    && item.url.query == nil && item.url.fragment == nil
                    && item.url.path.lowercased() == "/\(repository.slug)/\(segment)/\(item.number)".lowercased()
              }) else { throw GitHubClientError.message("GitHub returned unexpected item links.") }
        return items
    }

    public func load(kind: GitHubItemKind, remote: String, in directory: URL, limit: Int = 100, state: GitHubItemState = .open, control: GitCommandControl) throws -> [GitHubItem] {
        guard kind == .pullRequest || state != .merged else {
            throw GitHubClientError.message("Merged applies to pull requests, not issues.")
        }
        let address = try GitClient(control: control, commandTimeout: 30).remoteAddress(name: remote, in: directory)
        let repository = try GitHubRepository(remoteAddress: address)
        guard let executable = ["/opt/homebrew/bin/gh", "/usr/local/bin/gh", "/usr/bin/gh"].first(where: FileManager.default.isExecutableFile(atPath:)) else {
            throw GitHubClientError.message("GitHub CLI is not installed. Install it and sign in with gh auth login, then retry.")
        }
        let temporary = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: temporary, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: temporary) }
        let output = temporary.appendingPathComponent("output")
        let errors = temporary.appendingPathComponent("errors")
        FileManager.default.createFile(atPath: output.path, contents: nil)
        FileManager.default.createFile(atPath: errors.path, contents: nil)
        let stdout = try FileHandle(forWritingTo: output)
        let stderr = try FileHandle(forWritingTo: errors)
        defer { try? stdout.close(); try? stderr.close() }
        let process = Process()
        process.executableURL = URL(fileURLWithPath: executable)
        process.currentDirectoryURL = directory
        process.arguments = [kind.rawValue, "list", "--repo", "github.com/" + repository.slug, "--state", state.rawValue, "--limit", String(max(1, min(limit, 1000))), "--json", "number,title,url,author,state" + (kind == .pullRequest ? ",isDraft" : "")]
        var environment = GitClient.repositoryEnvironment(ProcessInfo.processInfo.environment)
        environment.removeValue(forKey: "GH_REPO")
        environment.removeValue(forKey: "GH_HOST")
        environment["GH_PROMPT_DISABLED"] = "1"
        environment["GH_NO_UPDATE_NOTIFIER"] = "1"
        environment["NO_COLOR"] = "1"
        process.environment = environment
        process.standardInput = FileHandle.nullDevice
        process.standardOutput = stdout
        process.standardError = stderr
        guard !control.isCancelled else { throw GitHubClientError.message("GitHub request cancelled.") }
        try process.run()
        do { try GitProcessWaiter.wait(process, control: control, timeout: 30) }
        catch { throw GitHubClientError.message(control.isCancelled ? "GitHub request cancelled." : "GitHub request timed out. Retry when the connection is available.") }
        guard process.terminationStatus == 0 else {
            let message = String(data: try Data(contentsOf: errors), encoding: .utf8) ?? "GitHub request failed."
            throw GitHubClientError.message(String(message.prefix(2000)))
        }
        return try Self.decode(Data(contentsOf: output), repository: repository, kind: kind)
    }
}
