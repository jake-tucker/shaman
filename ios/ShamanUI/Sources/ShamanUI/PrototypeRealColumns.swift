// PROTOTYPE for #731, "Whether the app's real screens stand under UIKit's navigation controllers". Throwaway: never
// merge.
//
// #725's container, found over placeholder screens, with the app's real screens in it: UIKit's split view hosted
// directly, a navigation controller in the Sidebar's column and the main view's, each screen a hosting controller,
// and the Aside in the inspector column. `SessionStack.path` stays the one value that says what is open: a change to
// it is asked of the main view's navigation controller, and what the reader pops is read back from that controller's
// screens as it says it did show one.
//
// `--container <uikit|swiftui>` chooses the container, and `--tree-badge` the tree button #716 decided. So that the
// UI suite runs unchanged, a simulator also reads both words from /tmp/duo732/config-<its UDID>, or
// /tmp/duo732/config, where no argument says. Every event is a line in /tmp/duo732/app.log.

import ShamanCore
import SwiftUI
import UIKit

@MainActor
enum Columns {
  private static let words: [String] = {
    let arguments = ProcessInfo.processInfo.arguments
    var words: [String] = []
    if let flag = arguments.firstIndex(of: "--container"), let named = arguments.dropFirst(flag + 1).first {
      words.append(named)
    }
    if arguments.contains("--tree-badge") { words.append("badge") }
    #if targetEnvironment(simulator)
      let udid = ProcessInfo.processInfo.environment["SIMULATOR_UDID"] ?? ""
      for name in ["config-\(udid)", "config"] {
        if let said = try? String(contentsOfFile: "/tmp/duo732/\(name)", encoding: .utf8) {
          words += said.split(whereSeparator: \.isWhitespace).map(String.init)
          break
        }
      }
    #endif
    return words
  }()

  /// Whether the real screens stand in UIKit's split view; otherwise in #714's SwiftUI one.
  static let uikit = !(words.first { $0 == "uikit" || $0 == "swiftui" || $0 == "stack" }.map { $0 != "uikit" } ?? false)
  /// Whether today's one navigation stack is asked for, as the control.
  static let stack = words.first { $0 == "uikit" || $0 == "swiftui" || $0 == "stack" } == "stack"
  /// Whether the tree button is a symbol wearing a badge, as #716 decided.
  static let treeBadge = words.contains("badge")
  /// Whether the Sidebar's navigation controller is asked to show its toolbar; `notoolbar` leaves it as UIKit has it.
  static let toolbar = !words.contains("notoolbar")
  /// Whether a hosted screen keeps its `NavigationLink`s and `navigationDestination`, to see what they do with no
  /// `NavigationStack` over them.
  static let links = words.contains("links")
  /// Whether a hosted screen is left to pop itself by SwiftUI's `dismiss`, to see what it does.
  static let dismisses = words.contains("dismiss")
}

func colLog(_ line: String) {
  #if targetEnvironment(simulator)
    let url = URL(fileURLWithPath: "/tmp/duo732/app.log")
    let stamp = String(format: "%07.3f", Date().timeIntervalSince1970.truncatingRemainder(dividingBy: 1000))
    let data = Data("\(stamp) \(line)\n".utf8)
    if let handle = try? FileHandle(forWritingTo: url) {
      handle.seekToEndOfFile()
      handle.write(data)
      try? handle.close()
    } else {
      try? data.write(to: url)
    }
  #endif
}

/// What a hosted screen asks of the main view's navigation controller, where a `NavigationLink`, a
/// `navigationDestination` or `dismiss` would have asked a `NavigationStack`.
struct Pushes {
  /// Pushes a screen the path names.
  let value: @MainActor (Pushed) -> Void
  /// Pushes a screen the path does not name, as the Workspace Picker.
  let screen: @MainActor (AnyView) -> Void
  /// Pops the screen on top.
  let pop: @MainActor () -> Void
}

extension EnvironmentValues {
  @Entry var pushes: Pushes? = nil
}

/// A row that pushes: a `NavigationLink` inside a `NavigationStack`, and a button asking the navigation controller
/// where the screen is hosted in one.
struct PushLink<Label: View>: View {
  let value: Pushed
  @ViewBuilder let label: Label
  @Environment(\.pushes) private var pushes

  var body: some View {
    if let pushes, !Columns.links {
      Button {
        pushes.value(value)
      } label: {
        HStack(spacing: 8) {
          label
          Spacer(minLength: 0)
          Image(systemName: "chevron.right")
            .font(.footnote.weight(.semibold))
            .foregroundStyle(.tertiary)
            .accessibilityHidden(true)
        }
        .contentShape(Rectangle())
      }
      .buttonStyle(.plain)
    } else {
      NavigationLink(value: value) { label }
    }
  }
}

/// A screen in a navigation controller: what the path names it, or nothing for one the path does not hold.
final class ScreenHost: UIHostingController<AnyView> {
  var key: Pushed?
  /// The new-session screen the main view holds while nothing is open, which becomes what is open as it begins.
  let resting: LandingRoute?
  let name: String

  init(name: String, key: Pushed? = nil, resting: LandingRoute? = nil, _ view: AnyView) {
    self.name = name
    self.key = key
    self.resting = resting
    super.init(rootView: view)
  }

  @available(*, unavailable) required init?(coder: NSCoder) { fatalError() }
}

/// The three columns over the app's real screens.
final class RealColumns: UISplitViewController, UISplitViewControllerDelegate, UINavigationControllerDelegate {
  /// The screens, each already given the environment a hosted screen reads.
  struct Screens {
    let listing: () -> AnyView
    let pushed: (Pushed, Pushes) -> AnyView
    let resting: (LandingRoute, Pushes) -> AnyView
    let aside: () -> AnyView
    let wrap: (AnyView, Pushes) -> AnyView
  }

  let stack: SessionStack
  private var screens: Screens
  let sidebar = UINavigationController()
  let main = UINavigationController()
  private let listing: ScreenHost
  private(set) lazy var aside: UIViewController = UIHostingController(rootView: screens.aside())
  /// PROTOTYPE #732: told as the split view changes which columns stand, and asked whether the Aside's view
  /// controller stands below the fold, lent to whoever lays that half out.
  var modeChanged: (() -> Void)?
  var lent: (() -> Bool)?
  /// How often the reader came back to the listing while it stood alone.
  private(set) var returned = 0

  /// The least the main view keeps beside the Aside, and the least the Aside takes: given to the split view, and
  /// the app's own rule for whether the Aside has room.
  static let mainMinimum: CGFloat = 400
  static let asideMinimum: CGFloat = 290

  init(stack: SessionStack, screens: Screens) {
    self.stack = stack
    self.screens = screens
    listing = ScreenHost(name: "listing", screens.listing())
    super.init(style: .doubleColumn)
    delegate = self
    sidebar.delegate = self
    main.delegate = self
    sidebar.viewControllers = [listing]
    main.viewControllers = stack.path.isEmpty ? [resting()] : stack.path.map(host)
    setViewController(sidebar, for: .primary)
    setViewController(main, for: .secondary)
    minimumSecondaryColumnWidth = Self.mainMinimum
    minimumInspectorColumnWidth = Self.asideMinimum
    preferredSplitBehavior = .tile
    preferredDisplayMode = .oneBesideSecondary
    stack.pathChanged = { [weak self] in
      self?.sync()
      self?.reconcile("the path")
    }
    stack.aside.changed = { [weak self] in self?.reconcile("the reader") }
    colLog("columns made, path \(stack.path.map(\.tag))")
  }

  @available(*, unavailable) required init?(coder: NSCoder) { fatalError() }

  /// What comes down from above the container, as the place new Sessions begin in, reaches the listing again.
  func update(_ screens: Screens) {
    self.screens = screens
    listing.rootView = screens.listing()
  }

  private var pushes: Pushes {
    Pushes(
      value: { [weak self] pushed in self?.stack.path.append(pushed) },
      screen: { [weak self] view in
        guard let self else { return }
        colLog("push a screen the path does not hold")
        main.pushViewController(ScreenHost(name: "unkeyed", screens.wrap(view, pushes)), animated: true)
      },
      pop: { [weak self] in
        colLog("pop asked")
        self?.main.popViewController(animated: true)
      })
  }

  private func host(_ pushed: Pushed) -> ScreenHost {
    ScreenHost(name: pushed.tag, key: pushed, screens.pushed(pushed, pushes))
  }

  private func resting() -> ScreenHost {
    let route = stack.rootLanding
    return ScreenHost(name: "resting", resting: route, screens.resting(route, pushes))
  }

  private func keys(_ nav: UINavigationController) -> [Pushed] {
    nav.viewControllers.compactMap { ($0 as? ScreenHost)?.key }
  }

  private func names(_ nav: UINavigationController) -> [String] {
    nav.viewControllers.map { ($0 as? ScreenHost)?.name ?? "\(type(of: $0))" }
  }

  private var where_: String {
    "collapsed=\(isCollapsed) mode=\(displayMode.rawValue) sidebar=\(names(sidebar)) main=\(names(main)) aside=\(isShowing(.inspector))"
  }

  // MARK: The path

  /// Asks the main view's navigation controller for what the path now says, where it does not hold it already.
  private func sync() {
    let path = stack.path
    // Begun from the main view's resting screen, that screen is what is open.
    if case .landing(let route)? = path.first, path.count == 1, let root = main.viewControllers.first as? ScreenHost,
      root.key == nil, root.resting == route
    {
      root.key = path.first
      colLog("the resting screen became what is open")
    }
    let held = keys(main)
    guard held != path else { return }
    colLog("sync \(held.map(\.tag)) -> \(path.map(\.tag)) \(where_)")
    guard let first = path.first else {
      // Nothing is open: the main view rests, and alone on screen the listing stands.
      main.setViewControllers([resting()], animated: false)
      if isCollapsed, sidebar.topViewController !== listing {
        colLog("back to the listing, asked by the path")
        show(.primary)
      }
      return
    }
    let hosts = main.viewControllers.compactMap { $0 as? ScreenHost }
    let screens = path.map { pushed in hosts.first { $0.key == pushed } ?? host(pushed) }
    if held.first == first {
      // Within what is open: UIKit pushes where the top is new and pops where it was beneath.
      main.setViewControllers(screens, animated: true)
    } else {
      main.setViewControllers(screens, animated: false)
      show(.secondary)
    }
  }

  // MARK: The Aside

  /// Whether the system stands the Aside beside the main view here.
  var tiles: Bool { !isCollapsed && view.bounds.width >= Self.mainMinimum + Self.asideMinimum }

  /// Shows the Aside where the reader has it shown, a Session is open and there is room, at a moment the system named.
  private func reconcile(_ why: String) {
    let state = stack.aside
    let open = stack.topSession != nil
    let want = state.wanted && open && tiles
    if want {
      // The column holds the Aside only while the reader has it shown: a view controller set in it is shown.
      if lent?() == true {
        colLog("the aside is lent below the fold; the column waits")
        said()
        return
      }
      if viewController(for: .inspector) == nil { setViewController(aside, for: .inspector) }
      if !isShowing(.inspector) {
        colLog("showing the aside at \(why)")
        show(.inspector)
      }
    } else if isShowing(.inspector), !state.wanted || !open {
      colLog("hiding the aside at \(why)")
      hide(.inspector)
    }
    said()
  }

  /// Says what the screens read of the container: whether columns stand, whether the Aside has room and is on
  /// screen, and whether the listing is the one to say a notice.
  private func said() {
    guard isViewLoaded else { return }
    let proto = Proto.shared
    let shown = isShowing(.inspector)
    if stack.aside.presented != shown { stack.aside.presented = shown }
    if proto.regular != !isCollapsed { proto.regular = !isCollapsed }
    if proto.wide != tiles { proto.wide = tiles }
    // Alone on screen, or standing over the main view, the listing says a notice; beside it, the main view does.
    let over = displayMode == .oneOverSecondary || displayMode == .twoOverSecondary
    let alone = isCollapsed || over
    if proto.listingSaysNotice != alone { proto.listingSaysNotice = alone }
  }

  /// PROTOTYPE #732: the Aside's view controller given back by whoever stood it below the fold.
  func given() { reconcile("the aside given back") }

  /// PROTOTYPE #732: a second hosted Aside, over the same state.
  func secondAside() -> UIViewController { UIHostingController(rootView: screens.aside()) }

  func splitViewController(_ svc: UISplitViewController, willChangeTo mode: UISplitViewController.DisplayMode) {
    colLog("display mode -> \(mode.rawValue)")
    modeChanged?()
  }

  // MARK: What the system says

  override func viewDidAppear(_ animated: Bool) {
    super.viewDidAppear(animated)
    colLog("columns appeared \(where_)")
    reconcile("appearing")
  }

  func splitViewController(
    _ svc: UISplitViewController, topColumnForCollapsingToProposedTopColumn proposed: UISplitViewController.Column
  ) -> UISplitViewController.Column {
    let top: UISplitViewController.Column = stack.path.isEmpty ? .primary : .secondary
    colLog("collapsing, top \(top == .primary ? "sidebar" : "main") \(where_)")
    if viewController(for: .inspector) != nil {
      // Alone on screen the system would stand the inspector as a sheet; with nothing in the column there is
      // nothing to stand.
      setViewController(nil, for: .inspector)
    }
    return top
  }

  func splitViewControllerDidCollapse(_ svc: UISplitViewController) {
    colLog("didCollapse \(where_)")
    said()
  }

  func splitViewControllerDidExpand(_ svc: UISplitViewController) {
    colLog("didExpand \(where_)")
    reconcile("didExpand")
  }

  func navigationController(_ nav: UINavigationController, willShow vc: UIViewController, animated: Bool) {
    // A hosted screen's bottom bar items reach its navigation controller's toolbar, which stands hidden until it is
    // asked for: shown for the listing, and not for the main view pushed over it while collapsed.
    if nav === sidebar, Columns.toolbar {
      let items = vc.toolbarItems?.count ?? -1
      colLog("nav sidebar willShow \((vc as? ScreenHost)?.name ?? "\(type(of: vc))"), toolbar items \(items)")
      sidebar.setToolbarHidden(vc !== listing, animated: animated)
    }
  }

  func navigationController(_ nav: UINavigationController, didShow vc: UIViewController, animated: Bool) {
    let which = nav === main ? "main" : "sidebar"
    colLog("nav \(which) didShow \((vc as? ScreenHost)?.name ?? "\(type(of: vc))") \(where_)")
    let narrow = (view.window ?? view).traitCollection.horizontalSizeClass == .compact
    if nav === sidebar, isCollapsed, narrow, sidebar.topViewController === vc, sidebar.viewControllers.count == 1,
      !stack.path.isEmpty
    {
      // The listing's own navigation controller has shown the listing, alone on screen: the reader came back.
      returned += 1
      colLog("the reader returned to the listing, \(returned)")
      stack.path = []
    }
    if nav === main {
      // What the reader popped is read back.
      let held = keys(main)
      let path = stack.path
      if !held.isEmpty, held.count < path.count, Array(path.prefix(held.count)) == held {
        colLog("read back \(path.map(\.tag)) -> \(held.map(\.tag))")
        stack.path = held
      }
      reconcile("the main view showing a screen")
    }
  }

  override func viewWillTransition(to size: CGSize, with coordinator: any UIViewControllerTransitionCoordinator) {
    colLog("viewWillTransition to \(Int(size.width))x\(Int(size.height)) \(where_)")
    super.viewWillTransition(to: size, with: coordinator)
    coordinator.animate(alongsideTransition: nil) { [weak self] _ in self?.reconcile("the size transition's end") }
  }

  override func viewDidLayoutSubviews() {
    super.viewDidLayoutSubviews()
    said()
  }
}

struct RealColumnsHost: UIViewControllerRepresentable {
  let stack: SessionStack
  let screens: RealColumns.Screens

  func makeUIViewController(context: Context) -> UIViewController {
    let columns = RealColumns(stack: stack, screens: screens)
    guard LaptopC.over else { return columns }
    // PROTOTYPE #732: the split view in a plain container, which lays the Aside over the half below the fold.
    return LaptopHolder(columns: columns, second: LaptopC.below == .overSecond ? columns.secondAside() : nil)
  }

  func updateUIViewController(_ controller: UIViewController, context: Context) {
    ((controller as? LaptopHolder)?.columns ?? controller as? RealColumns)?.update(screens)
  }
}

/// The Aside's column: the Session on top of the main view's, and nothing while none is open.
struct AsideRoot: View {
  let stack: SessionStack

  var body: some View {
    if let session = stack.topSession {
      AsideColumn(model: session, state: stack.aside, stack: stack)
        .id(session.route)
    }
  }
}
