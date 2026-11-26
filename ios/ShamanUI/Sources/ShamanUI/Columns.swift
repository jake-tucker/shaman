import SwiftUI
import UIKit

/// The columns every iPhone's screens stand in: UIKit's split view, the listing in its Sidebar's column and what is
/// open in its main view's, with its inspector column for the Aside. A navigation controller stands in each of the
/// first two, and each screen is hosted in one. Where there is room for one column alone, as on an ordinary iPhone
/// upright, the split view collapses, and what is open is pushed over the listing.
///
/// What is open is `SessionStack.path` and nothing else. A change to it is asked of the main view's navigation
/// controller, and what the reader pops there is read back as that controller says it did show a screen. While nothing
/// is open the main view holds the new-session screen at rest, and where the system stands the main view alone then, as
/// on the inner display in portrait, the Sidebar is asked for over it.
///
/// A notice is said once in the window: by the listing where it stands alone or over the main view, and by the main
/// view otherwise, as the columns say in `SessionStack.noticeSaid`. The Aside never says one.
final class Columns: UISplitViewController {
  /// The screens the columns hold, and what each is handed.
  struct Screens {
    /// The listing.
    let listing: AnyView
    /// The screen the path names.
    let open: @MainActor (Pushed) -> AnyView
    /// The new-session screen at rest.
    let rest: @MainActor (LandingRoute) -> AnyView
    /// What the Aside holds.
    let aside: AnyView
    /// Hands a screen what it reads of the environment, as a hosting controller made by hand inherits none of it.
    let handed: @MainActor (AnyView) -> AnyView
  }

  private let stack: SessionStack
  private var screens: Screens
  private let sidebar = UINavigationController()
  private let main = UINavigationController()
  private let listing: HostedScreen
  private let aside: HostedScreen
  /// The screens the path names that the main view was last asked to hold, the one on top last. Asked in the middle of
  /// a push or a pop, it holds them only once that has ended.
  private var asked: [HostedScreen] = []
  /// Whether it has yet to hold them.
  private var owed = false
  /// Whether the listing, standing alone, was asked to give way to the main view and has yet to.
  private var givingWay = false
  /// Whether anything was open as the path was last written.
  private var wasOpen: Bool
  /// Whether the Sidebar is to be asked for once what moves now has ended.
  private var sidebarOwed = false

  init(stack: SessionStack, screens: Screens) {
    self.stack = stack
    self.screens = screens
    listing = HostedScreen(screens.handed(Self.said(screens.listing, listing: true, in: stack)))
    aside = HostedScreen(screens.handed(AnyView(screens.aside.environment(\.saysNotices, false))))
    wasOpen = !stack.path.isEmpty
    super.init(style: .doubleColumn)
    delegate = self
    sidebar.delegate = self
    main.delegate = self
    sidebar.viewControllers = [listing]
    asked = stack.path.map(hosted)
    main.viewControllers = asked.isEmpty ? [resting()] : asked
    setViewController(sidebar, for: .primary)
    setViewController(main, for: .secondary)
    // Set once, here. The main view is given no least width: with one, the split view will not divide the Sidebar and
    // the main view at a fold, and the mode set again then stands the one over the other (#733).
    preferredDisplayMode = .oneBesideSecondary
    preferredSplitBehavior = .tile
    minimumInspectorColumnWidth = Self.leastAside
    stack.moved = { [weak self] in
      guard let self else { return }
      let emptied = wasOpen && stack.path.isEmpty
      wasOpen = !stack.path.isEmpty
      follow()
      standAside()
      if emptied {
        standSidebar()
      }
    }
    stack.aside.changed = { [weak self] in self?.standAside() }
    aside.appearing = { [weak self] in
      self?.stack.aside.appearances += 1
      self?.sayStanding()
    }
    aside.disappeared = { [weak self] in self?.sayStanding() }
  }

  @available(*, unavailable)
  required init?(coder: NSCoder) {
    fatalError("made in code alone")
  }

  /// Takes the screens again as what they were made of above the container changes, as where new Sessions begin does:
  /// the listing is drawn again with it, its state kept, and what is open is as it was made.
  func take(_ screens: Screens) {
    self.screens = screens
    listing.rootView = screens.handed(Self.said(screens.listing, listing: true, in: stack))
  }

  // MARK: Notices

  /// `screen`, saying a notice where the listing does, where it is the `listing`, or where the main view does.
  private static func said(_ screen: AnyView, listing: Bool, in stack: SessionStack) -> AnyView {
    AnyView(Saying(stack: stack, listing: listing, screen: screen))
  }

  /// Says which screen says a notice, as the split view stands, or is to stand in `mode` where one is given.
  private func sayWhoSays(_ mode: DisplayMode? = nil) {
    let said: NoticeSaid =
      isCollapsed ? .oneColumn : (mode ?? displayMode) == .oneOverSecondary ? .listing : .mainView
    if stack.noticeSaid != said {
      stack.noticeSaid = said
    }
  }

  // MARK: The Aside

  /// The least the main view keeps beside the Aside, and the least the Aside is given.
  private static let leastMain: CGFloat = 400
  private static let leastAside: CGFloat = 290

  /// Whether the Aside has room beside the main view. The split view's own word is not enough: it would stand one over
  /// the main view where the window is too narrow for both.
  private var asideHasRoom: Bool {
    !isCollapsed && view.bounds.width >= Self.leastMain + Self.leastAside
  }

  /// Says whether the Aside has room, as the split view stands now.
  private func sayRoom() {
    let room = asideHasRoom
    if stack.aside.hasRoom != room {
      stack.aside.hasRoom = room
    }
  }

  /// Says whether the Aside stands beside the main view, as the split view is asked now.
  private func sayStanding() {
    let stands = viewController(for: .inspector) === aside && isShowing(.inspector)
    if stack.aside.standsBeside != stands {
      stack.aside.standsBeside = stands
    }
  }

  /// Whether the Aside is to stand: the reader has it shown, a Session is open and there is room.
  private var asideIsToStand: Bool {
    stack.aside.shown && stack.openSession != nil && asideHasRoom
  }

  /// Has the inspector column hold the Aside, and show it, while it is to stand, and hold nothing otherwise, as what is
  /// set in the column of a split view that is not collapsed is shown at once. With it shown the split view itself puts
  /// the Sidebar behind its button.
  private func standAside() {
    guard isViewLoaded, view.window != nil else { return }
    defer { sayStanding() }
    sayRoom()
    if asideIsToStand {
      if viewController(for: .inspector) == nil {
        setViewController(aside, for: .inspector)
      }
      if !isShowing(.inspector) {
        show(.inspector)
      }
    } else if isShowing(.inspector) {
      hide(.inspector)
      // The column gives it up once it has gone, which the split view tells its delegate nothing of.
      guard let hiding = transitionCoordinator else { return emptyAside() }
      hiding.animate(alongsideTransition: nil) { [weak self] _ in
        guard let self else { return }
        if asideIsToStand {
          standAside()
        } else {
          emptyAside()
        }
      }
    } else {
      emptyAside()
    }
  }

  /// Takes the Aside out of its column.
  private func emptyAside() {
    if viewController(for: .inspector) != nil {
      setViewController(nil, for: .inspector)
    }
    sayStanding()
  }

  // MARK: The Sidebar while nothing is open

  /// Asks for the Sidebar's column where the split view stands the main view alone while nothing is open, as it does on
  /// the inner display in portrait, where it will not tile: the system stands the Sidebar over the dimmed main view, and
  /// puts it away again as something opens, or as the reader taps beside it. It is never stood over a field that holds
  /// the keyboard, as the screen at rest is open once its field is touched. Asked only at the four moments the system
  /// names, as the columns appear, expand and end a change of size and as the path empties, so a Sidebar put away
  /// stays away until the next. Not from under anything moving, as the Aside going, a push or a pop, or a sheet over
  /// the columns going: once it has ended, where it is still to be asked for then.
  private func standSidebar() {
    guard isViewLoaded, view.window != nil, !sidebarOwed else { return }
    // Owed to the first of what moves that will say it has ended. One that will not cannot be waited for.
    for moving in moving {
      sidebarOwed = true
      let waits = moving.animate(alongsideTransition: nil) { [weak self] _ in
        guard let self else { return }
        sidebarOwed = false
        standSidebar()
      }
      if waits { return }
      sidebarOwed = false
    }
    guard !isCollapsed, stack.path.isEmpty, displayMode == .secondaryOnly else { return }
    #if DEBUG
      askingForTheSidebar?()
    #endif
    show(.primary)
  }

  /// What moves in the columns now, or over them: the split view itself, either column's navigation controller, or a
  /// sheet over them.
  private var moving: [any UIViewControllerTransitionCoordinator] {
    [
      transitionCoordinator, main.transitionCoordinator, sidebar.transitionCoordinator,
      presentedViewController?.transitionCoordinator,
    ].compactMap { $0 }
  }

  #if DEBUG
    /// Told just before the Sidebar is asked for, for a test to read what moves then.
    var askingForTheSidebar: (() -> Void)?
  #endif

  override func viewDidAppear(_ animated: Bool) {
    super.viewDidAppear(animated)
    standAside()
    standSidebar()
  }

  override func viewDidLayoutSubviews() {
    super.viewDidLayoutSubviews()
    sayRoom()
    sayStanding()
    sayWhoSays()
  }

  override func viewWillTransition(to size: CGSize, with coordinator: any UIViewControllerTransitionCoordinator) {
    // Turned while partly folded, the fold read of the old size is not laid out against at the new.
    stack.pose.turns(to: size)
    // Whether the Sidebar stood, read before the split view hears of the new size, as it changes its display mode as it
    // does: put away by its own button, which the system keeps, it is not asked for after the change (#733).
    let sidebarStood = isCollapsed || displayMode != .secondaryOnly
    super.viewWillTransition(to: size, with: coordinator)
    coordinator.animate(alongsideTransition: nil) { [weak self] _ in
      self?.standAside()
      if sidebarStood {
        self?.standSidebar()
      }
    }
  }

  // MARK: What is open

  private func hosted(_ pushed: Pushed) -> HostedScreen {
    host(screens.open(pushed), named: pushed)
  }

  private func resting() -> HostedScreen {
    host(screens.rest(stack.rest), atRest: stack.rest)
  }

  private func host(_ screen: AnyView, named: Pushed? = nil, atRest: LandingRoute? = nil) -> HostedScreen {
    let push = PushScreenAction { [weak self] screen in
      guard let self else { return }
      main.pushViewController(host(screen), animated: true)
    }
    return HostedScreen(
      AnyView(screens.handed(Self.said(screen, listing: false, in: stack)).environment(\.pushScreen, push)),
      named: named, atRest: atRest)
  }

  /// The screens the path names that the main view holds now.
  private var held: [HostedScreen] {
    main.viewControllers.compactMap { $0 as? HostedScreen }.filter { $0.named != nil }
  }

  /// Whether the listing stands alone, with nothing over it in the Sidebar's own stack.
  private var listingAlone: Bool { isCollapsed && sidebar.viewControllers == [listing] }

  /// Has the columns hold what the path says: the screens the main view holds already are kept, and the rest pushed or
  /// popped.
  private func follow() {
    let path = stack.path
    guard let first = path.first else {
      asked = []
      owed = false
      // Alone on screen the listing is returned to first, and the main view rests once it has gone.
      if isCollapsed, !listingAlone || givingWay {
        // But not from under a push or a pop within the main view, given up or carried through, which UIKit's bars do
        // not bear: once that has ended.
        if let moving = main.transitionCoordinator {
          moving.animate(alongsideTransition: nil) { [weak self] _ in
            guard let self, stack.path.isEmpty else { return }
            follow()
          }
          return
        }
        givingWay = false
        show(.primary)
      } else {
        rest()
      }
      return
    }
    if case .landing(let route) = first, let root = main.viewControllers.first as? HostedScreen, root.atRest == route {
      // The screen at rest is what is open now, as it stands.
      root.named = first
      root.atRest = nil
      asked = [root]
    }
    let kept = zip(asked, path).prefix { $0.named == $1 }.count
    if kept < asked.count || kept < path.count || owed {
      // Over the same root UIKit pushes where the top is new, and pops where it was beneath.
      asked = asked.prefix(kept) + path.dropFirst(kept).map(hosted)
      main.setViewControllers(asked, animated: kept > 0 && view.window != nil)
      owed = held != asked
    }
    // Shown in the listing's place where it stands alone, and from under it where the system stands it over the main
    // view, which puts it away.
    if kept == 0 || listingAlone || displayMode == .oneOverSecondary {
      givingWay = listingAlone
      show(.secondary)
    }
  }

  /// Has the main view hold the new-session screen at rest, as it does while nothing is open.
  private func rest() {
    guard (main.viewControllers.first as? HostedScreen)?.atRest != stack.rest else { return }
    main.setViewControllers([resting()], animated: false)
  }

  /// Takes back what the reader popped of what is open, the main view having held it.
  private func readBack() {
    let held = held
    if held == asked {
      owed = false
    }
    guard !owed, !held.isEmpty, held.count < asked.count, asked.starts(with: held) else { return }
    asked = held
    stack.path = held.compactMap(\.named)
  }

  /// Closes what is open where the reader has returned to the listing, as its navigation controller says by having
  /// shown it alone, nothing having been opened meanwhile. Expanding shows it there too, but the window's width is
  /// regular by then. The split view cannot be asked, as it says the main view went where a screen is popped within it.
  private func returned() {
    guard listingAlone, !givingWay, (view.window ?? view).traitCollection.horizontalSizeClass == .compact else {
      return
    }
    if stack.path.isEmpty {
      rest()
    } else {
      stack.path = []
    }
  }
}

extension Columns: UINavigationControllerDelegate {
  func navigationController(
    _ controller: UINavigationController, willShow screen: UIViewController, animated: Bool
  ) {
    guard controller === sidebar else { return }
    // The listing's foot reaches the controller's toolbar, which stays hidden until it is asked for.
    controller.setToolbarHidden(screen !== listing, animated: animated)
  }

  func navigationController(_ controller: UINavigationController, didShow screen: UIViewController, animated: Bool) {
    if controller === main {
      readBack()
    } else if screen === listing {
      returned()
    } else {
      givingWay = false
      // Closed as it was being shown.
      if stack.path.isEmpty {
        follow()
      }
    }
  }
}

extension Columns: UISplitViewControllerDelegate {
  func splitViewController(
    _ columns: UISplitViewController, topColumnForCollapsingToProposedTopColumn proposed: UISplitViewController.Column
  ) -> UISplitViewController.Column {
    givingWay = false
    // Alone on screen the system would stand it as a sheet.
    emptyAside()
    return stack.path.isEmpty ? .primary : .secondary
  }

  func splitViewController(_ columns: UISplitViewController, willChangeTo mode: DisplayMode) {
    sayWhoSays(mode)
  }

  func splitViewControllerDidCollapse(_ columns: UISplitViewController) {
    sayWhoSays()
  }

  func splitViewControllerDidExpand(_ columns: UISplitViewController) {
    sayWhoSays()
    givingWay = false
    // Expanding as the listing is being returned to, the main view is to rest beside it.
    follow()
    standSidebar()
  }
}

/// A screen the columns host, told whether it says a notice as they stand: the listing, or one of the main view's.
private struct Saying: View {
  let stack: SessionStack
  let listing: Bool
  let screen: AnyView

  var body: some View {
    screen.environment(\.saysNotices, stack.saysNotices(listing: listing))
  }
}

/// A screen hosted in a column.
final class HostedScreen: UIHostingController<AnyView> {
  /// What the path names it; nothing for the listing, for the screen at rest, and for one pushed that it does not name.
  var named: Pushed?
  /// The new-session screen it is, while at rest.
  var atRest: LandingRoute?
  /// Told each time it comes on screen.
  var appearing: (() -> Void)?
  /// Told each time it has left the screen.
  var disappeared: (() -> Void)?

  init(_ screen: AnyView, named: Pushed? = nil, atRest: LandingRoute? = nil) {
    self.named = named
    self.atRest = atRest
    super.init(rootView: screen)
  }

  @available(*, unavailable)
  required init?(coder: NSCoder) {
    fatalError("made in code alone")
  }

  override func viewIsAppearing(_ animated: Bool) {
    super.viewIsAppearing(animated)
    appearing?()
  }

  override func viewDidDisappear(_ animated: Bool) {
    super.viewDidDisappear(animated)
    disappeared?()
  }
}

/// Pushes a screen the path does not name over the main view's others, which `dismiss` pops, where a
/// `navigationDestination` would push it: in a hosted screen there is no `NavigationStack` for one.
struct PushScreenAction {
  fileprivate let push: @MainActor (AnyView) -> Void

  fileprivate init(_ push: @escaping @MainActor (AnyView) -> Void) {
    self.push = push
  }

  @MainActor
  func callAsFunction(_ screen: some View) {
    push(AnyView(screen))
  }
}

extension EnvironmentValues {
  /// Pushes a screen the path does not name, in a screen the columns host.
  @Entry var pushScreen = PushScreenAction { _ in }
}

/// `Columns` as a view.
struct ColumnsView: UIViewControllerRepresentable {
  let stack: SessionStack
  let screens: Columns.Screens

  func makeUIViewController(context: Context) -> Columns {
    Columns(stack: stack, screens: screens)
  }

  func updateUIViewController(_ columns: Columns, context: Context) {
    columns.take(screens)
  }
}
