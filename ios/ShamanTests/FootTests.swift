import SwiftUI
import Testing
import UIKit

@testable import ShamanUI

/// A screen with a composer's foot in a window of the app's own scene, as tall as the display, over the software
/// keyboard, which the simulator holding a hardware one would else minimise.
@MainActor private final class HostedFoot: NSObject {
  let window: UIWindow
  private let host: UIHostingController<AnyView>
  private let arguments = UserDefaults.standard.volatileDomain(forName: UserDefaults.argumentDomain)
  private var link: CADisplayLink?
  private var watched: UIView?
  /// How far above the display's bottom edge the field's box was drawn at each frame.
  private var stood: [CGFloat] = []
  /// How far above that edge the keyboard last said it will stand.
  private(set) var keyboard: CGFloat = 0

  init(_ screen: some View) throws {
    // As a launch argument has it, and for this process alone.
    UserDefaults.standard.setVolatileDomain(
      arguments.merging(["AutomaticMinimizationEnabled": false]) { _, new in new },
      forName: UserDefaults.argumentDomain)
    let scene = try #require(UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }.first)
    window = UIWindow(windowScene: scene)
    host = UIHostingController(rootView: AnyView(screen))
    super.init()
    NotificationCenter.default.addObserver(
      self, selector: #selector(rising), name: UIResponder.keyboardWillShowNotification, object: nil)
    window.rootViewController = UINavigationController(rootViewController: host)
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

@Suite(.serialized) @MainActor struct FootHostedTests {
  private func session(_ session: String = "workstation-key/7") -> AnyView {
    AnyView(SessionScreen(model: SessionModel(core: FakeCore(.paired), session: session)))
  }

  /// The new-session screen at rest, which does not arrive writing.
  private func landing() -> AnyView {
    let core = FakeCore(.composing)
    let stack = SessionStack(core: core)
    return AnyView(NewSessionScreen(model: NewSessionModel(core: core), stack: stack, atRest: stack.rest))
  }

  /// It rests 28 points above the edge at compact width and 22 at regular, and is one view at both.
  @Test(arguments: [false, true]) func theFieldStandsAsTheWidthClassSays(session: Bool) async throws {
    let hosted = try HostedFoot(session ? self.session() : landing())
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
    let hosted = try HostedFoot(session ? self.session() : landing())
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

  /// A Subagent's Session that has ended has no foot, and its conversation's last row rests clear of the display's edge.
  @Test func aConversationWithNoFootEndsClearOfTheEdge() async throws {
    let hosted = try HostedFoot(session("workstation-key/12.2"))
    defer { hosted.close() }
    let list = try await hosted.found(FootCollectionView.self)
    try await Task.sleep(for: .seconds(1))
    let end = list.convert(list.bounds, to: nil).maxY - list.adjustedContentInset.bottom
    #expect(hosted.window.safeAreaInsets.bottom > 0)
    #expect(abs(hosted.window.bounds.maxY - end - hosted.window.safeAreaInsets.bottom) < 0.5)
  }
}
