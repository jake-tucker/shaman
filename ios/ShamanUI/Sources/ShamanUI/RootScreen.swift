import ShamanCore
import SwiftUI

/// What the app shows over its core: pairing alone while the phone holds no Remote, and otherwise the Remote the
/// Outlook stands at.
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
      case .remote(let name, let beginsIn):
        HomeScreen(core: core, appVersion: appVersion, remote: name, beginsIn: beginsIn, invite: invite)
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

/// Where the app lands on the Remote the Outlook stands at, until the listing stands here: the bottom bar, with the
/// sheet of Servers, Relays and This device behind its button.
struct HomeScreen: View {
  let core: any Core
  let appVersion: String
  let remote: String
  let beginsIn: String
  @State private var sheet: HomeSheet?

  init(core: any Core, appVersion: String, remote: String, beginsIn: String, invite: String?) {
    self.core = core
    self.appVersion = appVersion
    self.remote = remote
    self.beginsIn = beginsIn
    _sheet = State(initialValue: invite.map(HomeSheet.pairing))
  }

  var body: some View {
    NavigationStack {
      Color.clear
        .navigationTitle(remote)
        .toolbar {
          ToolbarItem(placement: .bottomBar) {
            Button {
              sheet = .servers
            } label: {
              Label("Servers & device", systemImage: "server.rack")
            }
            .accessibilityIdentifier("home.servers")
          }
          ToolbarItem(placement: .status) {
            Text(beginsIn)
              .font(.footnote)
              .foregroundStyle(.secondary)
              .accessibilityIdentifier("home.beginsIn")
          }
        }
    }
    .sheet(item: $sheet) { sheet in
      switch sheet {
      case .servers:
        DeviceScreen(core: core, appVersion: appVersion)
      case .pairing(let invite):
        NavigationStack {
          ConfirmationScreen(core: core, invite: invite) {
            self.sheet = .servers
          } paired: {
            self.sheet = nil
          }
        }
      }
    }
  }
}

/// A sheet over where the app lands.
private enum HomeSheet: Hashable, Identifiable {
  case servers
  /// Confirming an Invite that arrived by the launch environment.
  case pairing(String)

  var id: Self { self }
}

#if DEBUG
  #Preview("A paired phone") {
    RootScreen(core: FakeCore(.paired), appVersion: "0.1.0 (1)")
  }

  #Preview("A phone holding no server") {
    RootScreen(core: FakeCore(.fresh), appVersion: "0.1.0 (1)")
  }
#endif
