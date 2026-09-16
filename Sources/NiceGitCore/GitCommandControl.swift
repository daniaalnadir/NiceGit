import Foundation
import Darwin

public final class GitCommandControl: @unchecked Sendable {
    private let lock = NSLock()
    private var cancelled = false

    public init() {}

    public func cancel() {
        lock.lock()
        cancelled = true
        lock.unlock()
    }

    public var isCancelled: Bool {
        lock.lock()
        defer { lock.unlock() }
        return cancelled
    }
}

enum GitProcessWaiter {
    static func wait(_ process: Process, control: GitCommandControl?, timeout: TimeInterval) throws {
        let deadline = ProcessInfo.processInfo.systemUptime + timeout
        let pid = process.processIdentifier
        // Only signal a group led by our own child, never the app's process group.
        let isolatedGroup = getpgid(pid) == pid
        while process.isRunning {
            if control?.isCancelled == true || ProcessInfo.processInfo.systemUptime >= deadline {
                let cancelled = control?.isCancelled == true
                if isolatedGroup { kill(-pid, SIGTERM) }
                else { process.terminate() }
                let grace = ProcessInfo.processInfo.systemUptime + 1
                while (process.isRunning || (isolatedGroup && kill(-pid, 0) == 0)) && ProcessInfo.processInfo.systemUptime < grace {
                    Thread.sleep(forTimeInterval: 0.02)
                }
                if isolatedGroup { kill(-pid, SIGKILL) }
                else if process.isRunning { kill(pid, SIGKILL) }
                process.waitUntilExit()
                throw GitClientError.commandFailed(command: "git", message: cancelled ? "Operation cancelled. Check the repository state before retrying." : "Git exceeded the command time limit. Refresh the repository before retrying.")
            }
            Thread.sleep(forTimeInterval: 0.01)
        }
        process.waitUntilExit()
    }
}
