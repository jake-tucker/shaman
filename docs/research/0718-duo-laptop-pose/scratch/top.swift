import AppKit
// top <image>...  Prints, in points of a 951-point display, the first row past 30% down from which the middle column stays white.
for p in CommandLine.arguments.dropFirst() {
  guard let d = try? Data(contentsOf: URL(fileURLWithPath: p)), let rep = NSBitmapImageRep(data: d) else { continue }
  let w = rep.pixelsWide, h = rep.pixelsHigh, s = Double(h) / 951
  func white(_ y: Int) -> Bool { let c = rep.colorAt(x: w / 2, y: y)!; return c.redComponent > 0.98 && c.greenComponent > 0.98 && c.blueComponent > 0.98 }
  var top = -1
  for y in Int(Double(h) * 0.3)..<h - 8 where (0..<8).allSatisfy({ white(y + $0) }) { top = y; break }
  print((p as NSString).lastPathComponent, w, h, "top", (Double(top) / s * 10).rounded() / 10)
}
