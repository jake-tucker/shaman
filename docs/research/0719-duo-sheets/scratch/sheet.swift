import AppKit

// sheet <out.png> <columns> <cell width> <caption>=<image>...   Tiles images in a grid, each under its caption (its
// file name where none is given).
let args = CommandLine.arguments
let out = args[1]
let columns = Int(args[2])!
let cellWidth = CGFloat(Int(args[3])!)
let paths = Array(args[4...])
let images = paths.compactMap { arg -> (String, NSImage)? in
  let split = arg.split(separator: "=", maxSplits: 1).map(String.init)
  let p = split.last!
  if split.count == 2, let i = NSImage(contentsOfFile: p), let rep = i.representations.first {
    i.size = NSSize(width: rep.pixelsWide, height: rep.pixelsHigh)
    return (split[0], i)
  }
  guard let i = NSImage(contentsOfFile: p), let rep = i.representations.first else { return nil }
  i.size = NSSize(width: rep.pixelsWide, height: rep.pixelsHigh)
  return ((p as NSString).lastPathComponent.replacingOccurrences(of: ".small.png", with: "").replacingOccurrences(of: ".png", with: ""), i)
}
let label: CGFloat = 24
let gap: CGFloat = 8
let heights = images.map { cellWidth * $0.1.size.height / $0.1.size.width }
let rows = Int((Double(images.count) / Double(columns)).rounded(.up))
var rowHeights: [CGFloat] = []
for r in 0..<rows {
  let slice = heights[(r * columns)..<min((r + 1) * columns, heights.count)]
  rowHeights.append((slice.max() ?? 0) + label + gap)
}
let width = CGFloat(columns) * (cellWidth + gap) + gap
let height = rowHeights.reduce(0, +) + gap
let rep = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: Int(width), pixelsHigh: Int(height), bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0)!
NSGraphicsContext.current = NSGraphicsContext(bitmapImageRep: rep)
NSColor(white: 0.35, alpha: 1).setFill()
NSRect(x: 0, y: 0, width: width, height: height).fill()
var y = height - gap
for r in 0..<rows {
  for c in 0..<columns {
    let i = r * columns + c
    guard i < images.count else { break }
    let x = gap + CGFloat(c) * (cellWidth + gap)
    let h = heights[i]
    (images[i].0 as NSString).draw(at: NSPoint(x: x, y: y - label + 3), withAttributes: [.font: NSFont.boldSystemFont(ofSize: 15), .foregroundColor: NSColor.white])
    images[i].1.draw(in: NSRect(x: x, y: y - label - h, width: cellWidth, height: h))
  }
  y -= rowHeights[r]
}
try! rep.representation(using: .png, properties: [:])!.write(to: URL(fileURLWithPath: out))
