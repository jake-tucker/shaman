import SwiftUI

/// How far from each edge of its column a conversation stands, its rows and what its foot holds alike: by the container's
/// content margins, though never nearer than `least`, as the container has none beside a vertical bar; and further
/// where the column is wide, so that it reads at most `widest` points across, in the middle of the column.
struct ReadingSides: Equatable {
  static let widest: CGFloat = 700
  static let least: CGFloat = 8

  let leading: CGFloat
  let trailing: CGFloat
}

extension ReadingSides {
  init(across width: CGFloat, margins: EdgeInsets) {
    let spare = max(0, width - Self.widest)
    // From a whole point, so that words stand on pixels, and the same one where the width is a hair less or more.
    let half = (spare.rounded() / 2).rounded(.down)
    self.init(
      leading: max(margins.leading, Self.least, half), trailing: max(margins.trailing, Self.least, spare - half))
  }

  /// What they leave of `width`.
  func within(_ width: CGFloat) -> CGFloat {
    max(0, width - leading - trailing)
  }
}

extension GeometryProxy {
  /// The container's content margins at the sides, to the point, as whatever is worked out from a width may be a hair
  /// more or less each time it is.
  var readingMargins: EdgeInsets {
    let margins = contentMargins(for: .container)
    return EdgeInsets(top: 0, leading: margins.leading.rounded(), bottom: 0, trailing: margins.trailing.rounded())
  }
}

extension View {
  /// Stands it across its column as a conversation reads, by `ReadingSides`.
  func readingSides() -> some View {
    modifier(StandsByReadingSides())
  }
}

/// The margins are read where it stands and held; its sides are worked out from them and the width it is proposed as it
/// is laid out, and never held. SwiftUI's own arithmetic has that width a hair more or less each time the screen is laid
/// out, as 832 points and 832.0000000000001, so sides held from it would lay it out again without end.
private struct StandsByReadingSides: ViewModifier {
  @State private var margins = EdgeInsets()

  func body(content: Content) -> some View {
    ReadingColumn(margins: margins) { content }
      .onGeometryChange(for: EdgeInsets.self) {
        $0.readingMargins
      } action: {
        margins = $0
      }
  }
}

/// The one view it holds, across the width proposed but for the sides.
private struct ReadingColumn: Layout {
  let margins: EdgeInsets

  func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
    guard let width = proposal.width, width.isFinite, let held = subviews.first else {
      return subviews.first?.sizeThatFits(proposal) ?? .zero
    }
    return CGSize(width: width, height: held.sizeThatFits(within(width, proposal.height)).height)
  }

  func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
    let sides = ReadingSides(across: bounds.width, margins: margins)
    subviews.first?.place(
      at: CGPoint(x: bounds.minX + sides.leading, y: bounds.minY), proposal: within(bounds.width, bounds.height))
  }

  private func within(_ width: CGFloat, _ height: CGFloat?) -> ProposedViewSize {
    let sides = ReadingSides(across: width, margins: margins)
    return ProposedViewSize(width: sides.within(width), height: height)
  }
}
