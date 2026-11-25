import SwiftUI
import Testing
import UIKit

@testable import ShamanUI

/// The columns in a window of the app's own scene, as wide or as narrow as a test asks, so that they collapse, expand
/// and push as they do on a phone.
@MainActor private final class HostedColumns {
  let window: UIWindow
  let stack: SessionStack
  let columns: Columns

  /// The columns over a path of `open`, in a window with room for one column alone, or for several where it is `wide`.
  init(open: [Pushed] = [], wide: Bool = false) async throws {
    let scene = try #require(UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }.first)
    window = UIWindow(windowScene: scene)
    stack = SessionStack(core: FakeCore(.paired), path: open)
    let stack = stack
    let screen = { (words: String) in AnyView(Text(words).navigationTitle(words)) }
    columns = Columns(
      stack: stack,
      screens: Columns.Screens(
        listing: screen("Listing"), open: { screen("\($0)") },
        rest: { route in
          AnyView(NewSessionScreen(model: stack.landing(for: route), stack: stack, atRest: route))
        },
        aside: AnyView(AsideScreen(stack: stack)), handed: { $0 }))
    size(wide: wide)
    window.rootViewController = columns
    window.windowLevel = .alert + 1
    window.makeKeyAndVisible()
    try await until { self.columns.isCollapsed != wide }
    try await settle()
  }

  var sidebar: UINavigationController { columns.viewController(for: .primary) as! UINavigationController }
  var main: UINavigationController { columns.viewController(for: .secondary) as! UINavigationController }
  /// What the path names each screen the main view holds, the one on top last.
  var held: [Pushed?] { main.viewControllers.map { ($0 as! HostedScreen).named } }
  /// Whether the listing stands alone.
  var listingAlone: Bool { columns.isCollapsed && sidebar.viewControllers.count == 1 }

  /// Gives the window room for one column alone, or for several.
  func size(wide: Bool) {
    size(width: wide ? 951 : 402, regular: wide)
  }

  /// Gives the window `width` points, of regular width or compact. The columns are told of it as a turn tells them, as
  /// setting a window's frame does not.
  func size(width: CGFloat, regular: Bool) {
    let size = CGSize(width: width, height: 669)
    let transition = SizeTransition()
    columns.viewWillTransition(to: size, with: transition)
    window.traitOverrides.horizontalSizeClass = regular ? .regular : .compact
    window.frame = CGRect(origin: .zero, size: size)
    window.layoutIfNeeded()
    transition.end()
  }

  /// Gives the window room for several columns, or for one, and lets them expand or collapse.
  func resize(wide: Bool) async throws {
    size(wide: wide)
    try await until { self.columns.isCollapsed != wide }
    try await settle()
  }

  func until(_ shown: () -> Bool, sourceLocation: SourceLocation = #_sourceLocation) async throws {
    for _ in 0..<1_000 where !shown() {
      try await Task.sleep(for: .milliseconds(10))
    }
    #expect(shown(), "main \(held), sidebar \(sidebar.viewControllers)", sourceLocation: sourceLocation)
  }

  /// Lets whatever is animated end.
  func settle() async throws {
    try await Task.sleep(for: .milliseconds(700))
    try await until { self.main.transitionCoordinator == nil && self.sidebar.transitionCoordinator == nil }
  }

  /// The field of the new-session screen the main view holds, once it is drawn.
  func field() async throws -> UIView {
    var found: UIView?
    try await until {
      found = self.main.viewControllers.first.flatMap { Self.field(in: $0.view) }
      return found != nil
    }
    return try #require(found)
  }

  private static func field(in view: UIView) -> UIView? {
    if view is UITextView || view is UITextField { return view }
    return view.subviews.lazy.compactMap { field(in: $0) }.first
  }

  func close() {
    window.isHidden = true
  }
}

/// A window's change of size, not animated, ended when the test says.
@MainActor private final class SizeTransition: NSObject, UIViewControllerTransitionCoordinator {
  private var ending: [(any UIViewControllerTransitionCoordinatorContext) -> Void] = []

  func end() {
    for ended in ending { ended(self) }
    ending = []
  }

  func animate(
    alongsideTransition animation: ((any UIViewControllerTransitionCoordinatorContext) -> Void)?,
    completion: ((any UIViewControllerTransitionCoordinatorContext) -> Void)?
  ) -> Bool {
    animation?(self)
    if let completion { ending.append(completion) }
    return true
  }

  func animateAlongsideTransition(
    in view: UIView?, animation: ((any UIViewControllerTransitionCoordinatorContext) -> Void)?,
    completion: ((any UIViewControllerTransitionCoordinatorContext) -> Void)?
  ) -> Bool {
    animate(alongsideTransition: animation, completion: completion)
  }

  func notifyWhenInteractionEnds(_ handler: @escaping (any UIViewControllerTransitionCoordinatorContext) -> Void) {}
  func notifyWhenInteractionChanges(_ handler: @escaping (any UIViewControllerTransitionCoordinatorContext) -> Void) {}

  var isAnimated: Bool { false }
  var presentationStyle: UIModalPresentationStyle { .none }
  var initiallyInteractive: Bool { false }
  var isInterruptible: Bool { false }
  var isInteractive: Bool { false }
  var isCancelled: Bool { false }
  var transitionDuration: TimeInterval { 0 }
  var percentComplete: CGFloat { 1 }
  var completionVelocity: CGFloat { 0 }
  var completionCurve: UIView.AnimationCurve { .linear }
  func viewController(forKey key: UITransitionContextViewControllerKey) -> UIViewController? { nil }
  func view(forKey key: UITransitionContextViewKey) -> UIView? { nil }
  var containerView: UIView { UIView() }
  var targetTransform: CGAffineTransform { .identity }
}

@Suite(.serialized) @MainActor struct ColumnsTests {
  private let session = SessionRoute(session: "workstation-key/7")
  private var open: Pushed { .session(session) }
  private var work: Pushed { .work(session, turn: "work") }
  private var detail: Pushed { .detail(session, activity: "check") }

  @Test func theSidebarIsPreferredBesideTheMainViewWhichIsGivenNoLeastWidth() async throws {
    let hosted = try await HostedColumns()
    defer { hosted.close() }
    let columns = hosted.columns
    let preferred = {
      #expect(columns.style == .doubleColumn)
      #expect(columns.preferredDisplayMode == .oneBesideSecondary)
      #expect(columns.preferredSplitBehavior == .tile)
      // With one the split view will not divide the Sidebar and the main view at a fold.
      #expect(columns.minimumSecondaryColumnWidth == UISplitViewController.automaticDimension)
    }
    preferred()
    // Nor is the Sidebar's column ever hidden, which would write the main view alone into what is preferred.
    for wide in [false, true, false] {
      try await hosted.resize(wide: wide)
      hosted.stack.path = [open, work]
      try await hosted.settle()
      preferred()
      hosted.stack.path = []
      try await hosted.settle()
      preferred()
    }
  }

  @Test func whatIsOpenIsPushedOverTheListingAndTheScreensHeldAreKept() async throws {
    let hosted = try await HostedColumns()
    defer { hosted.close() }
    #expect(hosted.listingAlone)
    hosted.stack.path = [open]
    try await hosted.until { hosted.sidebar.topViewController !== hosted.sidebar.viewControllers.first }
    let root = try #require(hosted.main.viewControllers.first)
    hosted.stack.path += [work, detail]
    try await hosted.settle()
    #expect(hosted.held == [open, work, detail])
    #expect(hosted.main.viewControllers.first === root)
    let pushed = hosted.main.viewControllers[1]
    hosted.stack.path = [open, work]
    try await hosted.settle()
    #expect(hosted.main.viewControllers == [root, pushed], "popped to, each the screen it was")
    // Another Session in its place is a screen of its own.
    hosted.stack.open("workstation-key/7")
    #expect(hosted.stack.path == [open])
    hosted.stack.open("workstation-key/1")
    try await hosted.settle()
    #expect(hosted.main.viewControllers.count == 1)
    #expect(hosted.main.viewControllers.first !== root)
  }

  @Test func whatTheReaderPopsIsReadBackAndOnlyReturningToTheListingClosesWhatIsOpen() async throws {
    let hosted = try await HostedColumns(open: [open, work, detail])
    defer { hosted.close() }
    try await hosted.until { hosted.held == [open, work, detail] }
    hosted.main.popViewController(animated: true)
    try await hosted.until { hosted.stack.path == [open, work] }
    hosted.main.popViewController(animated: true)
    try await hosted.until { hosted.stack.path == [open] }
    try await hosted.settle()
    #expect(hosted.stack.path == [open])
    hosted.sidebar.popViewController(animated: true)
    try await hosted.until { hosted.stack.path.isEmpty && hosted.listingAlone }
    try await hosted.until { hosted.held == [nil] }
    #expect((hosted.main.viewControllers.first as? HostedScreen)?.atRest == hosted.stack.rest)
    // And again, what closed the first leaving the second whole.
    hosted.stack.open("workstation-key/1")
    try await hosted.settle()
    #expect(hosted.stack.path.count == 1)
    #expect(!hosted.listingAlone)
  }

  @Test func screensPushedOneUponAnotherAllStand() async throws {
    let hosted = try await HostedColumns()
    defer { hosted.close() }
    // Each in the middle of the last one's push.
    hosted.stack.path = [open]
    hosted.stack.path.append(work)
    try await Task.sleep(for: .milliseconds(60))
    hosted.stack.path.append(detail)
    try await Task.sleep(for: .milliseconds(60))
    // Twice, as tapped twice it would be were it not held back.
    hosted.stack.path.append(detail)
    try await hosted.settle()
    #expect(hosted.stack.path == [open, work, detail, detail])
    #expect(hosted.held == [open, work, detail, detail])
    #expect(!hosted.listingAlone)
    // And popped in the middle of it.
    hosted.stack.path.removeLast()
    try await Task.sleep(for: .milliseconds(60))
    hosted.stack.path.removeLast()
    try await hosted.settle()
    #expect(hosted.stack.path == [open, work])
    #expect(hosted.held == [open, work])
  }

  @Test func aSessionGoneReturnsToTheListingWhereItStoodAlone() async throws {
    let hosted = try await HostedColumns(open: [open, work])
    defer { hosted.close() }
    #expect(!hosted.listingAlone)
    hosted.stack.leave(session)
    try await hosted.until { hosted.listingAlone }
    try await hosted.until { hosted.held == [nil] }
    #expect(hosted.stack.path.isEmpty)
  }

  @Test func aSessionGoneAsItOrAScreenOverItIsPushedReturnsToTheListing() async throws {
    let hosted = try await HostedColumns()
    defer { hosted.close() }
    for pushed in [[], [work], [work, detail]] {
      hosted.stack.path = [open]
      if !pushed.isEmpty {
        try await hosted.settle()
        hosted.stack.path += pushed
      }
      try await Task.sleep(for: .milliseconds(60))
      hosted.stack.leave(session)
      try await hosted.until { hosted.listingAlone && hosted.held == [nil] && !hosted.sidebar.isToolbarHidden }
      try await hosted.settle()
      #expect(hosted.stack.path.isEmpty)
      #expect(hosted.listingAlone)
      #expect(hosted.held == [nil])
      #expect(!hosted.sidebar.isToolbarHidden)
    }
  }

  @Test func aSessionGoneAsTheReaderPopsWithinItIsNotOpenedAgain() async throws {
    let hosted = try await HostedColumns(open: [open, work])
    defer { hosted.close() }
    try await hosted.until { hosted.held == [open, work] }
    hosted.main.popViewController(animated: true)
    try await Task.sleep(for: .milliseconds(60))
    #expect(hosted.stack.path == [open, work], "in the middle of the pop")
    hosted.stack.leave(session)
    #expect(hosted.stack.path.isEmpty)
    try await hosted.settle()
    #expect(hosted.stack.path.isEmpty)
    #expect(hosted.listingAlone)
    #expect(hosted.held == [nil])
  }

  @Test func anotherOpenedAsTheListingIsReturnedToStaysOpen() async throws {
    let hosted = try await HostedColumns(open: [open])
    defer { hosted.close() }
    for session in ["workstation-key/7", "workstation-key/1"] {
      hosted.sidebar.popViewController(animated: true)
      try await Task.sleep(for: .milliseconds(80))
      hosted.stack.open(session)
      try await hosted.settle()
      #expect(hosted.stack.path.count == 1)
      #expect(!hosted.listingAlone)
      #expect(hosted.held == hosted.stack.path.map(Optional.some))
    }
  }

  @Test func collapsingAndExpandingClosesNothingAndMakesNoScreenAfresh() async throws {
    let hosted = try await HostedColumns(open: [open, work])
    defer { hosted.close() }
    let screens = hosted.main.viewControllers
    #expect(screens.count == 2)
    for wide in [true, false, true, false] {
      try await hosted.resize(wide: wide)
      #expect(hosted.stack.path == [open, work])
      #expect(hosted.main.viewControllers == screens)
      #expect(hosted.listingAlone == false)
    }
    hosted.main.popViewController(animated: true)
    try await hosted.until { hosted.stack.path == [open] }
    try await hosted.resize(wide: true)
    #expect(hosted.columns.displayMode == .oneBesideSecondary, "the Sidebar beside the main view")
    hosted.stack.open("workstation-key/1")
    try await hosted.settle()
    #expect(hosted.held == hosted.stack.path.map(Optional.some))
    #expect(hosted.columns.displayMode == .oneBesideSecondary)
  }

  @Test func withNothingOpenTheMainViewRestsAndCollapsingLandsOnTheListing() async throws {
    let hosted = try await HostedColumns(wide: true)
    defer { hosted.close() }
    #expect(hosted.held == [nil])
    try await hosted.resize(wide: false)
    #expect(hosted.listingAlone)
    #expect(hosted.stack.path.isEmpty)
    try await hosted.resize(wide: true)
    #expect(hosted.held == [nil])
    hosted.stack.path = [open]
    try await hosted.settle()
    hosted.stack.path = []
    try await hosted.settle()
    #expect(hosted.held == [nil], "what is open closed beside the listing")
  }

  @Test func theScreenAtRestOpenedIsTheSameScreen() async throws {
    let hosted = try await HostedColumns(wide: true)
    defer { hosted.close() }
    let resting = try #require(hosted.main.viewControllers.first as? HostedScreen)
    let route = hosted.stack.rest
    hosted.stack.openRest(route)
    #expect(hosted.stack.path == [.landing(route)])
    #expect(hosted.main.viewControllers == [resting])
    #expect(resting.named == .landing(route))
    #expect(hosted.stack.rest != route, "another rests once nothing is open")
    try await hosted.resize(wide: false)
    #expect(!hosted.listingAlone, "opened, it is what one column shows")
    #expect(hosted.main.viewControllers == [resting])
  }

  @Test func theAsidesColumnHoldsItOnlyWhileItIsShownWithASessionOpenAndRoom() async throws {
    let hosted = try await HostedColumns(open: [open], wide: true)
    defer { hosted.close() }
    let columns = hosted.columns
    let aside = hosted.stack.aside
    let held = { columns.viewController(for: .inspector) }
    let stands = {
      try await hosted.until { held() != nil && columns.isShowing(.inspector) }
      try await hosted.settle()
      #expect(held() != nil && columns.isShowing(.inspector))
    }
    let gone = {
      try await hosted.until { held() == nil }
      try await hosted.settle()
      #expect(held() == nil && !columns.isShowing(.inspector))
    }
    #expect(columns.minimumInspectorColumnWidth == 290)
    #expect(aside.hasRoom)
    #expect(held() == nil, "hidden as it begins")

    aside.shown = true
    try await stands()
    let screen = held()
    #expect(columns.displayMode == .secondaryOnly, "the Sidebar behind its button")
    aside.shown = false
    try await gone()
    #expect(columns.displayMode == .oneBesideSecondary, "the Sidebar back beside the main view")

    // Asked for as it goes, and taken down as it comes.
    for shown in [true, false, true, false] {
      aside.shown = shown
      try await Task.sleep(for: .milliseconds(80))
    }
    try await gone()
    for shown in [true, false, true] {
      aside.shown = shown
      try await Task.sleep(for: .milliseconds(80))
    }
    try await stands()

    // From one Session to the next, and not while none is open.
    aside.shown = true
    try await stands()
    hosted.stack.open("workstation-key/1")
    try await stands()
    hosted.stack.compose()
    try await gone()
    #expect(columns.displayMode == .oneBesideSecondary)
    hosted.stack.open("workstation-key/7")
    try await stands()
    #expect(held() === screen)

    // Nor with one column alone.
    try await hosted.resize(wide: false)
    try await gone()
    #expect(hosted.columns.presentedViewController == nil, "and it stands nowhere else")
    #expect(aside.shown)
    try await hosted.resize(wide: true)
    try await stands()
    #expect(columns.preferredDisplayMode == .oneBesideSecondary)
  }

  @Test func theAsideHasRoomWhereColumnsStandIn690PointsOrMore() async throws {
    let hosted = try await HostedColumns(open: [open], wide: true)
    defer { hosted.close() }
    for (width, regular, room) in [
      (690, true, true), (689, true, false), (951, true, true), (669, true, false), (874, false, false),
      (956, true, true),
    ] as [(CGFloat, Bool, Bool)] {
      hosted.size(width: width, regular: regular)
      try await hosted.until { hosted.stack.aside.hasRoom == room }
      try await hosted.settle()
      #expect(hosted.stack.aside.hasRoom == room, "\(width)")
    }
    // Shown once, it stands wherever there is enough, the window's change of size alone bringing it back.
    hosted.stack.aside.shown = true
    for (width, stands) in [(669, false), (951, true), (669, false), (690, true)] as [(CGFloat, Bool)] {
      hosted.size(width: width, regular: true)
      try await hosted.until { (hosted.columns.viewController(for: .inspector) != nil) == stands }
      try await hosted.settle()
      #expect((hosted.columns.viewController(for: .inspector) != nil) == stands, "\(width)")
      #expect(hosted.columns.isShowing(.inspector) == stands, "\(width)")
    }
  }

  @Test func touchingTheFieldOfTheScreenAtRestOpensItWhereItStands() async throws {
    let hosted = try await HostedColumns(wide: true)
    defer { hosted.close() }
    let resting = try #require(hosted.main.viewControllers.first as? HostedScreen)
    let route = hosted.stack.rest
    // At rest no change of size puts the cursor in it.
    for wide in [false, true] {
      try await hosted.resize(wide: wide)
      #expect(hosted.main.viewControllers == [resting])
      #expect(hosted.stack.path.isEmpty)
    }
    let field = try await hosted.field()
    #expect(!field.isFirstResponder)
    #expect(field.becomeFirstResponder())
    try await hosted.until { hosted.stack.path == [.landing(route)] }
    #expect(resting.named == .landing(route) && resting.atRest == nil)
    for wide in [false, true] {
      try await hosted.resize(wide: wide)
      #expect(hosted.main.viewControllers == [resting], "no screen is made afresh")
      #expect(try await hosted.field() === field)
      #expect(field.isFirstResponder, "nor is the keyboard put down")
      #expect(!hosted.listingAlone)
    }
    // Its keyboard put down, it stays open, and down.
    field.resignFirstResponder()
    for wide in [false, true, false] {
      try await hosted.resize(wide: wide)
      #expect(hosted.stack.path == [.landing(route)])
      #expect(!field.isFirstResponder)
      #expect(!hosted.listingAlone)
    }
    // Returned from, nothing is open, and another rests.
    hosted.sidebar.popViewController(animated: true)
    try await hosted.until { hosted.stack.path.isEmpty && hosted.listingAlone && hosted.held == [nil] }
    #expect((hosted.main.viewControllers.first as? HostedScreen)?.atRest == hosted.stack.rest)
    #expect(hosted.main.viewControllers != [resting])
  }

  @Test func composingGivesTheScreenAfresh() async throws {
    let hosted = try await HostedColumns(wide: true)
    defer { hosted.close() }
    let resting = hosted.main.viewControllers
    hosted.stack.compose()
    try await hosted.settle()
    #expect(hosted.held == hosted.stack.path.map(Optional.some))
    let composed = hosted.main.viewControllers
    #expect(composed != resting)
    hosted.stack.compose()
    try await hosted.settle()
    #expect(hosted.main.viewControllers != composed, "and again over itself")
    try await hosted.resize(wide: false)
    #expect(!hosted.listingAlone, "it is open")
  }

  @Test func theListingsFootStandsWhileTheListingDoes() async throws {
    let hosted = try await HostedColumns()
    defer { hosted.close() }
    #expect(!hosted.sidebar.isToolbarHidden)
    hosted.stack.path = [open]
    try await hosted.settle()
    #expect(hosted.sidebar.isToolbarHidden)
    hosted.stack.path = []
    try await hosted.settle()
    #expect(!hosted.sidebar.isToolbarHidden)
    hosted.stack.path = [open]
    try await hosted.settle()
    try await hosted.resize(wide: true)
    #expect(!hosted.sidebar.isToolbarHidden, "beside the main view")
    try await hosted.resize(wide: false)
    #expect(hosted.stack.path == [open])
    #expect(hosted.sidebar.isToolbarHidden, "the session over the listing again")
  }
}
