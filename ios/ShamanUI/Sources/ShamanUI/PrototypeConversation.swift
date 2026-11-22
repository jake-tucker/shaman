// PROTOTYPE for #717, "How the conversation and composer stand on a Duo". Throwaway: never merge.
//
// Built over #714's prototype, in its three columns. Each choice below is switched from the pill's "Conversation"
// menu, or at launch:
//
//   --conv-place <lost|kept>          whether a conversation away from its foot keeps the words at its top through a
//                                     change of width (today's build loses them)
//   --conv-rows <whole|blocks>        an Agent's answer as one row, or a row for each block of its Markdown, so that
//                                     the place kept is a block's and not a share of the whole answer
//   --conv-measure <0|600|700>        how wide a conversation reads at most, 0 for as wide as its column
//   --conv-align <centre|leading>     where a conversation narrower than its column stands
//   --conv-margins <fixed|system>     14 points a side, or the container's content margins
//   --conv-band <kept|16|8|0>         the bottom safe area under the composer: kept, or trimmed to so many points
//   --conv-title <automatic|never|down|up>   whether the title's bar scrolls away under the conversation
//   --conv-foot <inset|bar>           the foot as a `safeAreaInset` on its own ground, or as a `safeAreaBar`
//   --conv-ground <column|under>      whether the foot's ground stops at the side bar or reaches under it

import ShamanCore
import SwiftUI
import UIKit

@MainActor @Observable
final class ProtoC {
  static let shared = ProtoC()

  enum Place: String, CaseIterable {
    case lost, kept
    var name: String { self == .lost ? "lost, as today" : "kept" }
  }

  enum Rows: String, CaseIterable {
    case whole, blocks
    var name: String { self == .whole ? "one row" : "a row for each block" }
  }

  enum Align: String, CaseIterable {
    case centre, leading
  }

  enum Margins: String, CaseIterable {
    case fixed, system
    var name: String { self == .fixed ? "14 points" : "the system's" }
  }

  enum Title: String, CaseIterable {
    case automatic, never, down, up
    var name: String {
      switch self {
      case .automatic: "the system's choice"
      case .never: "stays"
      case .down: "goes scrolling down"
      case .up: "goes scrolling up"
      }
    }

    var behavior: ToolbarMinimizationBehavior {
      switch self {
      case .automatic: .automatic
      case .never: .never
      case .down: .onScrollDown
      case .up: .onScrollUp
      }
    }
  }

  enum Foot: String, CaseIterable {
    case inset, bar
    var name: String { self == .inset ? "safeAreaInset, own ground" : "safeAreaBar" }
  }

  enum Ground: String, CaseIterable {
    case column, under
    var name: String { self == .column ? "stops at the side bar" : "reaches under the side bar" }
  }

  var place: Place
  var rows: Rows
  /// How wide a conversation reads at most; 0 for as wide as its column.
  var measure: CGFloat
  var align: Align
  var margins: Margins
  /// What is left of the bottom safe area under the composer; nil keeps it.
  var band: CGFloat?
  var title: Title
  var foot: Foot
  var ground: Ground

  /// The container's content margins, as the system last gave them.
  var system = EdgeInsets()
  /// The window's own bottom safe area, which a keyboard does not change.
  var windowBottom: CGFloat = 0
  /// The bottom safe area the Session's screen stands in: the window's, or a keyboard's while one is up.
  var screenBottom: CGFloat = 0

  private init() {
    let arguments = ProcessInfo.processInfo.arguments
    func value(_ flag: String) -> String? {
      arguments.firstIndex(of: flag).flatMap { arguments.dropFirst($0 + 1).first }
    }
    place = value("--conv-place").flatMap(Place.init) ?? .kept
    rows = value("--conv-rows").flatMap(Rows.init) ?? .whole
    measure = value("--conv-measure").flatMap(Double.init).map { CGFloat($0) } ?? 700
    align = value("--conv-align").flatMap(Align.init) ?? .centre
    margins = value("--conv-margins").flatMap(Margins.init) ?? .fixed
    band = value("--conv-band").flatMap(Double.init).map { CGFloat($0) }
    title = value("--conv-title").flatMap(Title.init) ?? .automatic
    foot = value("--conv-foot").flatMap(Foot.init) ?? .inset
    ground = value("--conv-ground").flatMap(Ground.init) ?? .column
  }

  /// The padding a row of the conversation takes at each side, given what the app writes today.
  func sides(fixed: CGFloat) -> (leading: CGFloat, trailing: CGFloat) {
    switch margins {
    case .fixed: (fixed, fixed)
    case .system: (max(system.leading, 8), max(system.trailing, 8))
    }
  }
}

/// The conversation's choices, in the pill's menu.
struct ProtoConversationMenu: View {
  @Bindable var proto = ProtoC.shared

  var body: some View {
    Menu("Conversation") {
      Picker("Place through a change of width", selection: $proto.place) {
        ForEach(ProtoC.Place.allCases, id: \.self) { Text($0.name).tag($0) }
      }
      Picker("An answer is", selection: $proto.rows) {
        ForEach(ProtoC.Rows.allCases, id: \.self) { Text($0.name).tag($0) }
      }
      Picker("Reads at most", selection: $proto.measure) {
        Text("its column's width").tag(CGFloat(0))
        ForEach([560, 600, 640, 700] as [CGFloat], id: \.self) { Text("\(Int($0)) points").tag($0) }
      }
      Picker("Narrower than its column, stands", selection: $proto.align) {
        ForEach(ProtoC.Align.allCases, id: \.self) { Text($0.rawValue).tag($0) }
      }
      Picker("Margins", selection: $proto.margins) {
        ForEach(ProtoC.Margins.allCases, id: \.self) { Text($0.name).tag($0) }
      }
    }
    Menu("Composer and bars") {
      Picker("Band under the composer", selection: $proto.band) {
        Text("kept").tag(CGFloat?.none)
        ForEach([16, 8, 0] as [CGFloat], id: \.self) { Text("trimmed to \(Int($0)) points").tag(CGFloat?.some($0)) }
      }
      Picker("The title's bar", selection: $proto.title) {
        ForEach(ProtoC.Title.allCases, id: \.self) { Text($0.name).tag($0) }
      }
      Picker("The foot is", selection: $proto.foot) {
        ForEach(ProtoC.Foot.allCases, id: \.self) { Text($0.name).tag($0) }
      }
      Picker("The foot's ground", selection: $proto.ground) {
        ForEach(ProtoC.Ground.allCases, id: \.self) { Text($0.name).tag($0) }
      }
    }
  }
}

/// Reads the container's content margins and the window's bottom safe area for the screen it stands behind.
struct ProtoMarginsReader: View {
  var body: some View {
    GeometryReader { proxy in
      let margins: EdgeInsets = {
        if #available(iOS 27.1, *) {
          proxy.contentMargins(for: .container)
        } else {
          EdgeInsets()
        }
      }()
      Color.clear
        .onChange(of: Seen(margins: margins, size: proxy.size, safe: proxy.safeAreaInsets), initial: true) { _, seen in
          ProtoC.shared.system = seen.margins
          ProtoC.shared.screenBottom = seen.safe.bottom
          let window = UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }
            .flatMap(\.windows).first(where: \.isKeyWindow)
          ProtoC.shared.windowBottom = window?.safeAreaInsets.bottom ?? 0
          protoLog(
            "conversation column \(Int(seen.size.width))x\(Int(seen.size.height)) safe \(seen.safe) margins \(seen.margins) window bottom \(ProtoC.shared.windowBottom)"
          )
        }
    }
    .allowsHitTesting(false)
    .accessibilityHidden(true)
  }

  private struct Seen: Equatable {
    let margins: EdgeInsets
    let size: CGSize
    let safe: EdgeInsets
  }
}

extension View {
  /// Narrows a row or the foot to the measure the conversation reads at, standing where the pill says.
  func protoMeasured() -> some View {
    let proto = ProtoC.shared
    return frame(maxWidth: proto.measure > 0 ? proto.measure : .infinity)
      .frame(maxWidth: .infinity, alignment: proto.align == .centre ? .center : .leading)
  }

  /// Pulls the foot down into the bottom safe area, leaving `band` points of it, while no keyboard stands there.
  func protoBand() -> some View {
    let proto = ProtoC.shared
    // With the keyboard up the inset is the keyboard's, and the foot stands on it as it is.
    let own = proto.screenBottom <= proto.windowBottom + 0.5
    let pull = proto.band.map { own ? max(0, proto.windowBottom - $0) : 0 } ?? 0
    return padding(.bottom, -pull)
  }
}

/// An Agent's Markdown as its blocks: split at blank lines outside fenced code. Rough, and the core's to do in a
/// real build, where Swift folds nothing.
func protoBlocks(_ markdown: String) -> [String] {
  var blocks: [String] = []
  var current: [Substring] = []
  var fenced = false
  for line in markdown.split(separator: "\n", omittingEmptySubsequences: false) {
    let trimmed = line.trimmingCharacters(in: .whitespaces)
    if trimmed.hasPrefix("```") || trimmed.hasPrefix("~~~") {
      fenced.toggle()
    }
    if trimmed.isEmpty, !fenced {
      if !current.isEmpty {
        blocks.append(current.joined(separator: "\n"))
        current = []
      }
    } else {
      current.append(line)
    }
  }
  if !current.isEmpty {
    blocks.append(current.joined(separator: "\n"))
  }
  return blocks.isEmpty ? [markdown] : blocks
}
