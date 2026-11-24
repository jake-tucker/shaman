// PROTOTYPE for #732, "Whether the laptop pose's arrangement stands in the UIKit container". Throwaway: never merge.
//
// #718's arrangement, sat like a laptop, over #731's container: the conversation ends at the fold with the status
// lines at its foot, the Aside's content stands below the fold, and the field and send stand at the foot of the half
// below, going above the fold as the keyboard takes that half.
//
// `--laptop-container <words>` chooses how, the words joined by commas; a simulator also reads them from
// /tmp/duo732/laptop-<its UDID>, or /tmp/duo732/laptop, where no argument says.
//
//   inline       (the default) the Aside's content is drawn again within the screen on top of the main view, in the
//                second part of a Session's foot or at the foot of a screen that has none: two show one state
//   over-second  the container lays a second hosted Aside over the split view's half below the fold, stopping short
//                of the field by the field's measured height
//   over-moved   the same, with the inspector column's own view controller taken out of its column and moved there
//   two          the field is two views, one above the fold and one below, to see the focus lost
//   fakefold     a fold is made up across the middle of any upright window, so an ordinary iPhone can be tried;
//                while /tmp/duo732/nofold exists there is none
//
// Every event is a line in /tmp/duo732/app.log.

import ShamanCore
import SwiftUI
import UIKit

@MainActor
enum LaptopC {
  enum Below: String {
    case inline
    case overSecond = "over-second"
    case overMoved = "over-moved"
  }

  private static let words: [String] = {
    let arguments = ProcessInfo.processInfo.arguments
    if let flag = arguments.firstIndex(of: "--laptop-container"), let named = arguments.dropFirst(flag + 1).first {
      return named.split(separator: ",").map(String.init)
    }
    #if targetEnvironment(simulator)
      let udid = ProcessInfo.processInfo.environment["SIMULATOR_UDID"] ?? ""
      for name in ["laptop-\(udid)", "laptop"] {
        if let said = try? String(contentsOfFile: "/tmp/duo732/\(name)", encoding: .utf8) {
          return said.split(whereSeparator: { $0.isWhitespace || $0 == "," }).map(String.init)
        }
      }
    #endif
    return []
  }()

  static let below = words.compactMap(Below.init).first ?? .inline
  static let fieldTwo = words.contains("two")
  static let fakeFold = words.contains("fakefold")

  /// Whether the container lays the Aside over the half below the fold itself.
  static var over: Bool { below != .inline }
}

/// What the window says of the pose: the fold while it lies across, and the bottom safe area, a keyboard's while one
/// is up. Read as the system lays the screen out, and again as the fold comes and goes; nothing asks on a clock.
struct LaptopPoseReader: View {
  private struct Read: Equatable {
    var fold: ProtoL.Fold?
  }

  var body: some View {
    let fake = LaptopC.fakeFold
    Color.clear
      .onGeometryChange(for: Read.self) { proxy in
        var fold: ProtoL.Fold?
        if #available(iOS 27.1, *) {
          fold = proxy.reservedRegions(kind: .division)
            .first { $0.isActive && $0.frame.width > $0.frame.height }
            .map { ProtoL.Fold(top: $0.frame.minY, bottom: $0.frame.maxY, window: proxy.size.height) }
        }
        // A file on the Mac takes the made-up fold away, as laying the phone flat would, at the next change of size.
        if fold == nil, fake, proxy.size.height > proxy.size.width,
          !FileManager.default.fileExists(atPath: "/tmp/duo732/nofold")
        {
          let middle = (proxy.size.height / 2).rounded()
          fold = ProtoL.Fold(top: middle - 20, bottom: middle + 20, window: proxy.size.height)
        }
        return Read(fold: fold)
      } action: { read in
        colLog("pose: fold \(read.fold.map { "\($0.top)-\($0.bottom) of \($0.window)" } ?? "none")")
        ProtoL.shared.fold = read.fold
      }
      .ignoresSafeArea()
      .allowsHitTesting(false)
      .accessibilityHidden(true)
  }
}

/// The Aside's content drawn within a screen of the main view, below the band kept for the fold.
struct LaptopInlineAside: View {
  let stack: SessionStack

  var body: some View {
    AsideRoot(stack: stack)
      .frame(maxWidth: .infinity, maxHeight: .infinity)
      .background(Color(.systemBackground))
      // The Aside pushes within itself, as in its column: the main view's navigation controller is not asked.
      .environment(\.pushes, nil)
      .accessibilityIdentifier("laptop.below")
  }
}

/// The half below the fold for a screen of the main view that has no foot of its own: a Turn's work or an Activity's
/// whole pushed in portrait, which stays pushed as the phone is sat like a laptop. The screen ends at the fold and the
/// Aside stands below it.
struct LaptopLower: ViewModifier {
  let stack: SessionStack
  /// Whether the screen stands the half below the fold itself, as a Session's and the new-session screen's foot do.
  let footed: Bool

  func body(content: Content) -> some View {
    let laptop = ProtoL.shared
    let lowers = !footed && laptop.deck != nil && !laptop.keyboardUp && stack.topSession != nil
    content.safeAreaInset(edge: .bottom, spacing: 0) {
      if lowers {
        VStack(spacing: 0) {
          Color.clear.frame(height: laptop.band)
          if LaptopC.below == .inline {
            LaptopInlineAside(stack: stack)
          } else {
            Color.clear
          }
        }
        .frame(height: laptop.lowerRoom)
        .background(Color(.systemBackground).ignoresSafeArea(.container, edges: .bottom))
      }
    }
  }
}

/// The split view, and over its half below the fold the Aside's content, where the container lays it out itself.
final class LaptopHolder: UIViewController {
  let columns: RealColumns
  private let second: UIViewController?
  private var below: UIViewController?
  private var last = ""

  init(columns: RealColumns, second: UIViewController?) {
    self.columns = columns
    self.second = second
    super.init(nibName: nil, bundle: nil)
    columns.modeChanged = { [weak self] in self?.view.setNeedsLayout() }
    columns.lent = { [weak self] in self?.below != nil && self?.below === self?.columns.aside }
  }

  @available(*, unavailable) required init?(coder: NSCoder) { fatalError() }

  override func viewDidLoad() {
    super.viewDidLoad()
    addChild(columns)
    view.addSubview(columns.view)
    columns.didMove(toParent: self)
  }

  /// What the half below the fold holds, and where: read in one layout pass from the fold, the keyboard, the split
  /// view's display mode and the field's measured height, each of which asks for the pass again as it changes.
  override func viewWillLayoutSubviews() {
    super.viewWillLayoutSubviews()
    columns.view.frame = view.bounds
    let frame = withObservationTracking {
      wanted()
    } onChange: { [weak self] in
      Task { @MainActor in self?.view.setNeedsLayout() }
    }
    if let frame {
      let host = below ?? raise()
      host.view.frame = frame
    } else if let host = below {
      lower(host)
    }
    let said = "holder: below \(frame.map { "\(Int($0.minY))-\(Int($0.maxY))" } ?? "none") of \(Int(view.bounds.height))"
    if said != last {
      last = said
      colLog(said)
    }
  }

  private func wanted() -> CGRect? {
    let laptop = ProtoL.shared
    let stack = columns.stack
    guard let fold = laptop.fold, laptop.deck != nil, !laptop.keyboardUp, let top = stack.topSession else { return nil }
    // The Sidebar called back stands over both halves, so what is below goes while it is up.
    guard columns.displayMode == .secondaryOnly || columns.isCollapsed else { return nil }
    // A screen with no foot runs to the bottom of the half; a Session's field stands under what is below.
    let footed: Bool =
      switch stack.path.last {
      case .session, .landing: true
      default: false
      }
    let carried = footed ? laptop.composerHeight[top.route] ?? 0 : 0
    return CGRect(
      x: 0, y: fold.bottom, width: view.bounds.width, height: max(0, view.bounds.height - fold.bottom - carried))
  }

  private func raise() -> UIViewController {
    let host: UIViewController
    if let second {
      host = second
    } else {
      // The one view controller: out of the inspector column, which holds nothing while it stands here.
      if columns.viewController(for: .inspector) != nil { columns.setViewController(nil, for: .inspector) }
      host = columns.aside
      host.willMove(toParent: nil)
      host.view.removeFromSuperview()
      host.removeFromParent()
    }
    colLog("holder: raising \(second == nil ? "the inspector's own" : "a second") aside below the fold")
    addChild(host)
    view.addSubview(host.view)
    host.didMove(toParent: self)
    below = host
    return host
  }

  private func lower(_ host: UIViewController) {
    colLog("holder: taking the aside below the fold away")
    host.willMove(toParent: nil)
    host.view.removeFromSuperview()
    host.removeFromParent()
    below = nil
    // The inspector's own is the column's to hold again, where the reader has it shown and there is room.
    if second == nil { columns.given() }
  }
}
