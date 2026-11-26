import SwiftUI

/// How a window stands: about the fold, where its device is sat like a laptop, and over a keyboard. It is read once,
/// above the columns, as the window's geometry changes, and handed down; no hosted screen asks for either, and nothing
/// asks on a clock. There is no pose but that one for the columns: in every other the layout is the size class's alone.
/// Pairing, which stands outside them, reads it the same way at its own root, and reads the fold of the book pose too.
@MainActor @Observable
final class Pose {
  /// The fold, while the device is partly folded like a book: the one last read, where it is read, which only pairing
  /// does.
  var book: BookFold?
  /// The fold, while the device is sat like a laptop: the one last read, while it is of a window as tall as the window
  /// was last told it is to stand.
  var fold: Fold? {
    get {
      guard let read, let height, abs(read.window - height) > 0.5 else { return read }
      return nil
    }
    set { read = newValue }
  }
  /// The fold as it was last read, of whatever size.
  private var read: Fold?
  /// How tall the window is to stand, as it was last told before a change of size; nothing until it is first told.
  private var height: CGFloat?
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

  /// Told the window is to stand at `size`, before anything is laid out at it: a fold read of a window as tall as it was
  /// is not of that one, so no screen is laid out against it until the fold is read again, or the window is told it
  /// stands as tall as that fold's once more, as where a change of size is announced and taken back.
  func turns(to size: CGSize) {
    if height != size.height {
      height = size.height
    }
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
  /// The fold in a window of `size`, among the regions the system reserves where a display divides: one that is active
  /// and wider than it is tall, lying across the window from side to side. Partly folded like a book, it is taller. Read
  /// as the window turns, the region may be the one of the size it had, which lies across no more than that.
  init?(among regions: some Sequence<(frame: CGRect, isActive: Bool)>, in size: CGSize) {
    guard
      let across = regions.first(where: {
        $0.isActive && $0.frame.width > $0.frame.height && $0.frame.minX <= 0.5 && $0.frame.maxX >= size.width - 0.5
      })
    else { return nil }
    self.init(top: across.frame.minY, bottom: across.frame.maxY, window: size.height)
  }
}

/// The fold standing across a window from head to foot, as the device partly folded like a book has it: the band the
/// system keeps for it, with a half before it, at the leading side, and a half after.
struct BookFold: Equatable {
  /// How far from the window's leading edge the band begins.
  let leading: CGFloat
  /// How wide the window is.
  let window: CGFloat

  /// How wide the half after is, from where the band begins.
  var after: CGFloat { max(0, window - leading) }
}

extension BookFold {
  /// The fold in a window of `size`, among the regions the system reserves where a display divides: one that is active
  /// and taller than it is wide, standing across the window from head to foot. Read as the window turns, the region may
  /// be the one of the size it had, which stands across no more than that.
  init?(among regions: some Sequence<(frame: CGRect, isActive: Bool)>, in size: CGSize) {
    guard
      let down = regions.first(where: {
        $0.isActive && $0.frame.height > $0.frame.width && $0.frame.minY <= 0.5
          && $0.frame.maxY >= size.height - 0.5
      })
    else { return nil }
    self.init(leading: down.frame.minX, window: size.width)
  }
}

extension View {
  /// Has `pose` say how the window this fills stands, as its geometry changes.
  func reads(_ pose: Pose) -> some View {
    background {
      Color.clear
        .onGeometryChange(for: Fold?.self) { window in
          Fold(among: window.divisions, in: window.size)
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

extension View {
  /// Has `pose` say too where the fold of the book pose stands in the window this fills, as its geometry changes, for
  /// pairing, which keeps to one side of it. The columns need nothing of it.
  func readsTheBooksFold(_ pose: Pose) -> some View {
    background {
      Color.clear
        .onGeometryChange(for: BookFold?.self) { window in
          BookFold(among: window.divisions, in: window.size)
        } action: {
          pose.book = $0
        }
        .ignoresSafeArea()
    }
  }
}

extension GeometryProxy {
  /// The regions the system reserves where the display divides, and whether each is active.
  fileprivate var divisions: [(frame: CGRect, isActive: Bool)] {
    reservedRegions(kind: .division).map { ($0.frame, $0.isActive) }
  }
}
