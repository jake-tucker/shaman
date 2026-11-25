import ShamanCore
import SwiftUI
import Textual

/// One block of what the Agent said, as Markdown across the width: code blocks and tables scroll sideways rather than
/// wrap. Where another Model answered than the one selected, a subdued line above a Message's first block names both.
struct AgentBlockView: View {
  let kind: AgentBlockKind
  let markdown: String
  let model: String?

  var body: some View {
    VStack(alignment: .leading, spacing: 4) {
      if let model {
        Text(verbatim: model)
          .font(.caption)
          .foregroundStyle(.tertiary)
          .accessibilityIdentifier("session.model")
      }
      StructuredText(markdown: markdown)
        .textual.overflowMode(.scroll)
        .textual.tableStyle(.overflow)
        .frame(maxWidth: .infinity, alignment: .leading)
    }
    .accessibilityElement(children: .combine)
    .accessibilityIdentifier("session.agentMessage")
    // A rule alone holds nothing to read.
    .accessibilityHidden(kind == .rule && model == nil)
  }
}

/// The margins Textual 0.5.0 gives a block above and below, in ems of the body size, none where it gives none. A block
/// drawn alone gets no margin, so the conversation's list stands each block of a Message below the one above it by the
/// gap the two would have had, which Textual has for itself: `AgentBlockHostedTests` holds these to what it draws.
struct BlockMargins: Equatable {
  private(set) var top: CGFloat?
  private(set) var bottom: CGFloat?

  /// The margins of a block of `kind` holding blocks of the kinds `holds`. A quotation and a list have none of their
  /// own: each edge of theirs is the largest of what they hold, however deep and wherever in them it stands.
  init(_ kind: AgentBlockKind, holding holds: [AgentBlockKind] = []) {
    switch kind {
    case .paragraph: top = 0.8
    case .heading: (top, bottom) = (1.6, 0.8)
    case .code: (top, bottom) = (0.88, 0)
    case .table, .rule: (top, bottom) = (1.6, 1.6)
    case .quotation, .list:
      for held in [.paragraph] + holds.filter({ $0 != .quotation && $0 != .list }) {
        let margins = BlockMargins(held)
        top = Self.larger(top, margins.top)
        bottom = Self.larger(bottom, margins.bottom)
      }
    }
  }

  /// The gap between this block and the one `lower` is of below it: the larger of this one's bottom margin and that
  /// one's top.
  func gap(above lower: BlockMargins) -> CGFloat {
    Self.larger(bottom, lower.top) ?? 0
  }

  private static func larger(_ one: CGFloat?, _ other: CGFloat?) -> CGFloat? {
    [one, other].compactMap(\.self).max()
  }
}

extension ConversationRow {
  /// How far below `upper` this row stands, in ems of the body size, where the two are blocks of one Message; nothing
  /// where they are not, as any two other rows stand the list's own gap apart.
  func gap(below upper: ConversationRow) -> CGFloat? {
    guard case .item(let above) = upper, case .item(let below) = self,
      case .agentBlock(let message, let kind, let holds, _, _) = above.kind,
      case .agentBlock(message, let lowerKind, let lowerHolds, _, _) = below.kind
    else { return nil }
    return BlockMargins(kind, holding: holds).gap(above: BlockMargins(lowerKind, holding: lowerHolds))
  }
}
