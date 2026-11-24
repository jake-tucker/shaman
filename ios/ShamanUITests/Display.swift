import XCTest

/// What the display the app is on shows, whichever display that is and however it is held.
@MainActor enum Display {
  /// A picture of the display the app is on, standing upright as a reader sees it, whose pixels may be read.
  ///
  /// A Duo has two, of which the one not in use is all black: the outer, which is the main screen, while it is open,
  /// and the inner while it is closed. Both are black for some seconds after it boots.
  static func picture() -> UIImage {
    let main = XCUIScreen.main.screenshot().image
    guard isBlack(main), let inner = XCUIScreen.screens.dropFirst().first?.screenshot().image else {
      return upright(main)
    }
    // XCUITest gives the inner display's pixels the same orientation in either portrait, though in the one a posed
    // run reaches, turned right once, they stand on their head. It tells the test of neither.
    guard inner.imageOrientation == .up else { return upright(inner) }
    return upright(UIImage(cgImage: inner.cgImage!, scale: inner.scale, orientation: .down))
  }

  /// `image` drawn again where its pixels do not lie as it is seen.
  private static func upright(_ image: UIImage) -> UIImage {
    guard image.imageOrientation != .up else { return image }
    let format = UIGraphicsImageRendererFormat()
    format.scale = image.scale
    format.opaque = true
    return UIGraphicsImageRenderer(size: image.size, format: format).image { _ in image.draw(at: .zero) }
  }

  /// Whether nothing at all is lit in `image`, which the status bar alone would be on a display in use.
  private static func isBlack(_ image: UIImage) -> Bool {
    let pixels = image.cgImage!
    let width = pixels.width / 8
    let height = pixels.height / 8
    let context = CGContext(
      data: nil, width: width, height: height, bitsPerComponent: 8, bytesPerRow: width,
      space: CGColorSpaceCreateDeviceGray(), bitmapInfo: CGImageAlphaInfo.none.rawValue)!
    context.draw(pixels, in: CGRect(x: 0, y: 0, width: width, height: height))
    return UnsafeRawBufferPointer(start: context.data!, count: width * height).allSatisfy { $0 == 0 }
  }
}

extension XCTestCase {
  /// Keeps `picture`, or one of the display the app is on now, with the test's results.
  @MainActor
  func attach(_ name: String, showing picture: UIImage? = nil) {
    let attachment = XCTAttachment(image: picture ?? Display.picture())
    attachment.name = name
    attachment.lifetime = .keepAlways
    add(attachment)
  }
}

extension XCUIApplication {
  /// The app's window. A drag begins and ends at points of it, and a swipe is made over it, as the app's own frame is
  /// the display's, which on an open Duo lies on its side.
  var window: XCUIElement { windows.firstMatch }

  /// The point `across` and `down` the window, as parts of its width and height.
  func point(across: Double, down: Double) -> XCUICoordinate {
    window.coordinate(withNormalizedOffset: CGVector(dx: across, dy: down))
  }
}
