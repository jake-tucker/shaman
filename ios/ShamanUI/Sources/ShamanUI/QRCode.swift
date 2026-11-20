#if DEBUG
  import CoreImage
  import CoreImage.CIFilterBuiltins
  import UIKit

  /// A QR code drawn as a Serving Server draws an Invite's: holding the text alone, at the lowest error-correction
  /// level, dark on light, with a quiet margin. What the fake scanner shows, and what the package's tests read back as
  /// a camera would.
  enum QRCode {
    /// `text` as a QR code, `scale` points to a module, or nothing where it is too long for one.
    static func image(of text: String, scale: CGFloat = 8) -> UIImage? {
      let generator = CIFilter.qrCodeGenerator()
      generator.message = Data(text.utf8)
      generator.correctionLevel = "L"
      guard let code = generator.outputImage else { return nil }
      // A module of white is added around whatever margin Core Image draws.
      let margined = code.composited(over: CIImage(color: .white).cropped(to: code.extent.insetBy(dx: -1, dy: -1)))
      let scaled = margined.transformed(by: .init(translationX: 1, y: 1))
        .transformed(by: .init(scaleX: scale, y: scale))
      guard let drawn = CIContext().createCGImage(scaled, from: scaled.extent) else { return nil }
      return UIImage(cgImage: drawn)
    }
  }
#endif
