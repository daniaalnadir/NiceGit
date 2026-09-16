import Foundation
import Testing
import Darwin
@testable import NiceGitCore

@Test func cancellationStopsChildProcessesInIsolatedGroup() throws {
    let process = Process()
    process.executableURL = URL(fileURLWithPath: "/bin/sh")
    process.arguments = ["-c", "sleep 20 >/dev/null 2>&1 & echo $!; wait"]
    let output = Pipe()
    process.standardOutput = output
    try process.run()
    let data = output.fileHandleForReading.availableData
    let child = try #require(Int32(String(decoding: data, as: UTF8.self).trimmingCharacters(in: .whitespacesAndNewlines)))
    defer { if kill(child, 0) == 0 { kill(child, SIGKILL) } }
    #expect(getpgid(process.processIdentifier) == process.processIdentifier)
    let control = GitCommandControl()
    control.cancel()
    #expect(throws: (any Error).self) { try GitProcessWaiter.wait(process, control: control, timeout: 30) }
    #expect(!process.isRunning)
    let deadline = Date().addingTimeInterval(2)
    while kill(child, 0) == 0 && Date() < deadline { Thread.sleep(forTimeInterval: 0.02) }
    #expect(kill(child, 0) != 0)
}

@Test func commandTimeoutStopsRunningProcess() throws {
    let process = Process()
    process.executableURL = URL(fileURLWithPath: "/bin/sleep")
    process.arguments = ["20"]
    try process.run()
    let start = Date()
    #expect(throws: (any Error).self) { try GitProcessWaiter.wait(process, control: nil, timeout: 0.05) }
    #expect(!process.isRunning)
    #expect(Date().timeIntervalSince(start) < 3)
}

@Test func cancellationStopsRunningProcess() throws {
    let process = Process()
    process.executableURL = URL(fileURLWithPath: "/bin/sleep")
    process.arguments = ["20"]
    try process.run()
    let control = GitCommandControl()
    control.cancel()
    #expect(throws: (any Error).self) { try GitProcessWaiter.wait(process, control: control, timeout: 30) }
    #expect(!process.isRunning)
}
