import AppKit
import Foundation
@testable import NiceGit
import NiceGitCore
import SwiftUI
import Testing

/// Drives the real window content with synthetic mouse and keyboard events inside the test
/// process, saving a screenshot after each step. It runs only when asked, for example:
///
///     DRIVE_REPO=/path/to/repo DRIVE_OUT=/tmp/shot DRIVE_STEPS="click:720:408;down;esc" \
///         swift test --filter driveNiceGitInteractively
///
/// Steps: click:x:y, cmdclick:x:y, dblclick:x:y, drag:x:y:toX (points from the top-left),
/// set:defaultsKey:number, widths, down, up, esc, return,
/// type:text, and palette. Sheets are captured too, but typing into them is not reliable.
/// A drag that AppKit tracks ends the test process when it finishes; set DRIVE_TRACE_EXIT to
/// print the stored panel widths as it exits, and run one drag per invocation.
@MainActor final class Driver {
    let model: AppModel
    let window: NSWindow
    let host: NSHostingView<AnyView>
    let out: String
    var step = 0

    init(repo: URL, out: String, defaults: UserDefaults) {
        model = AppModel(defaults: defaults)
        host = NSHostingView(rootView: AnyView(ContentView().environmentObject(model)))
        // DRIVE_SIZE=WIDTHxHEIGHT changes the window size from 1440x900.
        let size = (ProcessInfo.processInfo.environment["DRIVE_SIZE"] ?? "1440x900").split(separator: "x").compactMap { Double($0) }
        window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: size.first ?? 1440, height: size.last ?? 900), styleMask: [.titled, .resizable], backing: .buffered, defer: false)
        window.appearance = NSAppearance(named: .darkAqua)
        window.contentView = host
        self.out = out
        _ = NSApplication.shared
        window.orderFrontRegardless()
        window.makeKey()
    }

    func settle(_ seconds: Double = 1) async throws {
        let deadline = Date().addingTimeInterval(15)
        try await Task.sleep(for: .seconds(seconds))
        while model.isLoading && Date() < deadline { try await Task.sleep(for: .milliseconds(50)) }
        try await Task.sleep(for: .milliseconds(300))
        host.layoutSubtreeIfNeeded()
    }

    func shot(_ name: String) throws {
        step += 1
        let rep = try #require(host.bitmapImageRepForCachingDisplay(in: host.bounds))
        host.cacheDisplay(in: host.bounds, to: rep)
        try rep.representation(using: .png, properties: [:])!.write(to: URL(fileURLWithPath: "\(out)-\(String(format: "%02d", step))-\(name).png"))
        // Sheets open in their own windows; capture the frontmost one as well.
        if let sheet = window.attachedSheet ?? NSApplication.shared.windows.last(where: { $0 !== window && $0.isVisible && $0.contentView != nil }),
           let view = sheet.contentView, view.bounds.width > 0,
           let sheetRep = view.bitmapImageRepForCachingDisplay(in: view.bounds) {
            view.cacheDisplay(in: view.bounds, to: sheetRep)
            try sheetRep.representation(using: .png, properties: [:])!.write(to: URL(fileURLWithPath: "\(out)-\(String(format: "%02d", step))-\(name)-sheet.png"))
        }
    }

    /// Sends keys to the sheet when one is open, as the key window would.
    var target: NSWindow { window.attachedSheet ?? window }

    /// Clicks at a point measured from the top-left of the content, like a screenshot.
    func click(_ x: CGFloat, _ y: CGFloat, modifiers: NSEvent.ModifierFlags = []) {
        let point = NSPoint(x: x, y: host.bounds.height - y)
        for type in [NSEvent.EventType.leftMouseDown, .leftMouseUp] {
            let event = NSEvent.mouseEvent(with: type, location: point, modifierFlags: modifiers, timestamp: ProcessInfo.processInfo.systemUptime,
                                           windowNumber: window.windowNumber, context: nil, eventNumber: 0, clickCount: 1, pressure: 1)!
            window.sendEvent(event)
        }
    }

    /// Presses at one point, drags horizontally to another, and releases. The movement is queued
    /// first, so a control that tracks the drag itself (as dividers do) reads it from the queue.
    func drag(from start: (CGFloat, CGFloat), toX end: CGFloat) async throws {
        let y = host.bounds.height - start.1
        let steps = 10
        func event(_ type: NSEvent.EventType, _ x: CGFloat) -> NSEvent {
            NSEvent.mouseEvent(with: type, location: NSPoint(x: x, y: y), modifierFlags: [], timestamp: ProcessInfo.processInfo.systemUptime,
                               windowNumber: window.windowNumber, context: nil, eventNumber: 0, clickCount: 1, pressure: 1)!
        }
        for step in 1...steps {
            NSApplication.shared.postEvent(event(.leftMouseDragged, start.0 + (end - start.0) * CGFloat(step) / CGFloat(steps)), atStart: false)
        }
        NSApplication.shared.postEvent(event(.leftMouseUp, end), atStart: false)
        window.sendEvent(event(.leftMouseDown, start.0))
    }

    func key(_ code: UInt16, characters: String, modifiers: NSEvent.ModifierFlags = []) {
        for type in [NSEvent.EventType.keyDown, .keyUp] {
            let event = NSEvent.keyEvent(with: type, location: .zero, modifierFlags: modifiers, timestamp: ProcessInfo.processInfo.systemUptime,
                                         windowNumber: target.windowNumber, context: nil, characters: characters,
                                         charactersIgnoringModifiers: characters, isARepeat: false, keyCode: code)!
            target.sendEvent(event)
        }
    }
}

@Test @MainActor func driveNiceGitInteractively() async throws {
    let env = ProcessInfo.processInfo.environment
    guard let out = env["DRIVE_OUT"], let repo = env["DRIVE_REPO"] else { return }
    let suite = "NiceGitDrive-" + UUID().uuidString
    let defaults = try #require(UserDefaults(suiteName: suite))
    defer { defaults.removePersistentDomain(forName: suite) }
    if env["DRIVE_TRACE_EXIT"] != nil {
        // AppKit's mouse-drag tracking stops the main run loop when a drag ends, which ends a
        // test process; report what the drag changed as it exits.
        atexit {
            print("EXIT> widths:", UserDefaults.standard.double(forKey: "NiceGit.repositoryTabsWidth"),
                  UserDefaults.standard.double(forKey: "NiceGit.referenceSidebarWidth"))
        }
    }
    let driver = Driver(repo: URL(fileURLWithPath: repo), out: out, defaults: defaults)
    driver.model.loadRepository(at: URL(fileURLWithPath: repo))
    try await driver.settle(2)
    try driver.shot("loaded")
    let steps = (env["DRIVE_STEPS"] ?? "").split(separator: ";")
    for step in steps {
        let parts = step.split(separator: ":").map(String.init)
        switch parts[0] {
        case "click": driver.click(CGFloat(Double(parts[1])!), CGFloat(Double(parts[2])!))
        case "cmdclick": driver.click(CGFloat(Double(parts[1])!), CGFloat(Double(parts[2])!), modifiers: .command)
        case "drag": try await driver.drag(from: (CGFloat(Double(parts[1])!), CGFloat(Double(parts[2])!)), toX: CGFloat(Double(parts[3])!))
        case "set": UserDefaults.standard.set(Double(parts[2])!, forKey: parts[1])
        case "dblclick":
            // Queue each release before its press, so a control that tracks the press itself
            // (as dividers do) finds the release waiting instead of blocking.
            let point = NSPoint(x: CGFloat(Double(parts[1])!), y: driver.host.bounds.height - CGFloat(Double(parts[2])!))
            func event(_ type: NSEvent.EventType, _ count: Int) -> NSEvent {
                NSEvent.mouseEvent(with: type, location: point, modifierFlags: [], timestamp: ProcessInfo.processInfo.systemUptime,
                                   windowNumber: driver.window.windowNumber, context: nil, eventNumber: 0, clickCount: count, pressure: 1)!
            }
            for count in 1...2 {
                NSApplication.shared.postEvent(event(.leftMouseUp, count), atStart: false)
                driver.window.sendEvent(event(.leftMouseDown, count))
            }
        case "widths":
            print("widths:", UserDefaults.standard.double(forKey: "NiceGit.repositoryTabsWidth"), UserDefaults.standard.double(forKey: "NiceGit.referenceSidebarWidth"))
        case "down": driver.key(125, characters: String(UnicodeScalar(NSDownArrowFunctionKey)!))
        case "up": driver.key(126, characters: String(UnicodeScalar(NSUpArrowFunctionKey)!))
        case "esc": driver.key(53, characters: "\u{1b}")
        case "type": for character in parts[1] { driver.key(0, characters: String(character)) }
        case "return": driver.key(36, characters: "\r")
        case "palette": driver.model.showingCommandPalette = true
        default: break
        }
        try await driver.settle(parts[0] != "drag" && parts.count > 3 ? Double(parts[3])! : 0.8)
        try driver.shot(parts[0])
    }
}
