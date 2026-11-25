import ShamanCore
import SwiftUI

/// What is open: the Session or new-session screen open, and the screens pushed over it, as one value, `path`, whichever
/// columns stand, and the Aside beside it. `Columns` has the main view hold what the path says, and writes back to it
/// what the reader pops. Each Session is opened in the core as its screen is first drawn, and closed there once no
/// screen of it is on the path, however it leaves. What the scene keeps of it, and a path restored from what it kept,
/// cross as `KeptScene`'s screens.
@MainActor @Observable
final class SessionStack {
  /// The screens open, the one on top last; none while nothing is open.
  var path: [Pushed] {
    didSet {
      closeUnpushed()
      moved?()
    }
  }
  /// Told each time the path is written, whoever wrote it, and as a Session begins in a new-session screen, as the
  /// columns are.
  @ObservationIgnored var moved: (() -> Void)?
  /// What stands beside what is open, or waits to.
  let aside: Aside
  /// The new-session screen the main view holds at rest, while nothing is open. Not on the path, it is not open.
  @ObservationIgnored private(set) var rest = LandingRoute()
  @ObservationIgnored private let core: any Core
  @ObservationIgnored private var models: [SessionRoute: SessionModel] = [:]
  @ObservationIgnored private var landings: [LandingRoute: NewSessionModel] = [:]
  /// The Sessions' screens restored after a cold start, any of which found gone before it is read sends the reader
  /// back to the listing.
  @ObservationIgnored private var restored: Set<SessionRoute> = []

  init(core: any Core, path: [Pushed] = [], aside: Aside = Aside()) {
    self.core = core
    self.path = path
    self.aside = aside
  }

  /// The stack the scene kept, `kept`, pushed again after a cold start: each Session's screen, and the new-session
  /// screen, read afresh.
  convenience init(core: any Core, restoring kept: [KeptScene.Screen]) {
    let path = kept.map { screen -> Pushed in
      switch screen {
      case .session(let session): .session(SessionRoute(session: session))
      case .landing: .landing(LandingRoute())
      }
    }
    self.init(core: core, path: path)
    restored = Set(path.compactMap(\.route))
  }

  /// What the scene keeps of the path: each Session's screen, a new-session screen that has begun its Session kept as
  /// that Session's, and the new-session screen; never the screens of a Turn's work or an Activity.
  var kept: [KeptScene.Screen] {
    path.compactMap { pushed in
      switch pushed {
      case .session(let route): .session(route.session)
      case .landing(let route): landings[route]?.begun.map { .session($0.session) } ?? .landing
      case .work, .detail: nil
      }
    }
  }

  /// The model of the new-session screen `route` names, made the first time it is asked for. The Session begun from
  /// it is held as any pushed Session is, for as long as the screen is on the path.
  func landing(for route: LandingRoute) -> NewSessionModel {
    if let model = landings[route] {
      return model
    }
    let model = NewSessionModel(core: core)
    model.adopted = { [weak self] begun in
      guard let self else { return }
      models[begun.route] = begun
      // Begun from the screen at rest without its field being touched, that screen is open.
      openRest(route)
      moved?()
    }
    landings[route] = model
    return model
  }

  /// The model of the screen `route` pushed, opened the first time it is asked for.
  func model(for route: SessionRoute) -> SessionModel {
    if let model = models[route] {
      return model
    }
    let model = SessionModel(core: core, route: route)
    if restored.contains(route) {
      // A Session kept across a cold start found gone as it is read again, or whose Remote is, leaves nothing to
      // return to. Once read it is the Session the reader is in, and gone later its screen says so.
      model.whenGone = { [weak self] in self?.path = [] }
    }
    models[route] = model
    return model
  }

  /// The screen of the Session open on top, which the Aside answers for, whatever of it is pushed over it; none while
  /// no Session is open.
  var openSession: SessionRoute? {
    switch path.last {
    case .landing(let route): landings[route]?.begun?.route
    case let pushed: pushed?.route
    }
  }

  /// Opens the Session `session` names, chosen in the listing, in place of whatever is open, read afresh; but where it
  /// is what is open already, pops back to it.
  func open(_ session: String) {
    if let first = path.first, shows(first, session) {
      path = [first]
    } else {
      path = [.session(SessionRoute(session: session))]
    }
  }

  /// Opens the new-session screen afresh, in place of whatever is open.
  func compose() {
    path = [.landing(LandingRoute())]
  }

  /// Opens the new-session screen at rest as it stands, the same screen, where `route` is its, as touching its field
  /// does; another rests in its place once nothing is open again.
  func openRest(_ route: LandingRoute) {
    guard route == rest else { return }
    rest = LandingRoute()
    path = [.landing(route)]
  }

  /// Pushes `pushed` over what is open; nothing more where it is the screen on top already, as one tapped twice is.
  func push(_ pushed: Pushed) {
    guard path.last != pushed else { return }
    path.append(pushed)
  }

  /// Shows what a row of an open Session's conversation leads to: in the Aside, where it has room, and pushed over what
  /// is open where it has none.
  func lead(to pushed: Pushed) {
    switch pushed {
    case .work(let route, let turn) where aside.hasRoom:
      aside.holdWork(of: turn, latest: model(for: route).conversation.latestTurn, in: route)
      aside.shown = true
    case .detail(let route, let activity) where aside.hasRoom:
      aside.holdDetail(activity, latest: model(for: route).conversation.latestTurn, in: route)
      aside.shown = true
    default:
      push(pushed)
    }
  }

  /// Whether the Aside holds the tree of `session`, which is open: where that is the kind and the Session has one. Until
  /// the Session is framed whatever stands stays, as the tree does from one Session of it to the next.
  func asideShowsTree(of session: SessionModel) -> Bool {
    aside.kind == .tree && (session.frame == nil || session.frame?.tree != nil)
  }

  /// The Turn whose work the Aside is on screen at, as its card names it, where `session` is the Session it answers for;
  /// none while it shows the tree or is not on screen.
  func turnInAside(of session: SessionModel) -> String? {
    guard aside.onScreen, openSession == session.route, !asideShowsTree(of: session) else { return nil }
    return aside.place(of: session.route).held ?? session.conversation.latestTurn
  }

  /// Shows the tree in the Aside, where it has room. Answers whether it did: where it has none a sheet is to.
  func showTree() -> Bool {
    guard aside.hasRoom else { return false }
    aside.holdTree()
    aside.shown = true
    return true
  }

  /// Leaves the Session pushed as `route`, which turned out gone, with whatever is pushed over it.
  func leave(_ route: SessionRoute) {
    guard
      let pushed = path.firstIndex(where: {
        switch $0 {
        case .session(let pushed): pushed == route
        case .landing(let landing): landings[landing]?.begun?.route == route
        case .work, .detail: false
        }
      })
    else { return }
    path.removeSubrange(pushed...)
  }

  /// Shows the Session `session` names, chosen in the tree of the Session pushed as `route`: nothing more where it is
  /// that one, popping back to it where it is on the path already, as its own screen or as the new-session screen it
  /// was begun from, and pushing it over the path where it is not.
  func show(_ session: String, over route: SessionRoute) {
    guard route.session != session else { return }
    if let pushed = path.lastIndex(where: { shows($0, session) }) {
      path.removeSubrange(path.index(after: pushed)...)
    } else {
      path.append(.session(SessionRoute(session: session)))
    }
  }

  /// Whether `pushed` is the screen of the Session `session` names: its own, or the new-session screen it was begun
  /// from.
  private func shows(_ pushed: Pushed, _ session: String) -> Bool {
    switch pushed {
    case .session(let route): route.session == session
    case .landing(let landing): landings[landing]?.begun?.session == session
    case .work, .detail: false
    }
  }

  /// Follows the "Review" on the bar of the Session pushed as `route` where it leads: its own Interventions presented
  /// again, or the one Subagent's Session whose Intervention waits shown as an entry of its tree is. Answers whether
  /// the tree is to be opened, as it is where several Subagents' wait.
  func review(_ opens: ReviewOpens, over route: SessionRoute) -> Bool {
    switch opens {
    case .interventions:
      model(for: route).review()
      return false
    case .session(let session):
      show(session, over: route)
      return false
    case .tree:
      return true
    }
  }

  /// Closes every Session no screen of which is on the path, a Session begun from a new-session screen standing for
  /// as long as its screen does. The new-session screen at rest keeps its model.
  private func closeUnpushed() {
    let landed = Set(path.compactMap { if case .landing(let route) = $0 { route } else { nil } })
    let pushed = Set(path.compactMap(\.route) + landed.compactMap { landings[$0]?.begun?.route })
    for (route, model) in models where !pushed.contains(route) {
      model.close()
      models[route] = nil
    }
    for route in landings.keys where !landed.contains(route) && route != rest {
      landings[route] = nil
    }
    aside.keepPlaces(of: pushed)
  }
}

/// A Session's screen, open or pushed over what is: each its own, so the same Session pushed twice is two screens.
struct SessionRoute: Hashable {
  let session: String
  let push = UUID()
}

/// The new-session screen, open or at rest: each its own.
struct LandingRoute: Hashable {
  let push = UUID()
}

/// A screen the path names: a Session's, or a Turn's work or an Activity's whole pushed from one, which
/// read through the Session `route` names for as long as they are up; or the new-session screen.
enum Pushed: Hashable {
  case session(SessionRoute)
  case work(SessionRoute, turn: String)
  case detail(SessionRoute, activity: String)
  case landing(LandingRoute)

  /// The Session's screen it is, or was pushed from; none for the new-session screen, whose Session its model holds.
  var route: SessionRoute? {
    switch self {
    case .session(let route), .work(let route, _), .detail(let route, _): route
    case .landing: nil
    }
  }
}
