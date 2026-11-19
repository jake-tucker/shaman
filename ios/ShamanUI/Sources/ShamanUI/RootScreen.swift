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
  /// Whether the listing opens on the new-session screen, pushed over it at once.
  private let composing: Bool
  /// Whether what the scene kept is still to be handed to a listing: to the first one a launch shows, and never once
  /// the phone has held no Remote, nor where a debug build is asked to forget it.
  @State private var restoring: Bool
  @State private var outlook: Outlook?
  /// What the core says of its own accord, in the banner an act's failure is shown in.
  @State private var notices: NoticeModel
  @Environment(\.scenePhase) private var scenePhase
  /// What the scene keeps of the listing and what is pushed over it, which iOS restores after it ends the app: a
  /// `KeptScene`, as its plain string.
  @SceneStorage("listing") private var kept = ""

  /// The app over `core`, showing `appVersion` as its own version, already confirming `invite` where one is given, as
  /// one arriving by the launch environment in a debug build is, and returning to what the scene kept, but with the
  /// Session `opening` names pushed over the listing where one is named, or the new-session screen where it is
  /// `composing`, and as a new scene would open where it is `forgetting` what the scene kept. A notice the core gives
  /// stands `noticeStands` before it goes by itself.
  public init(
    core: any Core, appVersion: String, invite: String? = nil, opening: String? = nil, composing: Bool = false,
    forgetting: Bool = false, noticeStands: Duration = .seconds(8)
  ) {
    self.core = core
    _notices = State(initialValue: NoticeModel(core: core, stands: noticeStands))
    self.appVersion = appVersion
    self.opening = opening
    self.composing = composing
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
        ListingScreen(
          core: core, appVersion: appVersion, beginsIn: beginsIn, invite: invite,
          kept: restoring ? KeptScene(stored: kept) : KeptScene(), keep: { kept = $0.stored }, opening: opening,
          composing: composing)
      }
    }
    .overlay(alignment: .top) {
      if let notice = notices.notice {
        Banner(text: notice.words, quiet: true)
          .onTapGesture { notices.dismiss() }
          .accessibilityAddTraits(.isButton)
          .accessibilityHint("Dismisses this")
          .transition(.move(edge: .top).combined(with: .opacity))
      }
    }
    .animation(.default, value: notices.notice?.id)
    .task { await notices.follow() }
    .task {
      // Held while an act is under way, whatever screen asked it.
      await BackgroundTime(tasks: UIApplication.shared).follow(core.actsUnderWay())
    }
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

/// What the core says of its own accord, as the banner over every screen shows it: each notice until it is tapped away
/// or has stood a while, when the core is told it was seen, so it shows the next and never says that one again.
@MainActor @Observable
final class NoticeModel {
  private let core: any Core
  private(set) var notice: Notice?
  /// How long a notice stands before it goes by itself.
  private let stands: Duration
  /// What takes the notice away once it has stood its while.
  private var clearing: Task<Void, Never>?

  init(core: any Core, stands: Duration = .seconds(8)) {
    self.core = core
    self.stands = stands
  }

  /// Follows what the core says until the screen goes.
  func follow() async {
    for await notice in core.notices() {
      show(notice)
    }
  }

  /// Takes the notice away, telling the core it was seen.
  func dismiss() {
    clearing?.cancel()
    clearing = nil
    guard let notice else { return }
    self.notice = nil
    core.noticeSeen(notice.id)
  }

  private func show(_ next: Notice?) {
    guard next?.id != notice?.id else { return }
    clearing?.cancel()
    notice = next
    guard let next else { return }
    let stands = stands
    clearing = Task { [weak self] in
      try? await Task.sleep(for: stands)
      guard !Task.isCancelled, let self, self.notice?.id == next.id else { return }
      self.dismiss()
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
