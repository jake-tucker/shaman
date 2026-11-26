import AuthenticationServices
import Testing
import UIKit

@testable import ShamanUI

/// Where the system's authentication sheet a login at a Relay opens stands: over the window whose sheet asked for the
/// login, where the app shows several, not over whichever is key.
@Suite @MainActor struct LoginAnchorTests {
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
}
