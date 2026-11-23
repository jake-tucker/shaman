import AppKit

// crop <in.png> <out.png> <pixels>   Keeps the bottom <pixels> rows of an image, at its own resolution: `sips -c`
// crops nothing on this Mac.
let args = CommandLine.arguments
let rows = Int(args[3])!
guard let source = NSImage(contentsOfFile: args[1])?.cgImage(forProposedRect: nil, context: nil, hints: nil),
  let cut = source.cropping(to: CGRect(x: 0, y: source.height - rows, width: source.width, height: rows))
else { fatalError("cannot read \(args[1])") }
let rep = NSBitmapImageRep(cgImage: cut)
try! rep.representation(using: .png, properties: [:])!.write(to: URL(fileURLWithPath: args[2]))
