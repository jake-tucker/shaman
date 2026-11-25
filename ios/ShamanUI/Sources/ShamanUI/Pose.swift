import SwiftUI

/// How a window stands: about the fold, where its device is sat like a laptop, and over a keyboard. It is read once,
/// above the columns, as the window's geometry changes, and handed down; no hosted screen asks for either, and nothing
/// asks on a clock. There is no pose but that one: in every other the layout is the size class's alone.
@MainActor @Observable
final class Pose {
  /// The fold, while the device is sat like a laptop.
  var fold: Fold?
  /// How much of the window's foot the system counts as a keyboard's, which is how far it lifts a screen's foot.
  private(set) var keyboard: CGFloat = 0
  /// How much of the window's bottom safe area, while a keyboard is up, is above what the system counts as its: none on
  /// an ordinary iPhone. Sat like a laptop that area reaches past the band kept for the fold, which a keyboard below the
  /// fold need not cover.
  private(set) var aboveKeyboard: CGFloat = 0
  /// Whether a keyboard is up: whether the window's bottom safe area is more than the window's own. Sat like a laptop,
  /// it takes the half below the fold.
  private(set) var keyboardIsUp = false
  /// The window's own bottom safe area, as it last was with nothing of a keyboard in it.
  @ObservationIgnored private var own: CGFloat = 0

  /// Takes what stands at the window's foot now: its bottom safe area, and how much of that is a `keyboard`'s.
  func take(_ foot: WindowFoot) {
    if foot.keyboard <= 0 {
      own = foot.safeArea
    }
    let up = foot.safeArea > own
    let above = up ? max(0, foot.safeArea - foot.keyboard) : 0
    // Each is drawn by apart, and the keyboard's goes a little way at a time.
    if keyboard != foot.keyboard { keyboard = foot.keyboard }
    if aboveKeyboard != above { aboveKeyboard = above }
    if keyboardIsUp != up { keyboardIsUp = up }
  }

  /// The fold, while what stands below it has that half.
  var foldWithHalf: Fold? {
    keyboardIsUp ? nil : fold
  }
}

/// A window's bottom safe area, and how much of it a keyboard covers, both from the window's bottom edge.
struct WindowFoot: Equatable {
  let safeArea: CGFloat
  let keyboard: CGFloat
}

/// The fold lying across a window, as the device sat like a laptop has it: the band the system keeps for it, with a
/// half above it and a half below.
struct Fold: Equatable {
  /// How far below the window's head the band begins, and ends.
  let top: CGFloat
  let bottom: CGFloat
  /// How tall the window is.
  let window: CGFloat

  /// How tall the band is.
  var band: CGFloat { bottom - top }
  /// How tall the half below is, from where the band begins.
  var below: CGFloat { max(0, window - top) }
}

extension Fold {
  /// The fold in a window `height` tall, among the regions the system reserves where a display divides: one that is
  /// active and wider than it is tall. Partly folded like a book, it is taller.
  init?(among regions: some Sequence<(frame: CGRect, isActive: Bool)>, height: CGFloat) {
    guard let across = regions.first(where: { $0.isActive && $0.frame.width > $0.frame.height }) else { return nil }
    self.init(top: across.frame.minY, bottom: across.frame.maxY, window: height)
  }
}

extension View {
  /// Has `pose` say how the window this fills stands, as its geometry changes.
  func reads(_ pose: Pose) -> some View {
    background {
      Color.clear
        .onGeometryChange(for: Fold?.self) { window in
          Fold(
            among: window.reservedRegions(kind: .division).map { ($0.frame, $0.isActive) }, height: window.size.height)
        } action: {
          pose.fold = $0
        }
        .ignoresSafeArea()
      GeometryReader { window in
        let safeArea = window.safeAreaInsets.bottom
        Color.clear
          .onGeometryChange(for: WindowFoot.self) {
            // Past the container's safe area, what is left is the keyboard's. Both are told of at once.
            WindowFoot(safeArea: safeArea, keyboard: $0.safeAreaInsets.bottom)
          } action: {
            pose.take($0)
          }
          .ignoresSafeArea(.container, edges: .bottom)
      }
    }
  }
}
