import AuthenticationServices
import SwiftUI
import Testing
import UIKit

@testable import ShamanUI

/// Where the system's authentication sheet a login at a Relay opens stands: over the window whose sheet asked for the
/// login, where the app shows several, not over whichever is key.
@Suite(.oneWindowAtATime) @MainActor struct LoginAnchorTests {
  @Test func theSystemsSheetStandsOverTheWindowThatAskedAndNotTheKeyOne() throws {
    let scene = try #require(UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }.first)
    let key = try #require(scene.keyWindow, "the app's own window is key")
    let asked = UIWindow(windowScene: scene)
    defer { asked.isHidden = true }
    let browser = WebLoginBrowser()
    let session = ASWebAuthenticationSession(
      url: try #require(URL(string: "https://github.com/login/device")), callback: .customScheme("shaman-login")
    ) { _, _ in }
    #expect(browser.presentationAnchor(for: session) === key, "with no window to stand over, the key one")

    browser.anchor(to: asked)

    #expect(browser.presentationAnchor(for: session) === asked)
  }

  @Test func theLoginsSheetTellsItsBrowserTheWindowItStandsIn() async throws {
    let scene = try #require(UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }.first)
    let browser = AnchoredBrowser()
    let model = RelayLoginModel(
      core: FakeWindow(.relayLogin, loginConfirmsAfter: .seconds(60)), relay: FakeCore.relay, browser: browser)
    let window = UIWindow(windowScene: scene)
    defer {
      window.isHidden = true
      window.rootViewController = nil
    }
    window.rootViewController = UIHostingController(rootView: RelayLoginSheet(model: model) {})
    window.isHidden = false

    for _ in 0..<200 where browser.anchored == nil {
      try await Task.sleep(for: .milliseconds(10))
    }
    #expect(browser.anchored === window)
  }
}

/// A browser that records the window it was told to stand over, and opens nothing.
@MainActor
private final class AnchoredBrowser: LoginBrowser {
  private(set) weak var anchored: UIWindow?

  func open(_ url: URL) {}

  func close() {}

  func anchor(to window: UIWindow) {
    anchored = window
  }
}
