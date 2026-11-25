import ShamanCore
import SwiftUI

/// What the app shows of a phone holding a Remote: the listing of every Remote's Sessions and what is open, in
/// `Columns`. It follows the listing however the columns stand, returns to what the scene kept after a cold start, and
/// hands the scene what to keep on each change.
struct ColumnsScreen: View {
  let core: any Core
  let appVersion: String
  let beginsIn: BeginsIn
  /// The Invite the listing confirms at once, where one is given.
  private let invite: String?
  /// The scope the scene kept, handed back to the core as the listing is first followed.
  private let restoring: String?
  /// Hands the scene what to keep of the listing and what is open.
  private let keep: (KeptScene) -> Void
  @State private var model: ListingModel
  /// What is open, each Session of it closed in the core as its screen is popped.
  @State private var stack: SessionStack
  // What a hosted screen, and a sheet it presents, reads of the environment, none of which reaches it by itself.
  @Environment(NoticeModel.self) private var notices: NoticeModel?
  @Environment(\.interventionArming) private var arming
  @Environment(\.codeScanner) private var scanner

  /// The columns over `core`, confirming `invite` where one is given, returned to what the scene kept, `kept`, which
  /// `keep` is handed again on each change; but with the Session `opening` names open, or the new-session screen where
  /// it is `composing`. With one `opening`, the Aside is shown at `aside`'s kind, where one is given.
  init(
    core: any Core, appVersion: String, beginsIn: BeginsIn, invite: String?, kept: KeptScene = KeptScene(),
    keep: @escaping (KeptScene) -> Void = { _ in }, opening: String? = nil, composing: Bool = false,
    aside: AsideKind? = nil
  ) {
    self.core = core
    self.appVersion = appVersion
    self.beginsIn = beginsIn
    self.invite = invite
    self.restoring = kept.scope
    self.keep = keep
    _model = State(initialValue: ListingModel(core: core))
    // Open at launch as a tap opens it, where a debug build asks; otherwise what the scene kept, restored, which an
    // Invite to confirm stands in place of, as the listing presents it.
    let open: [Pushed]? =
      composing
      ? [.landing(LandingRoute())]
      : opening.map { [.session(SessionRoute(session: $0))] } ?? (invite == nil ? nil : [])
    let aside = Aside(showing: opening == nil ? nil : aside)
    _stack = State(
      initialValue: open.map { SessionStack(core: core, path: $0, aside: aside) }
        ?? SessionStack(core: core, restoring: kept.pushed))
  }

  /// What the scene is to keep now: the scope the listing stands at, or the one it was handed back until it says, and
  /// what is open.
  private var kept: KeptScene {
    KeptScene(scope: model.listing?.scope ?? restoring, pushed: stack.kept)
  }

  var body: some View {
    ColumnsView(
      stack: stack,
      screens: Columns.Screens(
        listing: AnyView(
          ListingScreen(
            core: core, appVersion: appVersion, beginsIn: beginsIn, invite: invite, model: model, stack: stack)),
        open: { [stack] pushed in AnyView(PushedScreen(pushed: pushed, stack: stack)) },
        rest: { [stack] route in
          AnyView(
            NewSessionScreen(model: stack.landing(for: route), stack: stack, atRest: route)
              .noticeBanner())
        },
        aside: AnyView(AsideScreen(stack: stack)),
        handed: { [notices, arming, scanner] screen in
          AnyView(
            screen
              .environment(notices)
              .environment(\.interventionArming, arming)
              .environment(\.codeScanner, scanner))
        })
    )
    // Each column keeps its own screens clear of the display's edges and the keyboard.
    .ignoresSafeArea()
    .task { await model.follow(restoring: restoring) }
    .onChange(of: kept, initial: true) { _, kept in keep(kept) }
  }
}

/// The screen the path names `pushed`, whose Session is opened as it is first drawn.
private struct PushedScreen: View {
  let pushed: Pushed
  let stack: SessionStack

  var body: some View {
    switch pushed {
    case .session(let route):
      SessionScreen(model: stack.model(for: route), stack: stack)
    case .work(let route, let turn):
      TurnWorkScreen(session: stack.model(for: route), turn: turn) { stack.push($0) }
        .noticeBanner()
    case .detail(let route, let activity):
      ActivityDetailScreen(session: stack.model(for: route), activity: activity)
        .noticeBanner()
    case .landing(let route):
      NewSessionScreen(model: stack.landing(for: route), stack: stack)
        .noticeBanner()
    }
  }
}

#if DEBUG
  #Preview("One server") {
    ColumnsScreen(
      core: FakeCore(.paired), appVersion: "0.1.0 (1)",
      beginsIn: BeginsIn(
        lead: "New sessions begin in", place: "workstation", said: "New sessions begin in workstation"),
      invite: nil)
  }

  #Preview("Several servers") {
    ColumnsScreen(
      core: FakeCore(.several), appVersion: "0.1.0 (1)",
      beginsIn: BeginsIn(
        lead: "New sessions begin in", place: "workstation", said: "New sessions begin in workstation"),
      invite: nil)
  }
#endif
