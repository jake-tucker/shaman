// PROTOTYPE for #714, "What the inner display shows". Throwaway: never merge.
//
// Three variants of the listing and the Session it opens, on the app's real screens, switched by the floating pill
// or by `--inner <stack|two|three>` at launch:
//
//   stack  today's build, as the control: one navigation stack, the listing at its root
//   two    one NavigationSplitView: the listing beside the open Session
//   three  the same, and beside the conversation a third column holding a Turn's work or the tree of Sessions
//
// The pill's menu, or `--inner-empty <landing|nothing>`, chooses what the second column holds while no Session is
// open; `--inner-style <balanced|automatic|prominent>` the split view's style; `--inner-pill off` hides the pill for
// a screenshot.

import ShamanCore
import SwiftUI

/// What the prototype is showing, held for as long as the app runs.
@MainActor @Observable
final class Proto {
  static let shared = Proto()

  enum Variant: String, CaseIterable {
    case stack, two, three

    var name: String {
      switch self {
      case .stack: "A · today's stack"
      case .two: "B · two columns"
      case .three: "C · three columns"
      }
    }
  }

  /// What the second column holds while no Session is open.
  enum Empty: String, CaseIterable {
    case landing, nothing

    var name: String {
      switch self {
      case .landing: "the new-session screen"
      case .nothing: "nothing"
      }
    }
  }

  enum Style: String, CaseIterable {
    case balanced, automatic, prominent
  }

  var variant: Variant
  var empty: Empty
  var style: Style
  var pill: Bool
  /// Whether the window is of regular width, read outside the split view, whose columns may say otherwise.
  var regular = false
  /// The fold, while the phone is partly folded like a book: how far across the window it begins, and how wide the
  /// band kept clear for it is.
  var fold: Fold?

  struct Fold: Equatable {
    var leading: CGFloat
    var width: CGFloat
    /// How wide the window is, less its safe area.
    var window: CGFloat
  }

  /// Whether the window is wide enough to tile columns: the inner display in landscape, not in portrait.
  var wide = false
  /// Whether the third column takes the listing's place while it is shown, two columns at a time, as Apple's
  /// engineer says of Mail on a Duo; otherwise the system fits all three as it can.
  var asideHidesListing = true

  private init() {
    let arguments = ProcessInfo.processInfo.arguments
    func value(_ flag: String) -> String? {
      arguments.firstIndex(of: flag).flatMap { arguments.dropFirst($0 + 1).first }
    }
    variant = value("--inner").flatMap(Variant.init) ?? .two
    empty = value("--inner-empty").flatMap(Empty.init) ?? .landing
    style = value("--inner-style").flatMap(Style.init) ?? .balanced
    pill = value("--inner-pill") != "off"
    asideHidesListing = value("--inner-aside") != "beside"
  }

  /// Whether a Turn's work and the tree stand in a third column rather than being pushed or presented.
  var asides: Bool { variant == .three && regular && wide }
}

/// The floating pill: the variant shown between two arrows, and a menu for the rest. It drags out of the way.
struct PrototypePill: View {
  @Bindable var proto = Proto.shared
  @State private var moved = CGSize.zero
  @GestureState private var dragging = CGSize.zero

  var body: some View {
    HStack(spacing: 2) {
      Button {
        step(-1)
      } label: {
        Image(systemName: "chevron.left").frame(width: 30, height: 30)
      }
      Menu {
        Picker("While no session is open, show", selection: $proto.empty) {
          ForEach(Proto.Empty.allCases, id: \.self) { Text($0.name).tag($0) }
        }
        Toggle("Third column hides the listing", isOn: $proto.asideHidesListing)
        Picker("Split view style", selection: $proto.style) {
          ForEach(Proto.Style.allCases, id: \.self) { Text($0.rawValue).tag($0) }
        }
      } label: {
        Text(verbatim: proto.variant.name)
          .font(.caption.weight(.semibold))
          .lineLimit(1)
          .fixedSize()
      }
      Button {
        step(1)
      } label: {
        Image(systemName: "chevron.right").frame(width: 30, height: 30)
      }
    }
    .foregroundStyle(.white)
    .padding(.horizontal, 4)
    .background(Color.pink, in: Capsule())
    .shadow(radius: 4, y: 2)
    .offset(x: moved.width + dragging.width, y: moved.height + dragging.height)
    .gesture(
      DragGesture()
        .updating($dragging) { value, state, _ in state = value.translation }
        .onEnded { value in
          moved.width += value.translation.width
          moved.height += value.translation.height
        }
    )
    .accessibilityIdentifier("prototype.pill")
  }

  private func step(_ by: Int) {
    let all = Proto.Variant.allCases
    let at = all.firstIndex(of: proto.variant) ?? 0
    proto.variant = all[(at + by + all.count) % all.count]
  }
}

/// What the third column shows beside a Session's conversation.
@MainActor @Observable
final class AsideState {
  enum Root: Hashable {
    case work(String)
    case detail(String)
    case tree
  }

  /// Whether the reader has the column shown: their standing choice, kept from one Session to the next and through a
  /// fold.
  var wanted = false
  /// Whether the column is up now, as the system has it: the system takes it down through a fold by itself.
  var presented = false
  /// The Session's screen whose work it shows.
  var owner: SessionRoute?
  /// The work of the last Turn that Session's conversation holds.
  var lastTurn: String?
  var root: Root?
  /// What is pushed over the root within the column: an Activity's whole.
  var path: [Pushed] = []
  /// Whether the reader has chosen what it shows, after which the Session's own changes leave it alone.
  var chosen = false

  func show(_ root: Root) {
    self.root = root
    path = []
    chosen = true
    wanted = true
    presented = true
  }

  /// The Session's own button: hides the column where it is up, and shows it where it is not.
  func toggle() {
    let next = !presented
    wanted = next
    presented = next
  }
}

/// The third column: a Turn's work, with each Activity's whole pushed within it, or the tree of Sessions.
struct AsideColumn: View {
  let model: SessionModel
  @Bindable var state: AsideState
  var stack: SessionStack?
  private var lastTurn: String? { state.lastTurn }

  var body: some View {
    Group {
      switch state.root {
      case .tree:
        TreeSheet(model: model) { session in
          stack?.show(session, over: model.route)
        }
      case .work(let turn):
        NavigationStack(path: $state.path) {
          TurnWorkScreen(session: model, turn: turn)
            .navigationDestination(for: Pushed.self) { destination($0) }
        }
        .id(turn)
      case .detail(let activity):
        NavigationStack(path: $state.path) {
          ActivityDetailScreen(session: model, activity: activity)
            .navigationDestination(for: Pushed.self) { destination($0) }
        }
        .id(activity)
      case nil:
        ContentUnavailableView("Nothing yet", systemImage: "list.bullet")
      }
    }
    .safeAreaInset(edge: .bottom) {
      if model.frame?.tree != nil, lastTurn != nil {
        HStack(spacing: 8) {
          choice("Work", tree: false)
          choice("Sessions", tree: true)
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 6)
        .background(Color(.systemBackground))
      }
    }
  }

  private func choice(_ words: String, tree: Bool) -> some View {
    Button {
      if tree {
        state.show(.tree)
      } else if let lastTurn {
        state.show(.work(lastTurn))
      }
    } label: {
      Text(words)
        .font(.footnote.weight(.medium))
        .frame(maxWidth: .infinity)
        .padding(.vertical, 6)
        .background(
          (state.root == .tree) == tree ? Color.accentColor.opacity(0.18) : Color(.secondarySystemBackground),
          in: RoundedRectangle(cornerRadius: 8))
    }
    .buttonStyle(.plain)
  }

  @ViewBuilder
  private func destination(_ pushed: Pushed) -> some View {
    switch pushed {
    case .detail(_, let activity):
      ActivityDetailScreen(session: model, activity: activity)
    case .session(let route):
      // A Subagent's Session opens in the second column, not here.
      Color.clear.onAppear {
        state.path = []
        stack?.show(route.session, over: model.route)
      }
    case .work, .landing:
      EmptyView()
    }
  }
}

/// PROTOTYPE: a line in a file on the Mac, from the simulator.
func protoLog(_ line: String) {
  #if targetEnvironment(simulator)
    let url = URL(fileURLWithPath: "/tmp/duo714/app.log")
    let data = Data(("\(Date().timeIntervalSince1970.truncatingRemainder(dividingBy: 1000)) " + line + "\n").utf8)
    if let handle = try? FileHandle(forWritingTo: url) {
      handle.seekToEndOfFile()
      handle.write(data)
      try? handle.close()
    } else {
      try? data.write(to: url)
    }
  #endif
}

/// PROTOTYPE: asks for the fold's reserved region, twice a second as #712's probe did, since nothing says a
/// `GeometryReader` is drawn again as the fold comes and goes.
@available(iOS 27.1, *)
struct FoldReader: View {
  var body: some View {
    TimelineView(.periodic(from: .now, by: 0.5)) { _ in
      GeometryReader { proxy in
        let region = proxy.reservedRegions(kind: .division).first { $0.isActive && $0.frame.height > $0.frame.width }
        let fold = region.map {
          Proto.Fold(leading: $0.frame.minX, width: $0.frame.width, window: proxy.size.width)
        }
        Color.clear.onChange(of: fold, initial: true) { _, fold in
          protoLog("fold \(String(describing: fold))")
          Proto.shared.fold = fold
        }
      }
    }
    .allowsHitTesting(false)
    .accessibilityHidden(true)
  }
}
