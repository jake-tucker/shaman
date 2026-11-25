import ShamanCore
import SwiftUI

/// The Sessions pushed over the listing, and what is pushed over each, as its navigation stack holds them: each
/// Session opened in the core as its screen is first drawn, and closed there once no screen of it is on the path,
/// however it leaves. What the scene keeps of it, and a path restored from what it kept, cross as `KeptScene`'s
/// screens.
@MainActor @Observable
final class SessionStack {
  /// The screens pushed, the one on top last.
  var path: [Pushed] {
    didSet { closeUnpushed() }
  }
  @ObservationIgnored private let core: any Core
  @ObservationIgnored private var models: [SessionRoute: SessionModel] = [:]
  @ObservationIgnored private var landings: [LandingRoute: NewSessionModel] = [:]
  /// The Sessions' screens restored after a cold start, any of which found gone before it is read sends the reader
  /// back to the listing.
  @ObservationIgnored private var restored: Set<SessionRoute> = []

  init(core: any Core, path: [Pushed] = []) {
    self.core = core
    self.path = path
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

  /// The model of the new-session screen `route` pushed, made the first time it is asked for. The Session begun from
  /// it is held as any pushed Session is, for as long as the screen is on the path.
  func landing(for route: LandingRoute) -> NewSessionModel {
    if let model = landings[route] {
      return model
    }
    let model = NewSessionModel(core: core)
    model.adopted = { [weak self] begun in self?.models[begun.route] = begun }
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

  /// Shows the Session `session` names, chosen in the tree of the Session pushed as `route`: nothing more where it is
  /// that one, popping back to it where it is on the path already, as its own screen or as the new-session screen it
  /// was begun from, and pushing it over the path where it is not.
  func show(_ session: String, over route: SessionRoute) {
    guard route.session != session else { return }
    let pushed = path.lastIndex {
      switch $0 {
      case .session(let pushed): pushed.session == session
      case .landing(let landing): landings[landing]?.begun?.session == session
      case .work, .detail: false
      }
    }
    if let pushed {
      path.removeSubrange(path.index(after: pushed)...)
    } else {
      path.append(.session(SessionRoute(session: session)))
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
  /// as long as its screen does.
  private func closeUnpushed() {
    let landed = Set(path.compactMap { if case .landing(let route) = $0 { route } else { nil } })
    let pushed = Set(path.compactMap(\.route) + landed.compactMap { landings[$0]?.begun?.route })
    for (route, model) in models where !pushed.contains(route) {
      model.close()
      models[route] = nil
    }
    for route in landings.keys where !landed.contains(route) {
      landings[route] = nil
    }
  }
}

/// A Session's screen pushed over the listing: each push its own, so the same Session pushed twice is two screens.
struct SessionRoute: Hashable {
  let session: String
  let push = UUID()
}

/// The new-session screen pushed over the listing: each push its own.
struct LandingRoute: Hashable {
  let push = UUID()
}

/// A screen pushed over the listing: a Session's, or a Turn's work or an Activity's whole pushed from one, which
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
