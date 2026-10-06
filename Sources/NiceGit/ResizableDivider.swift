import AppKit
import SwiftUI

/// A one-point divider that resizes the column to its left: drag it, double-click it to restore
/// the default width, or adjust it with VoiceOver. The grab area is an AppKit view a few points
/// wider than the line; AppKit hit-tests it by its own frame and tracks the drag itself, as
/// split-view dividers do, with the resize cursor shown through cursor rectangles.
struct ResizableDivider: View {
    @Binding var width: Double
    let range: ClosedRange<Double>
    let defaultWidth: Double
    var label = "Panel width"
    @State private var dragStartWidth: Double?

    var body: some View {
        Rectangle()
            .fill(dragStartWidth != nil ? Color.accentColor.opacity(0.6) : AppPalette.line)
            .frame(width: 1)
            .frame(maxHeight: .infinity)
            .overlay {
                ResizeHandle(
                    onBegin: { dragStartWidth = width },
                    onDrag: { offset in
                        let start = dragStartWidth ?? width
                        width = min(max(start + offset, range.lowerBound), range.upperBound)
                    },
                    onEnd: { dragStartWidth = nil },
                    onReset: { width = defaultWidth })
                    .frame(width: 9)
                    .help("Drag to resize; double-click to restore the default width")
            }
            .zIndex(1)
            .accessibilityElement()
            .accessibilityLabel(label)
            .accessibilityValue("\(Int(width)) points")
            .accessibilityAdjustableAction { direction in
                switch direction {
                case .increment: width = min(width + 20, range.upperBound)
                case .decrement: width = max(width - 20, range.lowerBound)
                @unknown default: break
                }
            }
    }
}

private struct ResizeHandle: NSViewRepresentable {
    let onBegin: () -> Void
    let onDrag: (Double) -> Void
    let onEnd: () -> Void
    let onReset: () -> Void

    func makeNSView(context: Context) -> ResizeHandleView { ResizeHandleView() }

    func updateNSView(_ view: ResizeHandleView, context: Context) {
        view.onBegin = onBegin
        view.onDrag = onDrag
        view.onEnd = onEnd
        view.onReset = onReset
    }
}

final class ResizeHandleView: NSView {
    var onBegin: () -> Void = {}
    var onDrag: (Double) -> Void = { _ in }
    var onEnd: () -> Void = {}
    var onReset: () -> Void = {}

    override func resetCursorRects() { addCursorRect(bounds, cursor: .resizeLeftRight) }
    override func acceptsFirstMouse(for event: NSEvent?) -> Bool { true }

    override func mouseDown(with event: NSEvent) {
        if event.clickCount == 2 {
            onReset()
            return
        }
        let start = event.locationInWindow.x
        onBegin()
        // Follow the pointer until the button is released, as AppKit split views do.
        while let next = window?.nextEvent(matching: [.leftMouseDragged, .leftMouseUp]) {
            onDrag(Double(next.locationInWindow.x - start))
            if next.type == .leftMouseUp { break }
        }
        onEnd()
    }
}
