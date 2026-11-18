import CoreText
import SwiftUI

/// The Symbols Nerd Font every Icon is drawn from, bundled whole, registered with CoreText for this process the first
/// time a glyph is drawn, and never before.
enum IconFont {
  /// The font's PostScript name, read from the bundled file as it is registered; none where the file could not be
  /// read or registered.
  static let name: String? = register()

  /// The bundled font file.
  static var file: URL? {
    Bundle.module.url(forResource: "SymbolsNerdFont-Regular", withExtension: "ttf", subdirectory: "Fonts")
  }

  private static func register() -> String? {
    guard let file,
      let descriptors = CTFontManagerCreateFontDescriptorsFromURL(file as CFURL) as? [CTFontDescriptor],
      let name = descriptors.first.flatMap({ CTFontDescriptorCopyAttribute($0, kCTFontNameAttribute) as? String })
    else {
      return nil
    }
    var error: Unmanaged<CFError>?
    if !CTFontManagerRegisterFontsForURL(file as CFURL, .process, &error) {
      let code = error.map { CFErrorGetCode($0.takeRetainedValue()) }
      guard code == CTFontManagerError.alreadyRegistered.rawValue else {
        return nil
      }
    }
    return name
  }

  /// The font at the size of `style`, growing with Dynamic Type as text of that style does.
  static func font(_ style: Font.TextStyle) -> Font {
    guard let name else {
      return .system(style)
    }
    return .custom(name, size: UIFont.preferredFont(forTextStyle: style.uiKit).pointSize, relativeTo: style)
  }
}

/// A glyph the core resolved from an Icon, drawn in the bundled font.
struct Glyph: View {
  let glyph: String
  var style: Font.TextStyle = .body

  var body: some View {
    Text(verbatim: glyph)
      .font(IconFont.font(style))
      .accessibilityHidden(true)
  }
}

extension Font.TextStyle {
  /// The UIKit style of the same name, whose size a custom font is drawn at.
  fileprivate var uiKit: UIFont.TextStyle {
    switch self {
    case .largeTitle: .largeTitle
    case .title: .title1
    case .title2: .title2
    case .title3: .title3
    case .headline: .headline
    case .subheadline: .subheadline
    case .callout: .callout
    case .footnote: .footnote
    case .caption: .caption1
    case .caption2: .caption2
    default: .body
    }
  }
}
