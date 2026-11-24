// PROTOTYPE for #733, "Whether the Sidebar stands where it was decided under the UIKit container". Throwaway: never
// merge.
//
// #732's container, left to itself, stands the Sidebar somewhere else than #714 decided in two places: the book pose
// with the Aside not shown, and portrait on the inner display with nothing open. This says what the split view's
// display mode and split behaviour are at each moment and who set them, and tries the ways of putting the Sidebar
// where it was decided.
//
// `--sidebar <words>` chooses, the words joined by commas; a simulator also reads them from
// /tmp/duo733/sidebar-<its UDID>, or /tmp/duo733/sidebar, where no argument says.
//
//   (none)          the way found: the main view is given no least width, so the split view divides the Sidebar and
//                   the main view at the fold by itself, and while nothing is open the Sidebar is asked for wherever
//                   the system stands the main view alone: as the container appears, as it expands, as what is open
//                   closes, and at the end of a change of size, there only if the reader had not put it away
//                   before the window changed size
//   min=<n>         the main view is given a least width, in points; 400 is #732's container
//   norest          the Sidebar is not asked for while nothing is open: the system's own choice
//   always          the Sidebar is asked for after a change of size whether or not it stood before it
//   prefer=<n>      the preferred display mode, by its raw value (0 automatic, 1 the main view alone, 2 the Sidebar
//                   beside, 3 the Sidebar over)
//   behavior=<n>    the preferred split behaviour (0 automatic, 1 tile, 2 overlay, 3 displace)
//   maxprimary=<n>  the most the Sidebar's column may take, in points
//   fraction=<f>    the share of the width the Sidebar's column prefers
//   who             each change of display mode is logged with the frames that asked for it
//   plain           the delegate answers to nothing #732's did not, to see whether being asked changes the choice
//
// While the app runs, a line written to /tmp/duo733/poke is done and the file removed (`dump`, `show primary`,
// `hide primary`, `prefer 2`, `behavior 1`, `maxprimary 600`, `fraction 0.5`, `minsecondary 0`, `close`): a way of
// trying things by hand, on a clock, and no part of what is found. Every event is a line in /tmp/duo733/app.log.

import ShamanCore
import SwiftUI
import UIKit

@MainActor
enum SidebarC {
  static let words: [String] = {
    let arguments = ProcessInfo.processInfo.arguments
    if let flag = arguments.firstIndex(of: "--sidebar"), let named = arguments.dropFirst(flag + 1).first {
      return named.split(separator: ",").map(String.init)
    }
    #if targetEnvironment(simulator)
      let udid = ProcessInfo.processInfo.environment["SIMULATOR_UDID"] ?? ""
      for name in ["sidebar-\(udid)", "sidebar"] {
        if let said = try? String(contentsOfFile: "/tmp/duo733/\(name)", encoding: .utf8) {
          return said.split(whereSeparator: { $0.isWhitespace || $0 == "," }).map(String.init)
        }
      }
    #endif
    return []
  }()

  static func has(_ word: String) -> Bool { words.contains(word) }

  static func number(_ name: String) -> Double? {
    words.first { $0.hasPrefix(name + "=") }.flatMap { Double($0.dropFirst(name.count + 1)) }
  }
}

/// What the window says of a fold standing upright, as in the book pose, read above the container beside the fold
/// lying across.
@MainActor @Observable
final class ProtoS {
  static let shared = ProtoS()

  struct Upright: Equatable {
    var left: CGFloat
    var right: CGFloat
    var window: CGFloat
  }

  var upright: Upright?
  /// Told as either fold comes or goes: the container's to hear, where the real build would be handed a value.
  @ObservationIgnored var foldChanged: (() -> Void)?
}

extension RealColumns {
  /// Everything the split view says of where its columns stand.
  var state: String {
    func span(_ controller: UIViewController) -> String {
      guard controller.isViewLoaded, controller.view.window != nil else { return "off" }
      let frame = controller.view.convert(controller.view.bounds, to: view)
      return "\(Int(frame.minX))-\(Int(frame.maxX))"
    }
    let size = view.bounds.size
    let window = (view.window ?? view).traitCollection
    let fold = ProtoS.shared.upright.map { "upright \($0.left)-\($0.right)" }
      ?? ProtoL.shared.fold.map { "across \($0.top)-\($0.bottom)" } ?? "none"
    return "mode=\(displayMode.rawValue) prefers=\(preferredDisplayMode.rawValue) behavior=\(splitBehavior.rawValue)"
      + " prefersBehavior=\(preferredSplitBehavior.rawValue) collapsed=\(isCollapsed)"
      + " size=\(Int(size.width))x\(Int(size.height)) class=\(window.horizontalSizeClass.rawValue)/\(window.verticalSizeClass.rawValue)"
      + " sidebar=\(span(sidebar)) main=\(span(main)) primaryWidth=\(Int(primaryColumnWidth))"
      + " showing=\(isShowing(.primary) ? "S" : "-")\(isShowing(.secondary) ? "M" : "-")\(isShowing(.inspector) ? "A" : "-")"
      + " open=\(stack.path.count) fold=\(fold)"
  }

  /// The frames that asked for what is being logged, UIKit's and the app's, nearest first.
  static func asked() -> String {
    Thread.callStackSymbols.dropFirst(2).prefix(16).map { line -> String in
      let parts = line.split(separator: " ", omittingEmptySubsequences: true)
      guard parts.count > 3 else { return line }
      return parts.dropFirst(3).joined(separator: " ").replacingOccurrences(of: " + ", with: "+")
    }.joined(separator: "\n        ")
  }

  /// Applies the words that are set once, as the container is made.
  func applySidebarWords() {
    if let mode = SidebarC.number("prefer").flatMap({ UISplitViewController.DisplayMode(rawValue: Int($0)) }) {
      preferredDisplayMode = mode
    }
    if let behavior = SidebarC.number("behavior").flatMap({ UISplitViewController.SplitBehavior(rawValue: Int($0)) }) {
      preferredSplitBehavior = behavior
    }
    if let most = SidebarC.number("maxprimary") { maximumPrimaryColumnWidth = most }
    if let share = SidebarC.number("fraction") { preferredPrimaryColumnWidthFraction = share }
    minimumSecondaryColumnWidth = SidebarC.number("min") ?? UISplitViewController.automaticDimension
    colLog("sidebar words \(SidebarC.words)")
  }

  /// A line from the Mac, done by hand while trying things. No part of what is found.
  func poke(_ line: String) {
    let words = line.split(separator: " ").map(String.init)
    let value = words.count > 1 ? Double(words[1]) : nil
    func column(_ name: String?) -> UISplitViewController.Column? {
      switch name {
      case "primary": .primary
      case "inspector": .inspector
      case "secondary": .secondary
      default: nil
      }
    }
    colLog("poke: \(line) | before \(state)")
    switch words.first ?? "" {
    case "dump": break
    case "show": if let column = column(words.dropFirst().first) { show(column) }
    case "hide": if let column = column(words.dropFirst().first) { hide(column) }
    case "prefer":
      if let mode = value.flatMap({ UISplitViewController.DisplayMode(rawValue: Int($0)) }) { preferredDisplayMode = mode }
    case "behavior":
      if let behavior = value.flatMap({ UISplitViewController.SplitBehavior(rawValue: Int($0)) }) {
        preferredSplitBehavior = behavior
      }
    case "maxprimary": if let value { maximumPrimaryColumnWidth = value }
    case "minprimary": if let value { minimumPrimaryColumnWidth = value }
    case "primary": if let value { preferredPrimaryColumnWidth = value }
    case "fraction": if let value { preferredPrimaryColumnWidthFraction = value }
    case "minsecondary": if let value { minimumSecondaryColumnWidth = value }
    case "close": stack.path = []
    default: colLog("poke: unknown")
    }
    colLog("poke: after \(state)")
  }

  func startPokes() {
    #if targetEnvironment(simulator)
      let path = "/tmp/duo733/poke"
      Timer.scheduledTimer(withTimeInterval: 0.2, repeats: true) { [weak self] _ in
        guard let told = try? String(contentsOfFile: path, encoding: .utf8) else { return }
        try? FileManager.default.removeItem(atPath: path)
        Task { @MainActor in self?.poke(told.trimmingCharacters(in: .whitespacesAndNewlines)) }
      }
    #endif
  }
}
