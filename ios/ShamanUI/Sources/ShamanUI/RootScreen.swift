import ShamanCore
import SwiftUI

/// What a window of the app shows over its core: pairing alone while the phone holds no Remote, and otherwise the
/// listing of every Remote's Sessions, in the columns. It is the window's root, which asks the device's core for the
/// window's own half once, as it is first drawn, and holds it while the window lives.
public struct RootScreen: View {
  /// This window's half of the core, a Client of its own, and what it says of its own accord.
  @State private var held: HeldWindow
  private var core: any WindowCore { held.core }
  private let appVersion: String
  /// The Invite the first screen shown confirms at once, and no screen after it.
  @State private var invite: String?
  /// The Session the listing opens on, pushed over it at once.
  private let opening: String?
  /// Whether the listing opens on the new-session screen, pushed over it at once.
  private let composing: Bool
  /// The kind of thing the Aside is shown at beside that Session, where it is to be shown at launch.
  private let aside: AsideKind?
  /// Whether what the scene kept is still to be handed to a listing: to the first one a launch shows, and never once
  /// the phone has held no Remote, nor where a debug build is asked to forget it.
  @State private var restoring: Bool
  @State private var outlook: Outlook?
  /// What the scene keeps of the listing and what is pushed over it, which iOS restores after it ends the app: a
  /// `KeptScene`, as its plain string.
  @SceneStorage("listing") private var kept = ""

  /// A window of the app over the device's `core`, showing `appVersion` as its own version, already confirming
  /// `invite` where one is given, as one arriving by the launch environment in a debug build is, and returning to what
  /// the scene kept, but with the Session `opening` names pushed over the listing where one is named, with the Aside
  /// shown beside it at `aside`'s kind where one is given, or the new-session screen where it is `composing`, and as a
  /// new scene would open where it is `forgetting` what the scene kept. A notice the core gives stands `noticeStands`
  /// before it goes by itself.
  public init(
    core: any DeviceCore, appVersion: String, invite: String? = nil, opening: String? = nil, composing: Bool = false,
    aside: AsideKind? = nil, forgetting: Bool = false, noticeStands: Duration = .seconds(8)
  ) {
    _held = State(initialValue: HeldWindow(over: core, noticeStands: noticeStands))
    self.appVersion = appVersion
    self.opening = opening
    self.composing = composing
    self.aside = aside
    _restoring = State(initialValue: !forgetting)
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
        ColumnsScreen(
          core: core, appVersion: appVersion, beginsIn: beginsIn, invite: invite,
          kept: restoring ? KeptScene(stored: kept) : KeptScene(), keep: { kept = $0.stored }, opening: opening,
          composing: composing, aside: aside)
      }
    }
    // Each screen shows it beneath its own bar, by `noticeBanner`.
    .environment(held.notices)
    .task { await held.notices.follow() }
    .task {
      for await outlook in core.outlook() {
        if self.outlook != nil {
          invite = nil
        }
        switch (self.outlook, outlook) {
        case (_, .nowhere):
          // A phone holding no Remote has nothing to return to, now or once it pairs again.
          kept = ""
          restoring = false
        case (.remote, .remote):
          // Handed already to the listing shown, once this launch.
          restoring = false
        default: break
        }
        self.outlook = outlook
      }
    }
  }
}

/// A window's half of the core, asked of the device's the first time the window holding it is drawn, and what the core
/// says of its own accord in that window, in the banner an act's failure is shown in, beneath each screen's bar.
/// SwiftUI may make a view's state more than once and keep the first, so nothing is asked of the core as it is made,
/// and no window is made that never stands.
@MainActor final class HeldWindow {
  private let device: any DeviceCore
  private let noticeStands: Duration

  init(over device: any DeviceCore, noticeStands: Duration = .seconds(8)) {
    self.device = device
    self.noticeStands = noticeStands
  }

  private(set) lazy var core: any WindowCore = device.window()
  private(set) lazy var notices = NoticeModel(core: core, stands: noticeStands)
}

#if DEBUG
  #Preview("A paired phone") {
    RootScreen(core: FakeCore(.paired), appVersion: "0.1.0 (1)")
  }

  #Preview("A phone holding no server") {
    RootScreen(core: FakeCore(.fresh), appVersion: "0.1.0 (1)")
  }
#endif
