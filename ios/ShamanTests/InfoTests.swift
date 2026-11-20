import Foundation
import Testing

/// What the app's Info settings say of it, as the built app carries them: hosted in the app, its bundle is the app's.
@Suite struct InfoTests {
  private let info = Bundle.main.infoDictionary ?? [:]

  /// The QR code holds the bare Invite, so the app opens no link of its own.
  @Test func theAppRegistersNoURLScheme() {
    #expect(Bundle.main.bundleIdentifier == "com.jaketucker.shaman")
    #expect(info["CFBundleURLTypes"] == nil)
  }

  /// The scanner asks for the camera, which iOS refuses an app saying nothing of why it would use it.
  @Test func theAppSaysWhyItWouldUseTheCamera() {
    let why = info["NSCameraUsageDescription"] as? String
    #expect(why?.isEmpty == false)
  }
}
