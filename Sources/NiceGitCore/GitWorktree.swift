import Foundation

public struct GitWorktree: Identifiable, Equatable, Sendable {
    public let path: String
    public let branch: String?
    public let isBare: Bool
    public let isLocked: Bool
    public let isPrunable: Bool
    public var id: String { path }

    public static func parse(_ output: String) -> [GitWorktree] {
        var result: [GitWorktree] = []
        var path: String?
        var branch: String?
        var bare = false
        var locked = false
        var prunable = false
        for field in output.components(separatedBy: "\0") {
            if field.isEmpty {
                if let path {
                    result.append(Self(path: path, branch: branch, isBare: bare, isLocked: locked, isPrunable: prunable))
                }
                path = nil; branch = nil; bare = false; locked = false; prunable = false
            } else if field.hasPrefix("worktree ") {
                path = String(field.dropFirst(9))
            } else if field.hasPrefix("branch refs/heads/") {
                branch = String(field.dropFirst(18))
            } else if field == "bare" { bare = true }
            else if field == "locked" || field.hasPrefix("locked ") { locked = true }
            else if field == "prunable" || field.hasPrefix("prunable ") { prunable = true }
        }
        return result
    }
}
