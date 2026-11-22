// PROTOTYPE for #719, "How the app's sheets stand on a Duo". Throwaway: never merge.
//
// Launch arguments, over #714's columns and #718's laptop pose:
//
//   --sheet-bar <asis|content|stack|both>   where `toolbarVerticalBehavior(.disabled)` is applied to each sheet: on
//                                           the sheet's outermost content (outside any NavigationStack), on the root
//                                           view inside its NavigationStack, or both. `asis` applies it nowhere.
//   --sheet-wrap                            a sheet with no NavigationStack of its own (the Approval, an Unreachable
//                                           Remote's) is wrapped in one whose navigation bar is hidden, and the
//                                           modifier applied inside it too as `--sheet-bar` says
//   --sheet-symbols                         Decline on a Questionnaire and Cancel on the relay login are a symbol
//                                           with their title, so they join the sheet's vertical bar
//   --sheet-place <automatic|leading|center|trailing>   an Intervention's sheet's `presentationPlacement(_:)`
//   --sheet-q-keeps                         a Questionnaire's sheet does not choose its lower detent as the detents
//                                           it offers change (the laptop pose coming and going)
//
// Every sheet says where it stands in /tmp/duo719/app.log as its frame or safe area changes.

import SwiftUI

@MainActor @Observable
final class ProtoS {
  static let shared = ProtoS()

  enum Bar: String {
    case asis, content, stack, both
  }

  var bar: Bar
  var wrap: Bool
  var symbols: Bool
  var place: String?
  var questionnaireKeeps: Bool

  private init() {
    let arguments = ProcessInfo.processInfo.arguments
    func value(_ flag: String) -> String? {
      arguments.firstIndex(of: flag).flatMap { arguments.dropFirst($0 + 1).first }
    }
    bar = value("--sheet-bar").flatMap(Bar.init) ?? .asis
    wrap = arguments.contains("--sheet-wrap")
    symbols = arguments.contains("--sheet-symbols")
    place = value("--sheet-place")
    questionnaireKeeps = arguments.contains("--sheet-q-keeps")
  }

  var placement: PresentationPlacement? {
    switch place {
    case "automatic": .automatic
    case "leading": .leading
    case "center": .center
    case "trailing": .trailing
    default: nil
    }
  }
}

/// Where in a sheet a modifier is being applied.
enum ProtoSheetSpot {
  /// The sheet's outermost content.
  case content
  /// The root view inside the sheet's NavigationStack.
  case stack
}

private struct ProtoSheetBar: ViewModifier {
  let spot: ProtoSheetSpot

  func body(content: Content) -> some View {
    let bar = ProtoS.shared.bar
    let applies = bar == .both || (spot == .content && bar == .content) || (spot == .stack && bar == .stack)
    if applies, #available(iOS 27.1, *) {
      content.toolbarVerticalBehavior(.disabled)
    } else {
      content
    }
  }
}

private struct ProtoSheetPlace: ViewModifier {
  func body(content: Content) -> some View {
    if let placement = ProtoS.shared.placement {
      content.presentationPlacement(placement)
    } else {
      content
    }
  }
}

/// Wraps a sheet that has no NavigationStack in one with its bar hidden, where the launch asked.
private struct ProtoSheetWrap: ViewModifier {
  func body(content: Content) -> some View {
    if ProtoS.shared.wrap {
      NavigationStack {
        content
          .toolbar(.hidden, for: .navigationBar)
          .modifier(ProtoSheetBar(spot: .stack))
      }
    } else {
      content
    }
  }
}

/// Says where a sheet stands, in the screen's own points, as that changes.
private struct ProtoSheetProbe: ViewModifier {
  let name: String

  struct Stands: Equatable {
    var frame: CGRect
    var insets: EdgeInsets
  }

  func body(content: Content) -> some View {
    content.background {
      Color.clear
        .onGeometryChange(for: Stands.self) {
          Stands(frame: $0.frame(in: .global), insets: $0.safeAreaInsets)
        } action: { stands in
          let f = stands.frame
          let i = stands.insets
          func r(_ value: CGFloat) -> String { String(format: "%.1f", value) }
          protoLog(
            "sheet \(name) safe x \(r(f.minX)) y \(r(f.minY)) w \(r(f.width)) h \(r(f.height))"
              + " insets t \(r(i.top)) l \(r(i.leading)) b \(r(i.bottom)) r \(r(i.trailing))")
        }
      Color.clear
        .ignoresSafeArea()
        .onGeometryChange(for: CGRect.self) { $0.frame(in: .global) } action: { f in
          func r(_ value: CGFloat) -> String { String(format: "%.1f", value) }
          protoLog("sheet \(name) whole x \(r(f.minX)) y \(r(f.minY)) w \(r(f.width)) h \(r(f.height))")
        }
    }
  }
}

extension View {
  /// On a sheet's outermost content: the vertical bar's behaviour, the placement and the probe.
  func protoSheet(_ name: String, places: Bool = false, wraps: Bool = false) -> some View {
    modifier(ProtoSheetProbe(name: name))
      .modifier(WrapIf(wraps: wraps))
      .modifier(ProtoSheetBar(spot: .content))
      .modifier(PlaceIf(places: places))
  }

  /// On the root view inside a sheet's NavigationStack.
  func protoSheetStack() -> some View {
    modifier(ProtoSheetBar(spot: .stack))
  }
}

private struct WrapIf: ViewModifier {
  let wraps: Bool
  func body(content: Content) -> some View {
    if wraps {
      content.modifier(ProtoSheetWrap())
    } else {
      content
    }
  }
}

private struct PlaceIf: ViewModifier {
  let places: Bool
  func body(content: Content) -> some View {
    if places {
      content.modifier(ProtoSheetPlace())
    } else {
      content
    }
  }
}
