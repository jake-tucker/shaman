import ShamanCore
import SwiftUI

/// What the app shows over its core: pairing alone while the phone holds no Remote, and otherwise the listing of
/// every Remote's Sessions.
public struct RootScreen: View {
  private let core: any Core
  private let appVersion: String
  /// The Invite the first screen shown confirms at once, and no screen after it.
  @State private var invite: String?
  @State private var outlook: Outlook?

  /// The app over `core`, showing `appVersion` as its own version, already confirming `invite` where one is given, as
  /// one arriving by the launch environment in a debug build is.
  public init(core: any Core, appVersion: String, invite: String? = nil) {
    self.core = core
    self.appVersion = appVersion
    _invite = State(initialValue: invite)
  }

  public var body: some View {
    Group {
      switch outlook {
      case nil:
        ProgressView()
      case .nowhere:
        PairingScreen(core: core, invite: invite)
      case .remote(_, let beginsIn):
        ListingScreen(core: core, appVersion: appVersion, beginsIn: beginsIn, invite: invite)
      }
    }
    .task {
      for await outlook in core.outlook() {
        if self.outlook != nil {
          invite = nil
        }
        self.outlook = outlook
      }
    }
  }
}

#if DEBUG
  #Preview("A paired phone") {
    RootScreen(core: FakeCore(.paired), appVersion: "0.1.0 (1)")
  }

  #Preview("A phone holding no server") {
    RootScreen(core: FakeCore(.fresh), appVersion: "0.1.0 (1)")
  }
#endif
