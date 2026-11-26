import SwiftUI
import Testing
import UIKit

@testable import ShamanUI

/// A screen with a composer's foot in a window of the app's own scene, as tall as the display, over the software
/// keyboard, which the simulator holding a hardware one would else minimise. How the window stands is read above it, as
/// it is above the columns: the screen is hosted in a navigation controller of its own within the one read.
@MainActor private final class HostedFoot: NSObject {
  let window: UIWindow
  /// How the window stands, as it is read above the screen.
  let pose: Pose
  private let host: UIHostingController<AnyView>
  private let arguments = UserDefaults.standard.volatileDomain(forName: UserDefaults.argumentDomain)
  private var watched: UIView?
  /// How far above the display's bottom edge the field's box was drawn at each frame.
  private var stood: [CGFloat] = []
  /// How far above that edge the keyboard last said it will stand.
  private(set) var keyboard: CGFloat = 0

  init(_ screen: some View, reading pose: Pose) throws {
    // As a launch argument has it, and for this process alone.
    UserDefaults.standard.setVolatileDomain(
      arguments.merging(["AutomaticMinimizationEnabled": false]) { _, new in new },
      forName: UserDefaults.argumentDomain)
    let scene = try #require(UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }.first)
    window = UIWindow(windowScene: scene)
    self.pose = pose
    host = UIHostingController(rootView: AnyView(screen))
    super.init()
    NotificationCenter.default.addObserver(
      self, selector: #selector(rising), name: UIResponder.keyboardWillShowNotification, object: nil)
    window.rootViewController = UIHostingController(rootView: Hosting(host: host).ignoresSafeArea().reads(pose))
    window.windowLevel = .alert + 1
    window.makeKeyAndVisible()
  }

  @objc private func rising(_ note: Notification) {
    guard let end = note.userInfo?[UIResponder.keyboardFrameEndUserInfoKey] as? CGRect else { return }
    keyboard = window.bounds.maxY - end.minY
  }

  /// The first view of `Kind` in the screen, once there is one.
  func found<Kind: UIView>(_ kind: Kind.Type) async throws -> Kind {
    for _ in 0..<1_000 {
      if let found = Self.first(kind, in: host.view) { return found }
      try await Task.sleep(for: .milliseconds(10))
    }
    return try #require(Self.first(kind, in: host.view))
  }

  private static func first<Kind: UIView>(_ kind: Kind.Type, in view: UIView) -> Kind? {
    view as? Kind ?? view.subviews.lazy.compactMap { first(kind, in: $0) }.first
  }

  /// Every view of `Kind` in the screen.
  func all<Kind: UIView>(_ kind: Kind.Type, in view: UIView? = nil) -> [Kind] {
    let view = view ?? host.view!
    return (view as? Kind).map { [$0] } ?? view.subviews.flatMap { all(kind, in: $0) }
  }

  /// The composer's field, its screen having come to rest.
  func field() async throws -> UIView {
    let field = try await found(UITextView.self)
    try await Task.sleep(for: .seconds(1))
    return field
  }

  /// How far above the display's bottom edge the box of `field`, its words in the middle of 40 points, is drawn now.
  func stands(_ field: UIView) -> CGFloat {
    let drawn = field.layer.presentation() ?? field.layer
    return window.bounds.maxY - drawn.convert(drawn.bounds, to: window.layer.presentation()).midY - 20
  }

  /// Where the field's box is drawn at each frame while `change` is made and what it animates ends.
  func frames(of field: UIView, as change: () -> Void) async throws -> [CGFloat] {
    stood = []
    watched = field
    let link = CADisplayLink(target: self, selector: #selector(tick))
    link.add(to: .main, forMode: .common)
    change()
    try await Task.sleep(for: .seconds(1.5))
    link.invalidate()
    return stood
  }

  @objc private func tick() {
    if let watched { stood.append(stands(watched)) }
  }

  func close() {
    window.endEditing(true)
    window.isHidden = true
    NotificationCenter.default.removeObserver(self)
    UserDefaults.standard.setVolatileDomain(arguments, forName: UserDefaults.argumentDomain)
  }
}

/// A screen's own controller in a navigation controller, as a column holds it.
private struct Hosting: UIViewControllerRepresentable {
  let host: UIViewController

  func makeUIViewController(context: Context) -> UINavigationController {
    UINavigationController(rootViewController: host)
  }

  func updateUIViewController(_ controller: UINavigationController, context: Context) {}
}

@Suite(.serialized) @MainActor struct FootHostedTests {
  private func session(_ session: String = "workstation-key/7") throws -> HostedFoot {
    let route = SessionRoute(session: session)
    let stack = SessionStack(core: FakeWindow(.paired), path: [.session(route)])
    return try HostedFoot(SessionScreen(model: stack.model(for: route), stack: stack), reading: stack.pose)
  }

  /// The new-session screen at rest, which does not arrive writing.
  private func landing() throws -> HostedFoot {
    let core = FakeWindow(.composing)
    let stack = SessionStack(core: core)
    return try HostedFoot(
      NewSessionScreen(model: NewSessionModel(core: core), stack: stack, atRest: stack.rest), reading: stack.pose)
  }

  /// It rests 28 points above the edge at compact width and 22 at regular, and is one view at both.
  @Test(arguments: [false, true]) func theFieldStandsAsTheWidthClassSays(session: Bool) async throws {
    let hosted = try session ? self.session() : landing()
    defer { hosted.close() }
    hosted.window.traitOverrides.horizontalSizeClass = .compact
    let field = try await hosted.field()
    #expect(abs(hosted.stands(field) - 28) < 0.5, "\(hosted.stands(field))")
    hosted.window.traitOverrides.horizontalSizeClass = .regular
    #expect(try await hosted.field() === field)
    #expect(abs(hosted.stands(field) - 22) < 0.5, "\(hosted.stands(field))")
  }

  /// Up with the keyboard to 8 points above it and down again, the field at each frame further on and never past where
  /// it ends: a foot that gave way before or after the keyboard moved would stand lower or higher than that for a while.
  @Test(arguments: [(true, false), (true, true), (false, false), (false, true)])
  func theFieldGoesWithTheKeyboardAndNeverJumps(session: Bool, regular: Bool) async throws {
    let hosted = try session ? self.session() : landing()
    defer { hosted.close() }
    hosted.window.traitOverrides.horizontalSizeClass = regular ? .regular : .compact
    let rests: CGFloat = regular ? 22 : 28
    let field = try await hosted.field()
    for _ in 0..<2 {
      #expect(abs(hosted.stands(field) - rests) < 0.5)
      let up = try await hosted.frames(of: field) { #expect(field.becomeFirstResponder()) }
      #expect(hosted.keyboard > 100, "the software keyboard is up")
      let raised = hosted.keyboard + 8
      #expect(abs((up.last ?? 0) - raised) < 0.5, "\(up)")
      #expect(up.allSatisfy { $0 > rests - 0.5 && $0 < raised + 0.5 }, "\(up)")
      #expect(zip(up, up.dropFirst()).allSatisfy { $1 > $0 - 0.5 }, "\(up)")
      #expect(up.contains { $0 > rests + 1 && $0 < raised - 1 }, "on the way: \(up)")
      #expect(try await hosted.field() === field)
      #expect(field.isFirstResponder, "it keeps its focus")

      let down = try await hosted.frames(of: field) { field.resignFirstResponder() }
      #expect(abs((down.last ?? 0) - rests) < 0.5, "\(down)")
      #expect(down.allSatisfy { $0 > rests - 0.5 && $0 < raised + 0.5 }, "\(down)")
      #expect(zip(down, down.dropFirst()).allSatisfy { $1 < $0 + 0.5 }, "\(down)")
      #expect(down.contains { $0 > rests + 1 && $0 < raised - 1 }, "on the way: \(down)")
    }
  }

  /// Sat like a laptop, the fold where the test says, it goes the same way over the software keyboard, from under the
  /// Aside, as that gives up the half below the fold to it, and back, one view and never without its focus.
  /// The new-session screen's goes the same way, from the foot of the half below, where it stands with nothing above it.
  @Test(arguments: [true, false]) func satLikeALaptopTheFieldGoesWithTheKeyboardFromTheFootOfTheHalfBelow(
    session: Bool
  ) async throws {
    let hosted = try session ? self.session() : landing()
    defer { hosted.close() }
    hosted.window.traitOverrides.horizontalSizeClass = .regular
    let field = try await hosted.field()
    // The conversation, or the new-session screen's middle.
    let list: UIScrollView =
      session
      ? try await hosted.found(FootCollectionView.self)
      : try #require(hosted.all(UIScrollView.self).first { !($0 is UITextView) })
    let height = hosted.window.bounds.height
    let fold = Fold(top: (height / 2).rounded() - 20, bottom: (height / 2).rounded() + 20, window: height)
    let ends = { list.convert(list.bounds, to: nil).maxY - list.adjustedContentInset.bottom }
    hosted.pose.fold = fold
    #expect(try await hosted.field() === field)
    for _ in 0..<2 {
      #expect(abs(hosted.stands(field) - 22) < 0.5)
      #expect(abs(ends() - fold.top) < 0.5, "\(ends())")
      let up = try await hosted.frames(of: field) { #expect(field.becomeFirstResponder()) }
      let raised = hosted.keyboard + 8
      #expect(hosted.keyboard > 100 && hosted.pose.keyboardIsUp && abs(hosted.pose.keyboard - hosted.keyboard) < 0.5)
      #expect(abs((up.last ?? 0) - raised) < 0.5, "\(up)")
      #expect(up.allSatisfy { $0 > 21.5 && $0 < raised + 0.5 }, "\(up)")
      #expect(zip(up, up.dropFirst()).allSatisfy { $1 > $0 - 0.5 }, "\(up)")
      #expect(ends() < height - raised - 40)
      #expect(try await hosted.field() === field)
      #expect(field.isFirstResponder, "it keeps its focus")

      let down = try await hosted.frames(of: field) { field.resignFirstResponder() }
      #expect(abs((down.last ?? 0) - 22) < 0.5, "\(down)")
      #expect(down.allSatisfy { $0 > 21.5 && $0 < raised + 0.5 }, "\(down)")
      #expect(zip(down, down.dropFirst()).allSatisfy { $1 < $0 + 0.5 }, "\(down)")
      #expect(!hosted.pose.keyboardIsUp)
      #expect(try await hosted.field() === field)
    }
  }

  /// A Subagent's Session that has ended has no foot, and its conversation's last row rests clear of the display's edge.
  @Test func aConversationWithNoFootEndsClearOfTheEdge() async throws {
    let hosted = try session("workstation-key/12.2")
    defer { hosted.close() }
    let list = try await hosted.found(FootCollectionView.self)
    try await Task.sleep(for: .seconds(1))
    let end = list.convert(list.bounds, to: nil).maxY - list.adjustedContentInset.bottom
    #expect(hosted.window.safeAreaInsets.bottom > 0)
    #expect(abs(hosted.window.bounds.maxY - end - hosted.window.safeAreaInsets.bottom) < 0.5)
  }
}
