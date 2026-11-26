import ShamanCore
import SwiftUI

/// The kind of thing the Aside holds: a Turn's work, or the tree of Sessions.
public enum AsideKind: String, Sendable {
  case work
  case tree
}

/// The Aside as a window has it, kept while the window lives: what it holds, whether the reader has it shown beside the
/// main view, and whether it has room there, as the columns say. The first two are each the reader's own choice, and
/// hold wherever the Aside stands and while it stands nowhere. Below the fold, where the device is sat like a laptop, it
/// stands by the pose alone, which `SessionStack` says.
@MainActor @Observable
final class Aside {
  // MARK: What it holds

  /// The kind of thing it holds, whichever Session is open.
  private(set) var kind: AsideKind
  /// How many times it was asked for the Turn whose work it is at, each of which returns to the top of that work.
  private(set) var returns = 0
  /// The place in the work of each Session's screen on the main view's path, each its own, so that Back finds a
  /// Session's as it was left; kept where it has one other than its latest Turn's work, following.
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
  /// Whether it stands beside the main view now, which the columns alone say.
  var standsBeside = false
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
  /// that is the one. Where it is at that Turn already, it returns to the top of its work.
  func holdWork(of turn: String, latest: String?, in route: SessionRoute) {
    kind = .work
    if turn == place(of: route).held ?? latest {
      returns += 1
    }
    places[route] = Place(held: turn == latest ? nil : turn)
  }

  /// Has it hold the work of the latest Turn, `latest` where there is one, of the Session open as `route`, following.
  func holdLatest(_ latest: String?, in route: SessionRoute) {
    if let latest {
      holdWork(of: latest, latest: latest, in: route)
    } else {
      kind = .work
      places[route] = nil
    }
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

  /// Has the Session open as `route` start afresh at its latest Turn's work, following, with nothing pushed over it, as
  /// one opened anew does. The kind stays as it stands.
  func startAfresh(_ route: SessionRoute) {
    places[route] = nil
  }

  /// Forgets the place of every Session's screen but `routes`, the rest having gone.
  func keepPlaces(of routes: Set<SessionRoute>) {
    places = places.filter { routes.contains($0.key) }
  }
}

/// The Aside's content, for the Session open on top: a Turn's work, with each Activity's whole pushed within it, or the
/// tree of Sessions, under a bar of its own that holds a title and Back, with the chooser between the two beneath it.
/// Only a Session's row pushes in the main view. It draws what `stack`'s Aside holds and keeps nothing itself, so it may
/// be drawn wherever the Aside stands.
struct AsideScreen: View {
  let stack: SessionStack
  /// The Session it answers for, as it does below the fold in that Session's own screen, whatever is pushed over that;
  /// with none handed, the one open on top.
  var session: SessionModel?

  var body: some View {
    // Its coming on screen is read here, outside what is drawn: a Session begun in a new-session screen may change
    // nothing else read here.
    content.environment(\.asideAppearances, stack.aside.appearances)
  }

  @ViewBuilder private var content: some View {
    if let session = session ?? stack.openSession.map(stack.model(for:)) {
      let route = session.route
      // The chooser beneath the rest and not a safe area inset over it, of which a list within takes no notice, so that
      // its last row would stand under it.
      VStack(spacing: 0) {
        Group {
          if stack.asideShowsTree(of: session) {
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
        .frame(maxHeight: .infinity)
        AsideChooser(session: session, stack: stack)
      }
    }
  }
}

/// The chooser beneath what the Aside holds, between its Session's work and its tree, where it has one, marking which
/// stands. "Work" gives the latest Turn's work, following; the other side the tree, by the title the core gives it. It
/// changes what the Aside holds and nothing of where that stands. Not a segmented picker, which would take no tap on
/// "Work" while the work of an earlier Turn stands.
private struct AsideChooser: View {
  let session: SessionModel
  let stack: SessionStack
  /// What the tree was last titled: until the next Session is framed the chooser stands as it did.
  @State private var title: String?

  var body: some View {
    let tree = session.frame == nil ? title : session.frame?.tree?.title
    let showsTree = stack.asideShowsTree(of: session)
    Group {
      if let tree {
        HStack(spacing: 8) {
          choice(Text("Work"), chosen: !showsTree) {
            stack.aside.holdLatest(session.conversation.latestTurn, in: session.route)
          }
          .accessibilityIdentifier("aside.work")
          choice(Text(verbatim: tree), chosen: showsTree) {
            stack.aside.holdTree()
          }
          .accessibilityIdentifier("aside.tree")
        }
        .padding(.horizontal, 12)
      } else {
        Color.clear.frame(height: 0)
      }
    }
    .onChange(of: tree, initial: true) { title = tree }
  }

  private func choice(_ words: Text, chosen: Bool, choose: @escaping () -> Void) -> some View {
    Button(action: choose) {
      words
        .font(.footnote.weight(.medium))
        .lineLimit(1)
        .frame(maxWidth: .infinity)
        .padding(.vertical, 7)
        .asideMark(chosen, over: Color(.tertiarySystemFill), cornerRadius: 8)
        // As tall to a tap as the chooser stands.
        .padding(.vertical, 6)
        .contentShape(Rectangle())
    }
    .buttonStyle(.plain)
    .accessibilityAddTraits(chosen ? .isSelected : [])
  }
}

extension View {
  /// Its ground, `ground`, or where it is `marked` how what the Aside is on is marked: the kind in its chooser, and the
  /// card whose work it shows.
  func asideMark(_ marked: Bool, over ground: Color, cornerRadius: CGFloat) -> some View {
    let shape = RoundedRectangle(cornerRadius: cornerRadius)
    return background(marked ? Color.accentColor.opacity(0.18) : ground, in: shape)
      .overlay {
        if marked {
          shape.strokeBorder(Color.accentColor, lineWidth: 1.5)
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
        session: session, turn: aside.place(of: route).held ?? latest, pushSession: { stack.push($0) },
        returns: aside.returns
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
