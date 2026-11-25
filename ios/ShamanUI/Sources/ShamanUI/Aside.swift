import ShamanCore
import SwiftUI

/// The kind of thing the Aside holds: a Turn's work, or the tree of Sessions.
public enum AsideKind: String, Sendable {
  case work
  case tree
}

/// The Aside as a window has it, kept while the window lives: what it holds, whether the reader has it shown beside the
/// main view, and whether it has room there, as the columns say. The first two are each the reader's own choice, and
/// hold wherever the Aside stands and while it stands nowhere.
@MainActor @Observable
final class Aside {
  // MARK: What it holds

  /// The kind of thing it holds, whichever Session is open.
  private(set) var kind: AsideKind
  /// Each Session's place in the work, where it has one other than its latest Turn's work, following.
  private var places: [SessionRoute: Place] = [:]

  /// A Session's place in the work the Aside shows.
  struct Place: Equatable {
    /// The Turn it holds at, as its card names it; none where it follows the latest.
    var held: String?
    /// What is pushed over that Turn's work: an Activity's whole.
    var pushed: [Pushed] = []
  }

  // MARK: Where it stands

  /// Whether the reader has it shown beside the main view, which is theirs to say whether it has room there or not.
  var shown: Bool {
    didSet { changed?() }
  }
  /// Told each time the reader asks for it or takes it down, as the columns are.
  @ObservationIgnored var changed: (() -> Void)?
  /// Whether it has room beside the main view, as the columns say.
  var hasRoom = false
  /// How many times it has come on screen beside the main view, as the columns say. What is drawn of it is kept while it
  /// is off screen, where SwiftUI starts no reading again as what is read changes, so each begins again as it comes.
  var appearances = 0

  /// The Aside as it begins in a window: hidden, at work; but shown, at `kind`, where one is given.
  init(showing kind: AsideKind? = nil) {
    self.kind = kind ?? .work
    shown = kind != nil
  }

  /// The place in the work of the Session open as `route`.
  func place(of route: SessionRoute) -> Place {
    places[route] ?? Place()
  }

  /// Has it hold the tree.
  func holdTree() {
    kind = .tree
  }

  /// Has it hold the work of the Turn `turn` of the Session open as `route`, whose latest is `latest`: following, where
  /// that is the one.
  func holdWork(of turn: String, latest: String?, in route: SessionRoute) {
    kind = .work
    places[route] = Place(held: turn == latest ? nil : turn)
  }

  /// Has it hold the whole of an Activity of the Session open as `route`, whose latest Turn is `latest`, pushed over
  /// the work it is at.
  func holdDetail(_ activity: String, latest: String?, in route: SessionRoute) {
    kind = .work
    push([.detail(route, activity: activity)], latest: latest, in: route)
  }

  /// Takes what is pushed over the work of the Session open as `route`, whose latest Turn is `latest`, as the reader
  /// pushes and pops it. While anything is, it holds at that work, which Back returns to.
  func push(_ pushed: [Pushed], latest: String?, in route: SessionRoute) {
    var place = place(of: route)
    place.pushed = pushed
    if !pushed.isEmpty {
      place.held = place.held ?? latest
    } else if place.held == latest {
      place.held = nil
    }
    places[route] = place
  }

  /// Forgets the place of every Session's screen but `routes`, the rest having gone.
  func keepPlaces(of routes: Set<SessionRoute>) {
    places = places.filter { routes.contains($0.key) }
  }
}

/// The Aside's content, for the Session open on top: a Turn's work, with each Activity's whole pushed within it, or the
/// tree of Sessions, under a bar of its own that holds a title and Back. Only a Session's row pushes in the main view.
/// It draws what `stack`'s Aside holds and keeps nothing itself, so it may be drawn wherever the Aside stands.
struct AsideScreen: View {
  let stack: SessionStack

  var body: some View {
    content.environment(\.asideAppearances, stack.aside.appearances)
  }

  @ViewBuilder private var content: some View {
    if let route = stack.openSession {
      let session = stack.model(for: route)
      // Until the Session is framed whatever stands stays, as the tree does from one Session of it to the next.
      if stack.aside.kind == .tree, session.frame == nil || session.frame?.tree != nil {
        NavigationStack {
          TreeScreen(model: session) { chosen in
            stack.show(chosen, over: route)
          }
        }
      } else {
        AsideWork(session: session, stack: stack)
          .id(route)
      }
    }
  }
}

/// The work of the Turn the Aside is at in `session`: the latest as each begins, or the one it holds at, and a line
/// saying so where there is no Turn.
private struct AsideWork: View {
  let session: SessionModel
  let stack: SessionStack

  var body: some View {
    let aside = stack.aside
    let route = session.route
    let latest = session.conversation.latestTurn
    NavigationStack(
      path: Binding(
        get: { aside.place(of: route).pushed },
        set: { aside.push($0, latest: latest, in: route) })
    ) {
      TurnWorkScreen(
        session: session, turn: aside.place(of: route).held ?? latest, pushSession: { stack.push($0) }
      )
      .navigationDestination(for: Pushed.self) { pushed in
        if case .detail(_, let activity) = pushed {
          ActivityDetailScreen(session: session, activity: activity)
        }
      }
    }
    // The Session's own screen follows it only while it is on top.
    .task(id: aside.appearances) { await session.followConversation() }
  }
}

extension EnvironmentValues {
  /// How many times the Aside has come on screen, in a screen it holds.
  @Entry var asideAppearances = 0
}

/// What a screen the Aside may hold reads, `what`, and the coming on screen it is read for: a reading begins again as
/// either changes.
struct Reading<What: Equatable>: Equatable {
  let what: What
  let appearances: Int
}

/// The button in an open Session's navigation bar that shows the Aside beside it, and takes it down.
struct AsideButton: View {
  let aside: Aside

  var body: some View {
    Button {
      aside.shown.toggle()
    } label: {
      Label(aside.shown ? "Hide aside" : "Show aside", systemImage: "sidebar.trailing")
    }
    .accessibilityIdentifier("session.aside")
  }
}
