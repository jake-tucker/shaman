import SwiftUI

extension View {
  /// Stands what `foot` draws of `held` at the screen's foot, on its own ground, with the composer's field at its end,
  /// and nothing while nothing is `held`.
  func composerFoot<Held>(_ held: Held?, @ViewBuilder foot: @escaping (Held) -> some View) -> some View {
    modifier(ComposerFoot(held: held, foot: foot))
  }
}

/// The foot of a screen whose composer stands there, the Session's and the new-session screen's.
///
/// Its field stands where Messages stands its own: as far above the display's bottom edge as the width class says, on
/// every iPhone and in every pose. Nothing else is asked, the home indicator and its band least of all: the screen
/// ignores the container's bottom safe area whole, its list and its foot together. Ignored by the foot alone, it leaves
/// a strip as tall above the foot. With no foot the list runs to the edge, and UIKit keeps its last row clear of it.
///
/// Over a keyboard the field stands by the keyboard instead. The room the keyboard takes is part of how far above the
/// edge the field already is, so the foot's own padding gives way by as much as it comes to, down to what it keeps over
/// one. However the keyboard comes and goes, the field is then never lower than it rests nor nearer the keyboard than
/// that. It is one view throughout, so it keeps its focus.
///
/// It keeps no sides: what the foot draws stands across its column by `readingSides`, under the conversation's words.
private struct ComposerFoot<Held, Foot: View>: ViewModifier {
  let held: Held?
  let foot: (Held) -> Foot
  @Environment(\.horizontalSizeClass) private var width
  /// How much of the screen's foot the keyboard covers.
  @State private var keyboard: CGFloat = 0

  /// How far above the display's bottom edge Messages stands its field, the keyboard down.
  private var stands: CGFloat { width == .regular ? 22 : 28 }
  /// How far above the keyboard the field stands, and the ground above what the foot holds.
  private let clear: CGFloat = 8

  func body(content: Content) -> some View {
    content
      .safeAreaInset(edge: .bottom, spacing: 0) {
        if let held {
          foot(held)
            .padding(.top, clear)
            .padding(.bottom, max(clear, stands - keyboard))
            .background(.bar)
        }
      }
      .onGeometryChange(for: CGFloat.self) {
        $0.safeAreaInsets.bottom
      } action: {
        // Past the container's safe area, what is left is the keyboard's.
        keyboard = $0
      }
      .ignoresSafeArea(.container, edges: .bottom)
  }
}
