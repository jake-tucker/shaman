import SwiftUI

extension View {
  /// Stands at the screen's foot, on its own ground, what `lines` draws of `held` over its composer's `field`, and
  /// nothing while nothing is `held`. Sat like a laptop, as `pose` says, `below` stands below the fold between the two,
  /// whatever is held.
  func composerFoot<Held>(
    _ held: Held?, pose: Pose?, @ViewBuilder lines: @escaping (Held) -> some View,
    @ViewBuilder field: @escaping (Held) -> some View, @ViewBuilder below: @escaping () -> some View
  ) -> some View {
    modifier(ComposerFoot(held: held, pose: pose, composes: true, lines: lines, field: field, below: below))
  }

  /// Has a screen of the main view with no composer end at the fold while `pose` says it is sat like a laptop, `below`
  /// standing in the half below the fold as it does in a composer's foot; in every other pose it stands as it would
  /// without.
  func halfBelowTheFold(pose: Pose, @ViewBuilder below: @escaping () -> some View) -> some View {
    modifier(
      ComposerFoot(
        held: Void?.none, pose: pose, composes: false, lines: { _ in EmptyView() }, field: { _ in EmptyView() },
        below: below))
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
/// that. Where the window's safe area reaches further up than the keyboard, as past the band kept for a fold, it stands
/// as far above that.
///
/// It is one stack of two parts: its lines, and then its field. Sat like a laptop with no keyboard up, the second part
/// is as tall as the half below the fold, and begins with the band kept for the fold and what stands below it, so the
/// screen's own content and the lines end at the fold. A keyboard takes that half, and the second part is the field
/// alone, above the fold. The field is one view throughout, so it keeps its focus: made as two, one above the fold and
/// one below, the keyboard never rises (#718, #732).
///
/// It keeps no sides: what the foot draws stands across its column by `readingSides`, under the conversation's words,
/// but for what stands below the fold, which is as wide as the column.
///
/// A screen with no composer, as a Turn's work pushed in the main view, carries the second part alone, and only while
/// it is sat like a laptop, so that what stands below the fold stands there as it does under a Session; out of the pose
/// it ignores no safe area.
private struct ComposerFoot<Held, Lines: View, Field: View, Below: View>: ViewModifier {
  let held: Held?
  let pose: Pose?
  /// Whether it is a composer's foot, which stands as Messages' does in every pose.
  let composes: Bool
  let lines: (Held) -> Lines
  let field: (Held) -> Field
  let below: () -> Below
  @Environment(\.horizontalSizeClass) private var width

  /// How far above the display's bottom edge Messages stands its field, the keyboard down.
  private var stands: CGFloat { width == .regular ? 22 : 28 }
  /// How far above the keyboard the field stands, and the ground above each thing the foot holds.
  private let clear: CGFloat = 8

  func body(content: Content) -> some View {
    let fold = pose?.foldWithHalf
    let keyboard = pose?.keyboard ?? 0
    content
      .safeAreaInset(edge: .bottom, spacing: 0) {
        if held != nil || fold != nil {
          VStack(spacing: 0) {
            if let held {
              Group(subviews: lines(held)) { lines in
                VStack(spacing: 0) {
                  ForEach(lines) { $0.padding(.top, clear) }
                }
                .padding(.bottom, fold == nil || lines.isEmpty ? 0 : clear)
              }
            }
            VStack(spacing: 0) {
              if let fold {
                Color.clear.frame(height: fold.band)
                below()
                  .frame(maxWidth: .infinity, maxHeight: .infinity)
              }
              if let held {
                field(held)
                  .padding(.top, clear)
              }
            }
            .padding(.bottom, max(clear + (pose?.aboveKeyboard ?? 0), stands - keyboard))
            // Less the last of a keyboard going, which is not up and lifts it still.
            .frame(height: fold.map { max(0, $0.below - keyboard) }, alignment: .bottom)
            .background(fold == nil ? Color.clear : Color(.systemBackground))
          }
          .background(.bar)
        }
      }
      .ignoresSafeArea(.container, edges: composes || fold != nil ? .bottom : [])
  }
}
