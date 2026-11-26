import SwiftUI
import Testing
import UIKit

@testable import ShamanUI

/// The columns in a window of the app's own scene, as wide or as narrow as a test asks, so that they collapse, expand
/// and push as they do on a phone.
@MainActor final class HostedColumns {
  let window: UIWindow
  let stack: SessionStack
  let columns: Columns
  /// What the core says of its own accord, which the screens show, where the test asks.
  let notices: NoticeModel?
  private var following: Task<Void, Never>?

  /// The columns over a path of `open`, over the fake's `scenario`, in a window with room for one column alone, or for
  /// several where it is `wide`, or of `size` where one is given, at regular width. What is open is words alone, or,
  /// with `sessions`, the screens the app's columns hold. Where `noticed` they show what the core says of its own
  /// accord, as do the listing's words and the screen at rest. `launching` is handed what is open before anything is
  /// drawn.
  init(
    open: [Pushed] = [], wide: Bool = false, sessions: Bool = false, scenario: FakeCore.Scenario = .paired,
    size: CGSize? = nil, noticed: Bool = false, launching: (SessionStack) -> Void = { _ in }
  ) async throws {
    let scene = try #require(UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }.first)
    window = UIWindow(windowScene: scene)
    let core = FakeWindow(scenario)
    stack = SessionStack(core: core, path: open)
    let notices = noticed ? NoticeModel(core: core, stands: .seconds(600)) : nil
    self.notices = notices
    let stack = stack
    let screen = { (words: String) in AnyView(Text(words).navigationTitle(words)) }
    columns = Columns(
      stack: stack,
      screens: Columns.Screens(
        listing: noticed ? AnyView(Text("Listing").navigationTitle("Listing").noticeBanner()) : screen("Listing"),
        open: { pushed in
          sessions ? AnyView(PushedScreen(pushed: pushed, stack: stack)) : screen("\(pushed)")
        },
        rest: { route in
          let screen = NewSessionScreen(model: stack.landing(for: route), stack: stack, atRest: route)
          return noticed ? AnyView(screen.noticeBanner()) : AnyView(screen)
        },
        aside: AnyView(AsideScreen(stack: stack)), handed: { AnyView($0.environment(notices)) }))
    if let size {
      self.size(width: size.width, height: size.height, regular: true)
    } else {
      self.size(wide: wide)
    }
    launching(stack)
    if let notices {
      following = Task { await notices.follow() }
    }
    window.rootViewController = columns
    window.windowLevel = .alert + 1
    window.makeKeyAndVisible()
    try await until { self.columns.isCollapsed != (wide || size != nil) }
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

  /// Gives the window `width` points, of regular width or compact, and `height`. The columns are told of it as a turn
  /// tells them, as setting a window's frame does not.
  func size(width: CGFloat, height: CGFloat = 669, regular: Bool) {
    sizing(width: width, height: height, regular: regular)()
  }

  /// Gives the window `width` points as `size` does, but leaves the transition going until what it answers is called.
  func sizing(width: CGFloat, height: CGFloat = 669, regular: Bool) -> () -> Void {
    let size = CGSize(width: width, height: height)
    let transition = SizeTransition()
    columns.viewWillTransition(to: size, with: transition)
    window.traitOverrides.horizontalSizeClass = regular ? .regular : .compact
    window.frame = CGRect(origin: .zero, size: size)
    window.layoutIfNeeded()
    return transition.end
  }

  /// Gives the window room for several columns, or for one, and lets them expand or collapse.
  func resize(wide: Bool) async throws {
    size(wide: wide)
    try await until { self.columns.isCollapsed != wide }
    try await settle()
  }

  func until(
    _ shown: () -> Bool, _ said: @autoclosure () -> String = "", sourceLocation: SourceLocation = #_sourceLocation
  ) async throws {
    for _ in 0..<1_000 where !shown() {
      try await Task.sleep(for: .milliseconds(10))
    }
    #expect(shown(), "main \(held), sidebar \(sidebar.viewControllers) \(said())", sourceLocation: sourceLocation)
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
    following?.cancel()
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
