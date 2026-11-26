import ShamanCore
import SwiftUI

/// What a window of the app shows over its core: pairing alone while the phone holds no Remote, and otherwise the
/// listing of every Remote's Sessions, in the columns. It is the window's root, which asks the device's core for the
/// window's own half once, as it is first drawn, and holds it while the window lives.
public struct RootScreen: View {
  /// This window's half of the core, a Client of its own, what it says of its own accord, and what it was asked at
  /// launch to show.
  @State private var held: HeldWindow
  /// This window's half of the core, asked for as the window is first drawn, handed what its scene kept of its Outlook.
  private var core: any WindowCore { held.core(handingBack: handingBack) }
  /// What the scene kept of the window's Outlook, to hand back as the window is made: nothing where the scene is not
  /// restored.
  private var handingBack: String? { restoring ? KeptScene(stored: kept).outlook : nil }
  private let appVersion: String
  /// The Invite the first screen shown confirms at once, and no screen after it.
  @State private var invite: String?
  /// Whether what the scene kept is still to be handed to a listing: to the first one a launch shows, and never once
  /// the phone has held no Remote, nor where a debug build is asked to forget it.
  @State private var restoring: Bool
  @State private var outlook: Outlook?
  /// What the scene keeps of where the Outlook stands, the listing and what is pushed over it, which iOS restores after
  /// it ends the app: a `KeptScene`, as its plain string.
  @SceneStorage("listing") private var kept = ""

  /// A window of the app over the device's `core`, showing `appVersion` as its own version, and returning to what the
  /// scene kept, its Outlook included, but as a new scene would open where it is `forgetting` what the scene kept. The
  /// first window whose root's first task runs shows what `launch` asks, as a debug build is asked by its launch
  /// arguments, and every other window nothing of it. A notice the core gives stands `noticeStands` before it goes by itself.
  public init(
    core: any DeviceCore, appVersion: String, launch: LaunchArguments = LaunchArguments(), forgetting: Bool = false,
    noticeStands: Duration = .seconds(8)
  ) {
    _held = State(initialValue: HeldWindow(over: core, launch: launch, noticeStands: noticeStands))
    self.appVersion = appVersion
    _restoring = State(initialValue: !forgetting)
  }

  public var body: some View {
    Group {
      switch outlook {
      case nil:
        ProgressView()
      case .nowhere:
        PairingScreen(core: core, invite: invite)
      case .remote(_, let beginsIn):
        let asked = held.launched
        ColumnsScreen(
          core: core, appVersion: appVersion, beginsIn: beginsIn, invite: invite,
          kept: restoring ? KeptScene(stored: kept) : KeptScene(), keep: { keep($0) }, opening: asked.opening,
          composing: asked.composing, aside: asked.aside)
      }
    }
    // One screen of the window shows it beneath its bar, by `noticeBanner`, as the columns say which.
    .environment(held.notices(handingBack: handingBack))
    .task { await held.notices(handingBack: handingBack).follow() }
    .task {
      invite = held.launched.invite
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
        if case .remote = outlook {
          keep(KeptScene(stored: kept))
        }
      }
    }
  }

  /// Keeps `scene`, what the listing hands the scene to keep, with where the Outlook stands now.
  private func keep(_ scene: KeptScene) {
    var scene = scene
    scene.outlook = core.keptOutlook()
    kept = scene.stored
  }
}

/// A window's half of the core, asked of the device's the first time the window holding it is drawn, handed what its
/// scene kept of its Outlook so nothing falls between making it and handing that back, what the core
/// says of its own accord in that window, in the banner an act's failure is shown in, beneath the bar of the one screen
/// that says it, and what the window was asked at launch to show. SwiftUI may make a view's state more than once and
/// keep the first, so nothing is asked of the core, nor launch arguments claimed, as it is made: the window's half of
/// the core is asked for as its root's body is first drawn, and the launch arguments are claimed as its root's first
/// task runs, so no window is made, nor arguments claimed, for a root that never stands.
@MainActor final class HeldWindow {
  private let device: any DeviceCore
  private let launch: LaunchArguments
  private let noticeStands: Duration

  init(over device: any DeviceCore, launch: LaunchArguments = LaunchArguments(), noticeStands: Duration = .seconds(8)) {
    self.device = device
    self.launch = launch
    self.noticeStands = noticeStands
  }

  private var window: (any WindowCore)?
  private var noticeModel: NoticeModel?

  /// The window's half of the core, asked of the device's the first time, handed `kept`, what its scene kept of its
  /// Outlook; what is handed after the first is not asked for.
  func core(handingBack kept: String?) -> any WindowCore {
    if let window { return window }
    let made = device.window(kept: kept)
    window = made
    return made
  }

  /// What the core says of its own accord in the window, over its half of the core, made as `core(handingBack:)` is.
  func notices(handingBack kept: String?) -> NoticeModel {
    if let noticeModel { return noticeModel }
    let made = NoticeModel(core: core(handingBack: kept), stands: noticeStands)
    noticeModel = made
    return made
  }
  /// What this window was asked at launch to show, claimed as its root's first task runs: everything where it is the
  /// first window whose root's task runs, and nothing otherwise.
  private(set) lazy var launched = launch.claimed()
}

#if DEBUG
  #Preview("A paired phone") {
    RootScreen(core: FakeCore(.paired), appVersion: "0.1.0 (1)")
  }

  #Preview("A phone holding no server") {
    RootScreen(core: FakeCore(.fresh), appVersion: "0.1.0 (1)")
  }
#endif
