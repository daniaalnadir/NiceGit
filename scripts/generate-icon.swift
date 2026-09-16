import AppKit

let destination = URL(fileURLWithPath: CommandLine.arguments[1], isDirectory: true)
try FileManager.default.createDirectory(at: destination, withIntermediateDirectories: true)

func render(size: Int) throws -> Data {
    let bitmap = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: size, pixelsHigh: size,
                                  bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true,
                                  isPlanar: false, colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0)!
    NSGraphicsContext.saveGraphicsState()
    NSGraphicsContext.current = NSGraphicsContext(bitmapImageRep: bitmap)
    let context = NSGraphicsContext.current!.cgContext
    context.scaleBy(x: CGFloat(size) / 1024, y: CGFloat(size) / 1024)
    NSColor(calibratedWhite: 0.12, alpha: 1).setFill()
    NSBezierPath(roundedRect: NSRect(x: 64, y: 64, width: 896, height: 896), xRadius: 195, yRadius: 195).fill()
    let green = NSColor(calibratedRed: 0.35, green: 0.82, blue: 0.55, alpha: 1)
    green.setStroke()
    let trunk = NSBezierPath()
    trunk.lineWidth = 60
    trunk.lineCapStyle = .round
    trunk.move(to: NSPoint(x: 350, y: 285))
    trunk.line(to: NSPoint(x: 350, y: 750))
    trunk.stroke()
    let branch = NSBezierPath()
    branch.lineWidth = 60
    branch.lineCapStyle = .round
    branch.move(to: NSPoint(x: 350, y: 370))
    branch.curve(to: NSPoint(x: 690, y: 665), controlPoint1: NSPoint(x: 350, y: 590), controlPoint2: NSPoint(x: 690, y: 435))
    branch.stroke()
    for (x, y) in [(350.0, 285.0), (350.0, 750.0), (690.0, 665.0)] {
        green.setFill()
        NSBezierPath(ovalIn: NSRect(x: x - 88, y: y - 88, width: 176, height: 176)).fill()
        NSColor(calibratedWhite: 0.12, alpha: 1).setFill()
        NSBezierPath(ovalIn: NSRect(x: x - 36, y: y - 36, width: 72, height: 72)).fill()
    }
    NSGraphicsContext.restoreGraphicsState()
    return bitmap.representation(using: .png, properties: [:])!
}

for size in [16, 32, 128, 256, 512] {
    try render(size: size).write(to: destination.appendingPathComponent("icon_\(size)x\(size).png"))
    try render(size: size * 2).write(to: destination.appendingPathComponent("icon_\(size)x\(size)@2x.png"))
}
