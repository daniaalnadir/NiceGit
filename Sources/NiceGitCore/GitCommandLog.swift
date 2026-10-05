import Foundation

/// Records the Git commands NiceGit runs and how long each took, while a measurement is
/// running. Off by default; when off, each command pays only for one locked flag check.
public final class GitCommandLog: @unchecked Sendable {
    public struct Entry: Sendable {
        public let arguments: [String]
        public let seconds: Double
        /// When the command started, in seconds after recording began.
        public let start: Double
    }

    public static let shared = GitCommandLog()
    private let lock = NSLock()
    private var recording = false
    private var entries: [Entry] = []
    private var began = ContinuousClock.now

    public func start() {
        lock.lock(); defer { lock.unlock() }
        entries = []
        began = ContinuousClock.now
        recording = true
    }

    @discardableResult
    public func stop() -> [Entry] {
        lock.lock(); defer { lock.unlock() }
        recording = false
        return entries
    }

    var isRecording: Bool {
        lock.lock(); defer { lock.unlock() }
        return recording
    }

    func record(_ arguments: [String], seconds: Double, started: ContinuousClock.Instant) {
        lock.lock(); defer { lock.unlock() }
        let offset = began.duration(to: started)
        if recording { entries.append(Entry(arguments: arguments, seconds: seconds, start: Double(offset.components.seconds) + Double(offset.components.attoseconds) / 1e18)) }
    }
}
