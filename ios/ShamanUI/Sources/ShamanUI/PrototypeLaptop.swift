// PROTOTYPE for #718, "Whether the laptop pose earns an arrangement of its own". Throwaway: never merge.
//
// Five variants of an open Session while the phone sits like a laptop, the inner display in portrait with the fold
// across its middle, on the app's real screens over #714's columns and #717's conversation. Switched by the floating
// pill's arrows, or by `--laptop <asis|clear|work|listing|arrangement>` at launch. Out of that pose every variant is
// the same build.
//
//   asis         the layout as it stands: one column through the fold, the composer on the bottom edge
//   clear        the conversation above the fold alone; the bar and composer below it, and the rest of that half empty
//   work         the conversation and composer above the fold; below it the third column, a Turn's work or the tree
//   listing      the conversation and composer above the fold; below it the listing
//   arrangement  the conversation and the foot in SwiftUI's own `ArrangementView`, split down the vertical axis
//
// `--laptop-composer <under|bottom>` says where `clear` stands the composer: just under the fold, or on the bottom
// edge. `--laptop-arrangement <split|fixed|overlay>` says how `arrangement` is styled: split evenly, split with the
// foot at its own height, or the foot overlaid on the conversation's bottom edge. `--laptop-deck-composer
// <bottom|above>` says where `work` and `listing` stand the field and send: on the bottom edge of the half below the
// fold, the bar and the other lines staying above it, or above the fold with them. `--laptop-questionnaire
// <half|full>` says whether a Questionnaire's sheet lands on the half below the fold, as an Approval's does.

import ShamanCore
import SwiftUI
import UIKit

@MainActor @Observable
final class ProtoL {
  static let shared = ProtoL()

  enum Variant: String, CaseIterable {
    case asis, clear, work, listing, arrangement

    var name: String {
      switch self {
      case .asis: "A · as it stands"
      case .clear: "B · composer below"
      case .work: "C · work below"
      case .listing: "D · listing below"
      case .arrangement: "E · arrangement view"
      }
    }
  }

  /// Where `clear` stands the composer in the half below the fold.
  enum Composer: String, CaseIterable {
    case under, bottom
    var name: String { self == .under ? "just under the fold" : "on the bottom edge" }
  }

  /// How `arrangement` styles SwiftUI's arrangement view.
  enum Styled: String, CaseIterable {
    case split, fixed, overlay
    var name: String {
      switch self {
      case .split: "split"
      case .fixed: "split, the foot at its own height"
      case .overlay: "overlay at the bottom edge"
      }
    }
  }

  /// Where `work` and `listing` stand the composer's field and send.
  enum DeckComposer: String, CaseIterable {
    case bottom, above
    var name: String { self == .bottom ? "on the bottom edge, the bar above the fold" : "above the fold" }
  }

  /// How tall a Questionnaire's sheet stands while the phone sits like a laptop.
  enum Questionnaire: String, CaseIterable {
    case half, full
    var name: String { self == .half ? "the half below the fold" : "full height, as today" }
  }

  var variant: Variant
  var composer: Composer
  var styled: Styled
  var deckComposer: DeckComposer
  var questionnaire: Questionnaire
  /// Whether a Session's screen is the one up in the second column.
  var sessionUp = false
  /// How tall the field and send stand, with their padding, for each Session's screen.
  var composerHeight: [SessionRoute: CGFloat] = [:]
  /// The same for the new-session screen.
  var landingComposerHeight: CGFloat = 0

  /// The fold while it lies across the window, in the window's own coordinates: where the band kept for it begins
  /// and ends, and how tall the window is.
  struct Fold: Equatable {
    var top: CGFloat
    var bottom: CGFloat
    var window: CGFloat
  }

  var fold: Fold?
  /// The bottom safe area the whole window stands in: the home indicator's, or a keyboard's while one is up.
  var rootBottom: CGFloat = 0

  private init() {
    let arguments = ProcessInfo.processInfo.arguments
    func value(_ flag: String) -> String? {
      arguments.firstIndex(of: flag).flatMap { arguments.dropFirst($0 + 1).first }
    }
    // PROTOTYPE #732: under the UIKit container the arrangement #718 chose is the default.
    variant = value("--laptop").flatMap(Variant.init) ?? (Columns.uikit && !Columns.stack ? .work : .asis)
    composer = value("--laptop-composer").flatMap(Composer.init) ?? .under
    styled = value("--laptop-arrangement").flatMap(Styled.init) ?? .split
    deckComposer = value("--laptop-deck-composer").flatMap(DeckComposer.init) ?? .bottom
    questionnaire = value("--laptop-questionnaire").flatMap(Questionnaire.init) ?? .half
  }

  /// Whether a keyboard stands in the half below the fold.
  var keyboardUp: Bool { rootBottom > 100 }

  /// What stands in the half below the fold beside the Session's own screen, while the phone sits like a laptop.
  var deck: Variant? {
    guard fold != nil, variant == .work || variant == .listing else { return nil }
    return variant
  }

  /// How far the second column's bottom is brought up so that it ends where the fold begins. With a keyboard up the
  /// keyboard has already brought it most of the way.
  var columnLift: CGFloat {
    guard let fold, deck != nil else { return 0 }
    // A screen's own foot does it where it also carries the field to the bottom edge.
    if deckComposer == .bottom { return 0 }
    return max(0, fold.window - fold.top - rootBottom)
  }

  /// Whether a Session's foot stands in two: the bar and its other lines above the fold, the field and send on the
  /// bottom edge, with the deck between.
  var footSplits: Bool {
    deck != nil && deckComposer == .bottom && sessionUp && !keyboardUp
  }

  /// Whether the new-session screen's foot stands in two the same way, while no Session is open.
  var landingSplits: Bool {
    deck != nil && deckComposer == .bottom && !sessionUp && !keyboardUp
  }

  /// How tall the second part of a split foot is, from where the fold begins to the bottom safe area.
  var lowerRoom: CGFloat {
    fold.map { max(0, $0.window - $0.top - rootBottom) } ?? 0
  }

  /// How tall the band kept for the fold is.
  var band: CGFloat {
    fold.map { $0.bottom - $0.top } ?? 0
  }

  /// The heights a Questionnaire's sheet may stand at.
  var questionnaireDetents: Set<PresentationDetent> {
    // A height detent stands 42 points taller than it is asked: the bottom safe area and 8 more. Asked so, the
    // sheet begins 8 points past the band kept for the fold, clear of it; the system's own medium begins 8 points
    // inside it.
    fold != nil && variant != .asis && questionnaire == .half ? [.height(max(200, below - 50)), .large] : [.large]
  }

  /// How tall the half below the fold is, from where the band kept for the fold ends.
  var below: CGFloat {
    fold.map { $0.window - $0.bottom } ?? 0
  }
}

/// PROTOTYPE: asks for the fold where it lies across the window, twice a second, as #714's reader does for the book
/// pose, and for the bottom safe area the window stands in.
@available(iOS 27.1, *)
struct LaptopFoldReader: View {
  var body: some View {
    TimelineView(.periodic(from: .now, by: 0.5)) { _ in
      GeometryReader { proxy in
        let region = proxy.reservedRegions(kind: .division).first { $0.isActive && $0.frame.width > $0.frame.height }
        let fold = region.map {
          ProtoL.Fold(top: $0.frame.minY, bottom: $0.frame.maxY, window: proxy.size.height)
        }
        Color.clear.onChange(of: fold, initial: true) { _, fold in
          protoLog("laptop fold \(String(describing: fold))")
          withAnimation(.snappy) { ProtoL.shared.fold = fold }
        }
      }
    }
    .ignoresSafeArea()
    .allowsHitTesting(false)
    .accessibilityHidden(true)
  }
}

/// PROTOTYPE: the foot of a Session's screen for `clear`: the band kept for the fold, then the foot just under it or
/// on the bottom edge, the conversation ending where the fold begins. One shape whatever the pose, so the composer
/// keeps its focus as the keyboard comes and the fold goes.
struct ProtoLaptopFoot<Foot: View>: View {
  @ViewBuilder let foot: Foot

  var body: some View {
    let laptop = ProtoL.shared
    let fold = laptop.variant == .clear && !laptop.keyboardUp ? laptop.fold : nil
    let room = fold.map { max(0, $0.window - $0.top - laptop.rootBottom) }
    VStack(spacing: 0) {
      Color.clear.frame(height: fold.map { $0.bottom - $0.top } ?? 0)
      Spacer(minLength: 0).frame(maxHeight: fold != nil && laptop.composer == .bottom ? .infinity : 0)
      foot
      Spacer(minLength: 0).frame(maxHeight: fold != nil && laptop.composer == .under ? .infinity : 0)
    }
    .frame(height: room, alignment: .top)
    .frame(maxWidth: .infinity)
    .background {
      if fold != nil {
        Color(.systemBackground).ignoresSafeArea(.container, edges: .bottom)
      }
    }
  }
}

/// The laptop pose's choices, in the pill's menu.
struct ProtoLaptopMenu: View {
  @Bindable var laptop = ProtoL.shared
  @Bindable var proto = Proto.shared

  var body: some View {
    Picker("Sat like a laptop", selection: $laptop.variant) {
      ForEach(ProtoL.Variant.allCases, id: \.self) { Text($0.name).tag($0) }
    }
    Picker("B stands the composer", selection: $laptop.composer) {
      ForEach(ProtoL.Composer.allCases, id: \.self) { Text($0.name).tag($0) }
    }
    Picker("C and D stand the composer", selection: $laptop.deckComposer) {
      ForEach(ProtoL.DeckComposer.allCases, id: \.self) { Text($0.name).tag($0) }
    }
    Picker("A questionnaire takes", selection: $laptop.questionnaire) {
      ForEach(ProtoL.Questionnaire.allCases, id: \.self) { Text($0.name).tag($0) }
    }
    Picker("E is styled", selection: $laptop.styled) {
      ForEach(ProtoL.Styled.allCases, id: \.self) { Text($0.name).tag($0) }
    }
    Menu("Columns") {
      Picker("Columns", selection: $proto.variant) {
        ForEach(Proto.Variant.allCases, id: \.self) { Text($0.name).tag($0) }
      }
    }
  }
}

/// PROTOTYPE: SwiftUI's arrangement view about a Session's conversation and foot, styled as the pill says.
@available(iOS 27.1, *)
struct ProtoArrangement<Primary: View, Secondary: View>: View {
  @ViewBuilder let primary: Primary
  @ViewBuilder let secondary: Secondary

  var body: some View {
    switch ProtoL.shared.styled {
    case .split:
      ArrangementView {
        primary
      } secondary: {
        secondary
      }
      .arrangementViewStyle(.split.axes(.vertical))
    case .fixed:
      ArrangementView {
        primary
      } secondary: {
        secondary.splitArrangementFixedLayoutSize(horizontal: false, vertical: true)
      }
      .arrangementViewStyle(.split.axes(.vertical))
    case .overlay:
      ArrangementView {
        primary
      } secondary: {
        secondary.overlayArrangementEdge(VerticalEdge.bottom)
      }
      .arrangementViewStyle(.overlay.axes(.vertical))
    }
  }
}
