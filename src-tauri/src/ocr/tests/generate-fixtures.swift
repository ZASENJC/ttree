// Deterministic OCR fixtures generated with macOS system fonts, entirely offline.
import AppKit

let directory = URL(fileURLWithPath: CommandLine.arguments[1], isDirectory: true)

func writeFixture(
    _ name: String, text: String, width: Int = 440, height: Int = 80,
    fontSize: CGFloat = 32, bits: Int = 8, grayscale: Bool = false,
    transparent: Bool = false
) throws {
    guard let bitmap = NSBitmapImageRep(
        bitmapDataPlanes: nil, pixelsWide: width, pixelsHigh: height,
        bitsPerSample: bits, samplesPerPixel: grayscale ? 1 : 4,
        hasAlpha: !grayscale, isPlanar: false,
        colorSpaceName: grayscale ? .deviceWhite : .deviceRGB,
        bytesPerRow: 0, bitsPerPixel: 0
    ), let context = NSGraphicsContext(bitmapImageRep: bitmap) else {
        fatalError("Could not create fixture bitmap: \(name)")
    }
    NSGraphicsContext.saveGraphicsState()
    NSGraphicsContext.current = context
    context.cgContext.setFillColor((transparent ? NSColor.clear : NSColor.white).cgColor)
    context.cgContext.setBlendMode(.copy)
    context.cgContext.fill(CGRect(x: 0, y: 0, width: CGFloat(width), height: CGFloat(height)))
    context.cgContext.setBlendMode(.normal)
    (text as NSString).draw(
        at: NSPoint(x: 4, y: 2),
        withAttributes: [.font: NSFont.systemFont(ofSize: fontSize),
                         .foregroundColor: NSColor.black]
    )
    context.flushGraphics()
    NSGraphicsContext.restoreGraphicsState()
    guard let png = bitmap.representation(using: .png, properties: [:]) else {
        fatalError("Could not encode fixture PNG: \(name)")
    }
    try png.write(to: directory.appendingPathComponent(name + ".png"))
}

try writeFixture("english", text: "Hello OCR 123")
try writeFixture("single-line", text: "Hello OCR 123", width: 160, height: 22, fontSize: 14)
try writeFixture("grayscale", text: "Hello OCR 123", grayscale: true)
try writeFixture("grayscale-16bit", text: "Hello OCR 123", bits: 16, grayscale: true)
try writeFixture("transparent", text: "Hello OCR 123", transparent: true)
try writeFixture("chinese", text: "你好世界 Hello OCR")
try writeFixture("japanese", text: "日本語テキスト")
