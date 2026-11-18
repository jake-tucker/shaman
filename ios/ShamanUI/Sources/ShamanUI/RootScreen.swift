import ShamanCore
import SwiftUI
import UIKit

/// What the app shows over its core: pairing alone while the phone holds no Remote, and otherwise the listing of
/// every Remote's Sessions.
public struct RootScreen: View {
  private let core: any Core
  private let appVersion: String
  /// The Invite the first screen shown confirms at once, and no screen after it.
  @State private var invite: String?
  /// The Session the listing opens on, pushed over it at once.
  private let opening: String?
  @State private var outlook: Outlook?
  @Environment(\.scenePhase) private var scenePhase

  /// The app over `core`, showing `appVersion` as its own version, already confirming `invite` where one is given, as
  /// one arriving by the launch environment in a debug build is, and with the Session `opening` names pushed over the
  /// listing where one is named.
  public init(core: any Core, appVersion: String, invite: String? = nil, opening: String? = nil) {
    self.core = core
    self.appVersion = appVersion
    self.opening = opening
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
        ListingScreen(core: core, appVersion: appVersion, beginsIn: beginsIn, invite: invite, opening: opening)
      }
    }
    .task {
      // Held while an act is under way, whatever screen asked it.
      await BackgroundTime(tasks: UIApplication.shared).follow(core.actsUnderWay())
    }
    .task {
      for await outlook in core.outlook() {
        if self.outlook != nil {
          invite = nil
        }
        self.outlook = outlook
      }
    }
    .task {
      for await hasPath in NetworkPath.changes() {
        core.pathChanged(hasPath: hasPath)
      }
    }
    // The core is told when the app comes to the front, so it checks every Remote, and when it leaves, when it does
    // nothing to its connections.
    .onChange(of: scenePhase) { _, phase in
      switch phase {
      case .active: core.cameToFront()
      case .background: core.leftFront()
      default: break
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
