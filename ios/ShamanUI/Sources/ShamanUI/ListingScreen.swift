import ShamanCore
import SwiftUI

/// What the app shows when no Session is open: every Remote's Sessions in one flat list, active rows above the shelf
/// of settled ones, under a title that is a menu narrowing it to a Remote or a Workspace and a field searching it by
/// Title, and the bottom bar, with the sheet of Servers, Relays and This device behind its button. It returns to what
/// the scene kept after a cold start, and hands the scene what to keep on each change.
struct ListingScreen: View {
  let core: any Core
  let appVersion: String
  let beginsIn: BeginsIn
  /// The scope the scene kept, handed back to the core as the listing is first followed.
  private let restoring: String?
  /// Hands the scene what to keep of the listing and what is pushed over it.
  private let keep: (KeptScene) -> Void
  @State private var model: ListingModel
  @State private var sheet: ListingSheet?
  /// The Sessions pushed over the listing, each closed in the core as its screen is popped.
  @State private var stack: SessionStack
  /// What the search field holds, which the core is handed as it changes and keeps no longer than the core runs.
  @State private var query = ""

  /// The listing over `core`, confirming `invite` where one is given, returned to what the scene kept, `kept`, which
  /// `keep` is handed again on each change; but with the Session `opening` names pushed over it, or the new-session
  /// screen where it is `composing`.
  init(
    core: any Core, appVersion: String, beginsIn: BeginsIn, invite: String?, kept: KeptScene = KeptScene(),
    keep: @escaping (KeptScene) -> Void = { _ in }, opening: String? = nil, composing: Bool = false
  ) {
    self.core = core
    self.appVersion = appVersion
    self.beginsIn = beginsIn
    self.restoring = kept.scope
    self.keep = keep
    _model = State(initialValue: ListingModel(core: core))
    _sheet = State(initialValue: invite.map(ListingSheet.pairing))
    // Pushed at launch as a tap pushes it, where a debug build asks; otherwise what the scene kept, restored.
    let pushed: [Pushed]? =
      composing ? [.landing(LandingRoute())] : opening.map { [.session(SessionRoute(session: $0))] }
    _stack = State(
      initialValue: pushed.map { SessionStack(core: core, path: $0) }
        ?? SessionStack(core: core, restoring: kept.pushed))
  }

  /// What the scene is to keep now: the scope the listing stands at, or the one it was handed back until it says, and
  /// what is pushed over it.
  private var kept: KeptScene {
    KeptScene(scope: model.listing?.scope ?? restoring, pushed: stack.kept)
  }

  @Bindable private var proto = Proto.shared
  @Environment(\.horizontalSizeClass) private var width
  @State private var columns = NavigationSplitViewVisibility.all
  @State private var asideSettling = false
  /// The fold, once the system has finished standing its own columns about it: widening the third column while it
  /// does so trips an assertion in UIKit's split view.
  @State private var settledFold: Proto.Fold?
  /// When the window's width class last changed, or the screen was made.
  @State private var settled = Date()
  @State private var compact = NavigationSplitViewColumn.sidebar

  var body: some View {
    Group {
      switch proto.variant {
      case .stack:
        NavigationStack(path: $stack.path) {
          listingColumn
            .navigationDestination(for: Pushed.self) { destination($0) }
        }
      case .two, .three:
        if proto.variant == .three {
          styled
            .inspector(isPresented: asideShown) {
              Group {
                if let session = stack.topSession {
                  AsideColumn(model: session, state: stack.aside, stack: stack)
                    .id(session.route)
                }
              }
              // Partly folded like a book, the column stands on the flat of the half past the fold, beginning where
              // the band kept for the fold ends; the conversation keeps the half before it, and the band stays empty.
              .inspectorColumnWidth(
                min: foldAside ?? 240, ideal: foldAside ?? 290, max: foldAside ?? 360)
            }
        } else {
          styled
        }
      }
    }
    .overlay(alignment: .top) {
      if proto.pill {
        PrototypePill().padding(.top, 52)
      }
    }
    .background {
      if #available(iOS 27.1, *) {
        FoldReader()
      }
    }
    .task(id: proto.fold) {
      let fold = proto.fold
      try? await Task.sleep(for: .milliseconds(900))
      guard !Task.isCancelled else { return }
      protoLog("settled fold \(String(describing: fold))")
      settledFold = fold
    }
    .onChange(of: width, initial: true) { proto.regular = width == .regular }
    .onGeometryChange(for: Bool.self) { $0.size.width > 800 } action: { proto.wide = $0 }
    .onChange(of: Arrangement(self), initial: true) { arrange() }
    .onChange(of: asideShown.wrappedValue ? settledFold.map { min(proto.foldGap, $0.width) } ?? 0 : 0, initial: true) { _, band in
      proto.asideBand = band
    }
    .onAppear { compact = stack.path.isEmpty ? .sidebar : .detail }
    .onChange(of: stack.path.isEmpty) { _, empty in
      // Collapsed, an empty path is the listing, and anything opened is the second column.
      compact = empty ? .sidebar : .detail
    }
    .onChange(of: compact) { protoLog("compact -> \(compact)") }
    .onChange(of: width) { settled = Date() }
    .onChange(of: proto.asides) { _, asides in
      // The system loses the third column through a fold; presenting it afresh once the window is wide again
      // brings it back.
      guard asides else { return }
      asideSettling = true
      Task {
        try? await Task.sleep(for: .milliseconds(400))
        stack.aside.presented = stack.aside.wanted
        asideSettling = false
      }
    }
    .task { await model.follow(restoring: restoring) }
    .onChange(of: kept, initial: true) { _, kept in keep(kept) }
    .onChange(of: model.listing?.unreachable) {
      // A Remote that answers again on its own takes its sheet with it.
      if case .unreachable(let remote) = sheet, model.unreachable(remote) == nil {
        sheet = nil
      }
    }
    .sheet(item: $sheet) { sheet in
      switch sheet {
      case .unreachable(let remote):
        // The sheet follows the Remote's row as the core changes it, so what it says may be in the way stays true.
        if let row = model.unreachable(remote) {
          UnreachableSheetView(row: row, model: TryAgainModel(core: core, remote: remote)) {
            self.sheet = nil
          }
        }
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

  /// One split view on every iPhone. The second column's root is the first screen on the path, a Session or the
  /// new-session screen, and the rest of the path is pushed over it within the column; with nothing on the path it
  /// holds what the pill's menu says. Collapsed, the listing stands while the path is empty.
  private var split: some View {
    NavigationSplitView(columnVisibility: $columns, preferredCompactColumn: $compact) {
      listingColumn
    } detail: {
      let first = stack.path.first
      NavigationStack(path: $stack.rest) {
        Group {
          if let first {
            // Each Session opened is a screen of its own, read afresh.
            destination(first)
              .id(first)
          } else {
            switch proto.empty {
            case .landing:
              NewSessionScreen(model: stack.landing(for: stack.rootLanding), stack: stack, arrivesWriting: false)
                .noticeBanner()
                .id(stack.rootLanding)
            case .nothing:
              ContentUnavailableView("No session open", systemImage: "bubble.left.and.text.bubble.right")
            }
          }
        }
        .navigationDestination(for: Pushed.self) { destination($0) }
      }
    }
  }

  @ViewBuilder
  private var styled: some View {
    switch proto.style {
    case .balanced: split.navigationSplitViewStyle(.balanced)
    case .automatic: split.navigationSplitViewStyle(.automatic)
    case .prominent: split.navigationSplitViewStyle(.prominentDetail)
    }
  }

  /// What decides which columns stand.
  private struct Arrangement: Equatable {
    let variant: Proto.Variant
    let regular: Bool
    let wide: Bool
    let root: Pushed?
    let aside: Bool
    let hides: Bool

    @MainActor
    init(_ screen: ListingScreen) {
      variant = screen.proto.variant
      regular = screen.proto.regular
      wide = screen.proto.wide
      root = screen.stack.path.first
      aside = screen.asideShown.wrappedValue
      hides = screen.proto.asideHidesListing
    }
  }

  /// Stands the columns for where the window is now. Tiled, the listing stands beside what is open, but where the
  /// third column takes its place. Too narrow to tile, as the inner display in portrait, the listing stands over the
  /// second column while nothing is open, and goes as something opens, as it does on the closed display.
  private func arrange() {
    guard proto.variant != .stack, proto.regular else { return }
    if !proto.wide {
      columns = stack.path.isEmpty ? .all : .detailOnly
    } else if proto.variant == .three, proto.asideHidesListing, asideShown.wrappedValue {
      columns = .detailOnly
    } else {
      columns = .all
    }
  }

  /// How wide the third column is while the phone is partly folded like a book: from where the fold begins to the
  /// window's far edge.
  private var foldAside: CGFloat? {
    settledFold.map { $0.window - $0.leading - min(proto.foldGap, $0.width) }
  }

  /// Whether the third column stands: where the variant has one, a Session's screen is up and the reader has it shown.
  private var asideShown: Binding<Bool> {
    Binding {
      proto.asides && !asideSettling && stack.topSession != nil && stack.aside.presented
    } set: { presented in
      // The system takes it down by itself, as through a fold; the reader's standing choice is kept apart.
      protoLog("aside presented -> \(presented)")
      stack.aside.presented = presented
    }
  }

  /// What is pushed over the second column's root.
  private var rest: Binding<[Pushed]> {
    Binding {
      Array(stack.path.dropFirst())
    } set: { rest in
      protoLog("rest set \(rest.map(\.tag))")
      stack.path = Array(stack.path.prefix(1)) + rest
    }
  }

  /// The listing came on screen. Collapsed, with something open, that is the reader coming back to it, which closes
  /// what was open. The column the system writes cannot say so: in iOS 27.2 it writes the listing as any screen
  /// pushed within the second column is popped, and nothing once it has. A fold re-hosts the listing, so its
  /// appearing just after the width changes is passed over.
  private func listingAppeared() {
    protoLog("listing appeared width \(String(describing: width)) path \(stack.path.map(\.tag))")
    guard proto.variant != .stack, width == .compact, !stack.path.isEmpty else { return }
    guard Date().timeIntervalSince(settled) > 1 else { return }
    stack.path = []
  }

  /// A row that opens its Session: pushed in the stack, and in the split view made the second column's root.
  @ViewBuilder
  private func opens(_ session: String, @ViewBuilder label: () -> some View) -> some View {
    if proto.variant == .stack {
      NavigationLink(value: Pushed.session(SessionRoute(session: session))) { label() }
    } else {
      Button {
        if !stack.roots(session) {
          stack.path = [.session(SessionRoute(session: session))]
        }
      } label: {
        label()
          .frame(maxWidth: .infinity, alignment: .leading)
          .contentShape(Rectangle())
      }
      .buttonStyle(.plain)
      .listRowBackground(stack.roots(session) && proto.regular ? Color.accentColor.opacity(0.14) : nil)
    }
  }

  @ViewBuilder
  private func destination(_ pushed: Pushed) -> some View {
    switch pushed {
    case .session(let route):
      SessionScreen(model: stack.model(for: route), stack: stack)
    case .work(let route, let turn):
      TurnWorkScreen(session: stack.model(for: route), turn: turn)
        .noticeBanner()
    case .detail(let route, let activity):
      ActivityDetailScreen(session: stack.model(for: route), activity: activity)
        .noticeBanner()
    case .landing(let route):
      NewSessionScreen(model: stack.landing(for: route), stack: stack)
        .noticeBanner()
    }
  }

  private var listingColumn: some View {
    List {
      if let listing = model.listing {
        rows(listing)
      }
    }
    .listStyle(.plain)
    .onAppear { listingAppeared() }
    .overlay {
      if let empty = model.listing?.empty {
        ContentUnavailableView {
          Label {
            Text(verbatim: empty)
              .accessibilityIdentifier("listing.empty")
          } icon: {
            Image(systemName: model.listing?.found == nil ? "square.and.pencil" : "magnifyingglass")
          }
        }
      }
    }
    .navigationTitle(model.listing?.title ?? "")
    .navigationBarTitleDisplayMode(.inline)
    .searchable(
      text: $query, placement: .navigationBarDrawer(displayMode: .always),
      prompt: Text(verbatim: model.listing?.searchPrompt ?? "")
    )
    .onChange(of: query) { model.search(query) }
    .safeAreaInset(edge: .top) {
      if let failure = model.failure {
        Banner(text: failure)
          .onTapGesture { model.dismissFailure() }
          .accessibilityAddTraits(.isButton)
          .accessibilityHint("Dismisses this")
      }
    }
    .noticeBanner(covered: sheet != nil)
    .toolbar {
      ToolbarItem(placement: .principal) {
        if let listing = model.listing {
          ScopeMenu(listing: listing) { scope in Task { await model.narrow(scope) } }
        }
      }
      bottomBar
    }
  }

  /// The compose button: pushed over the listing in the stack, and in the split view the second column's own root
  /// where that is what it holds while no Session is open.
  private func compose() {
    if proto.variant == .stack {
      stack.path.append(.landing(LandingRoute()))
    } else {
      stack.path = [.landing(LandingRoute())]
    }
  }

  @ViewBuilder
  private func rows(_ listing: Listing) -> some View {
    if let found = listing.found {
      // A search stands in place of both shelves, each row keeping its shelf's shape.
      ForEach(found, id: \.session) { row in
        switch row {
        case .active(let row): active(row)
        case .settled(let row): settled(row)
        }
      }
      if let waiting = listing.waiting {
        waitingRow(waiting)
      }
      unreachableRows(listing)
    } else {
      shelves(listing)
    }
  }

  @ViewBuilder
  private func shelves(_ listing: Listing) -> some View {
    Section {
      ForEach(listing.active, id: \.session) { row in active(row) }
      if let waiting = listing.waiting {
        waitingRow(waiting)
      }
      unreachableRows(listing)
    }
    if !listing.settled.isEmpty {
      Section {
        ForEach(listing.settled, id: \.session) { row in settled(row) }
        if let showMore = listing.showMore {
          Button(showMore) { model.showMore() }
            .font(.footnote)
            .accessibilityIdentifier("listing.showMore")
        }
      } header: {
        Text(verbatim: listing.settledHeader)
          .accessibilityIdentifier("listing.settledHeader")
      }
    }
  }

  /// What stands at the foot of the rows while a Remote in scope has yet to answer.
  private func waitingRow(_ waiting: String) -> some View {
    HStack(spacing: 8) {
      ProgressView()
        .controlSize(.mini)
      Text(verbatim: waiting)
    }
    .font(.footnote)
    .foregroundStyle(.secondary)
    .accessibilityElement(children: .combine)
    .accessibilityIdentifier("listing.waiting")
  }

  /// A slim row at the foot of the active rows for each Remote that reads Unreachable, opening the sheet that offers
  /// to try it again.
  private func unreachableRows(_ listing: Listing) -> some View {
    ForEach(listing.unreachable, id: \.remote) { row in
      Button {
        sheet = .unreachable(row.remote)
      } label: {
        HStack(spacing: 8) {
          Image(systemName: row.sheet.hint.symbol)
            .accessibilityHidden(true)
          Text(verbatim: row.words)
          Spacer(minLength: 8)
          Text(verbatim: row.tryAgain)
            .foregroundStyle(.tint)
        }
        .font(.footnote)
        .foregroundStyle(.secondary)
      }
      .accessibilityElement(children: .combine)
      .accessibilityIdentifier("listing.unreachable")
    }
  }

  private func active(_ row: ActiveRow) -> some View {
    opens(row.session) {
      ActiveRowView(row: row)
    }
    .accessibilityIdentifier("listing.active")
    .swipeActions(edge: .trailing) {
      if let settle = row.settle {
        Button(settle) { Task { await model.settle(row.session) } }
          .tint(.indigo)
      }
    }
  }

  private func settled(_ row: SettledRow) -> some View {
    opens(row.session) {
      SettledRowView(row: row)
    }
    .accessibilityIdentifier("listing.settled")
    .swipeActions(edge: .trailing) {
      Button(row.unsettle) { Task { await model.unsettle(row.session) } }
        .tint(.indigo)
    }
  }

  @ToolbarContentBuilder
  private var bottomBar: some ToolbarContent {
    ToolbarItem(placement: .bottomBar) {
      Button {
        sheet = .servers
      } label: {
        Label("Servers & device", systemImage: "server.rack")
      }
      .accessibilityIdentifier("home.servers")
    }
    ToolbarItem(placement: .status) {
      VStack(spacing: 1) {
        Text(verbatim: beginsIn.lead)
          .font(.caption2)
          .foregroundStyle(.secondary)
        Text(verbatim: beginsIn.place)
          .font(.caption.weight(.medium))
          .lineLimit(1)
          .truncationMode(.middle)
      }
      .accessibilityElement(children: .ignore)
      .accessibilityLabel(Text(verbatim: beginsIn.said))
      .accessibilityIdentifier("home.beginsIn")
    }
    ToolbarItem(placement: .bottomBar) {
      // The new-session screen, where the Outlook stands.
      Button {
        compose()
      } label: {
        Label("New session", systemImage: "square.and.pencil")
      }
      .accessibilityIdentifier("home.compose")
    }
  }
}

/// A sheet over the listing.
private enum ListingSheet: Hashable, Identifiable {
  case servers
  /// The offer to try the Unreachable Remote it names again.
  case unreachable(String)
  /// Confirming an Invite that arrived by the launch environment.
  case pairing(String)

  var id: Self { self }
}

/// What the listing holds while it is up.
@MainActor @Observable
final class ListingModel {
  private let core: any Core
  private(set) var listing: Listing?
  /// Why the last act failed, shown in a banner over the listing until it is tapped away or has stood a while.
  private(set) var failure: String?
  /// How long a failure's banner stands before it goes by itself.
  private let failureStands: Duration
  /// What takes the banner away once it has stood its while.
  private var clearing: Task<Void, Never>?

  init(core: any Core, failureStands: Duration = .seconds(6)) {
    self.core = core
    self.failureStands = failureStands
  }

  /// Takes the banner away.
  func dismissFailure() {
    clearing?.cancel()
    clearing = nil
    failure = nil
  }

  /// The slim row of the Unreachable Remote `remote` names, while it reads Unreachable.
  func unreachable(_ remote: String) -> UnreachableRow? {
    listing?.unreachable.first { $0.remote == remote }
  }

  /// Follows what the core lists until the screen goes, first narrowed to the scope `restoring` names, as the scene
  /// kept it, where one is.
  func follow(restoring: String? = nil) async {
    if let restoring {
      core.restoreListing(restoring)
    }
    for await listing in core.listing() {
      self.listing = listing
    }
  }

  func settle(_ session: String) async {
    await act { () async throws(ActError) in try await core.settle(session) }
  }

  func unsettle(_ session: String) async {
    await act { () async throws(ActError) in try await core.unsettle(session) }
  }

  func showMore() {
    core.showMoreSettled()
  }

  /// Narrows the listing to `scope`, which turns the Outlook where it names a Remote or a Workspace.
  func narrow(_ scope: String) async {
    await act { () async throws(ActError) in try await core.narrowListing(scope) }
  }

  /// Searches the listing by Title for `text`.
  func search(_ text: String) {
    core.searchListing(text)
  }

  private func act(_ act: () async throws(ActError) -> Void) async {
    do {
      try await act()
      dismissFailure()
    } catch {
      show(error.message)
    }
  }

  private func show(_ message: String) {
    clearing?.cancel()
    failure = message
    let stands = failureStands
    clearing = Task { [weak self] in
      try? await Task.sleep(for: stands)
      guard !Task.isCancelled else { return }
      self?.failure = nil
    }
  }
}

/// The short sheet an Unreachable Remote's slim row opens: why it may not be answering, the offer to try it again, and
/// where every way to it is on the local network, the way to the app's page in iOS Settings.
struct UnreachableSheetView: View {
  let row: UnreachableRow
  @State var model: TryAgainModel
  /// Closes the sheet, once the Remote answers.
  let answered: () -> Void
  @Environment(\.openURL) private var openURL
  /// The login at a Relay the sheet's own sheet is carrying out, while it is up.
  @State private var loggingIn: LoginAsk?

  var body: some View {
    VStack(alignment: .leading, spacing: 14) {
      Label {
        Text(verbatim: row.sheet.title)
          .font(.headline)
      } icon: {
        Image(systemName: row.sheet.hint.symbol)
          .foregroundStyle(.secondary)
      }
      .accessibilityIdentifier("unreachable.title")
      Text(verbatim: row.sheet.explanation)
        .foregroundStyle(.secondary)
        .fixedSize(horizontal: false, vertical: true)
        .accessibilityIdentifier("unreachable.explanation")
      Spacer(minLength: 0)
      Button {
        // Where a login at a Relay leads past what is in the way, trying again leads to it, and follows it.
        if let login = row.sheet.login {
          loggingIn = LoginAsk(relay: login.relay)
          return
        }
        Task {
          if await model.tryAgain() {
            answered()
          }
        }
      } label: {
        ZStack {
          Text(verbatim: row.sheet.login?.words ?? row.sheet.tryAgain).opacity(model.trying ? 0 : 1)
          if model.trying {
            ProgressView()
          }
        }
        .frame(maxWidth: .infinity)
      }
      .buttonStyle(.borderedProminent)
      .controlSize(.large)
      .disabled(model.trying)
      .accessibilityIdentifier("unreachable.tryAgain")
      if let openSettings = row.sheet.openSettings, let settings = URL(string: UIApplication.openSettingsURLString) {
        Button {
          openURL(settings)
        } label: {
          Text(verbatim: openSettings)
            .frame(maxWidth: .infinity)
        }
        .buttonStyle(.bordered)
        .controlSize(.large)
        .accessibilityIdentifier("unreachable.openSettings")
      }
    }
    .padding(24)
    .safeAreaInset(edge: .top) {
      // A try that did not reach it stands in the banner every act's failure does.
      if let failure = model.failure {
        Banner(text: failure)
          .padding(.top, 12)
          .onTapGesture { model.dismissFailure() }
          .accessibilityAddTraits(.isButton)
          .accessibilityHint("Dismisses this")
      }
    }
    .presentationDetents([.medium])
    .relayLogin($loggingIn, core: model.core) { _ in
      Task {
        if await model.tryAgain() {
          answered()
        }
      }
    }
  }
}

/// Trying an Unreachable Remote again, from its sheet.
@MainActor @Observable
final class TryAgainModel {
  let core: any Core
  private let remote: String
  /// Whether the core is trying it now, during which the sheet takes no second try.
  private(set) var trying = false
  /// Why the last try did not reach it, in the core's words, shown in a banner over the sheet until it is tapped away
  /// or tried again.
  private(set) var failure: String?

  init(core: any Core, remote: String) {
    self.core = core
    self.remote = remote
  }

  /// Takes the banner away.
  func dismissFailure() {
    failure = nil
  }

  /// Tries the Remote again, answering whether it answered.
  func tryAgain() async -> Bool {
    failure = nil
    trying = true
    defer { trying = false }
    do {
      try await core.tryAgain(remote)
      failure = nil
      return true
    } catch {
      failure = error.message
      return false
    }
  }
}

extension UnreachableHint {
  /// The symbol an Unreachable Remote's row and sheet are drawn with, or one that answers but works with the phone no
  /// more until a side is updated.
  var symbol: String {
    switch self {
    case .plain: "exclamationmark.icloud"
    case .localNetwork: "wifi.exclamationmark"
    case .noNetwork: "wifi.slash"
    case .loginNeeded: "person.badge.key"
    case .otherAccount: "person.2.slash"
    case .updateApp: "arrow.down.app"
    case .updateServer: "server.rack"
    }
  }
}

/// The listing's title, as the menu that narrows it: Everywhere, then each Remote with all of it and its Workspaces,
/// the scope chosen ticked.
struct ScopeMenu: View {
  let listing: Listing
  let narrow: (String) -> Void

  var body: some View {
    Menu {
      ForEach(Array(listing.scopes.enumerated()), id: \.offset) { _, section in
        // A Remote's section is headed by its name, so same-named Workspaces on two Remotes are told apart.
        if let header = section.header {
          Section(header) { choices(section) }
        } else {
          Section { choices(section) }
        }
      }
    } label: {
      HStack(spacing: 4) {
        Text(verbatim: listing.title)
          .font(.headline)
          .lineLimit(1)
          .truncationMode(.middle)
        Image(systemName: "chevron.down")
          .font(.caption2.weight(.bold))
          .accessibilityHidden(true)
      }
      .foregroundStyle(.primary)
    }
    .accessibilityIdentifier("listing.scope")
    .accessibilityLabel(Text(verbatim: listing.title))
  }

  /// A section's choices, the one chosen ticked, and choosing another narrows the listing to it.
  private func choices(_ section: ScopeSection) -> some View {
    ForEach(section.choices, id: \.scope) { choice in
      Toggle(
        isOn: Binding {
          choice.chosen
        } set: { _ in
          narrow(choice.scope)
        }
      ) {
        Label {
          Text(verbatim: choice.words)
        } icon: {
          choice.glyph.flatMap(Glyph.image) ?? Image(systemName: choice.kind.symbol)
        }
      }
    }
  }
}

extension FoundRow {
  /// The Session the row is of.
  var session: String {
    switch self {
    case .active(let row): row.session
    case .settled(let row): row.session
    }
  }
}

extension ScopeKind {
  /// The symbol a choice of the scope menu is drawn with.
  var symbol: String {
    switch self {
    case .everywhere: "globe"
    case .remote: "server.rack"
    case .workspace: "folder"
    }
  }
}

/// An active Session as its three lines say it, with the Rail down its left in its Standing's colour.
struct ActiveRowView: View {
  let row: ActiveRow

  var body: some View {
    HStack(spacing: 10) {
      RoundedRectangle(cornerRadius: 2)
        .fill(row.standing.map(\.color) ?? .clear)
        .frame(width: 4)
        .accessibilityHidden(true)
      VStack(alignment: .leading, spacing: 3) {
        HStack(alignment: .firstTextBaseline) {
          Text(verbatim: row.place)
            .font(.caption)
            .foregroundStyle(.secondary)
            .lineLimit(1)
          Spacer(minLength: 8)
          SlotView(slot: row.slot, standing: row.standing)
        }
        HStack(alignment: .firstTextBaseline, spacing: 6) {
          if let glyph = row.glyph {
            Glyph(glyph: glyph)
              .foregroundStyle(.secondary)
          }
          Text(verbatim: row.title)
            .font(.body.weight(.medium))
            .lineLimit(2)
        }
        if let checkout = row.checkout {
          HStack(alignment: .firstTextBaseline, spacing: 4) {
            if let glyph = checkout.glyph {
              Glyph(glyph: glyph, style: .caption)
            }
            Text(verbatim: checkout.words)
              .lineLimit(1)
          }
          .font(.caption)
          .foregroundStyle(.tertiary)
        }
      }
    }
    .padding(.vertical, 2)
    .opacity(row.dimmed ? 0.45 : 1)
    .accessibilityElement(children: .combine)
  }
}

/// A settled Session as its one slim line says it: what the work was, and how long ago it ended.
struct SettledRowView: View {
  let row: SettledRow

  var body: some View {
    HStack(alignment: .firstTextBaseline, spacing: 8) {
      if let glyph = row.glyph {
        Glyph(glyph: glyph, style: .subheadline)
      }
      Text(verbatim: row.title)
        .lineLimit(1)
      Spacer(minLength: 8)
      TimelineView(.everyMinute) { context in
        Text(verbatim: RunningTime.compact(since: row.ended, now: context.date))
          .font(.caption)
          .monospacedDigit()
      }
    }
    .font(.subheadline)
    .foregroundStyle(.secondary)
    .opacity(row.dimmed ? 0.45 : 1)
    .accessibilityElement(children: .combine)
  }
}

/// What an active row's right slot reads: the Standing's word, with how long it has held where it ticks, or the
/// compact time since the Session last moved.
struct SlotView: View {
  let slot: Slot
  let standing: Standing?

  var body: some View {
    switch slot {
    case .said(let words):
      Text(verbatim: words)
        .font(.caption.weight(.medium))
        .foregroundStyle(standing.map(\.color) ?? .secondary)
    case .running(let words, let since):
      TimelineView(.periodic(from: .now, by: 1)) { context in
        Text(verbatim: "\(words) \(RunningTime.duration(since: since, now: context.date))")
      }
      .font(.caption.weight(.medium))
      .monospacedDigit()
      .foregroundStyle(standing.map(\.color) ?? .secondary)
    case .moved(let at):
      TimelineView(.everyMinute) { context in
        Text(verbatim: RunningTime.compact(since: at, now: context.date))
      }
      .font(.caption)
      .monospacedDigit()
      .foregroundStyle(.secondary)
    }
  }
}

extension Standing {
  /// The colour a Rail, and the word in the right slot, are drawn in.
  var color: Color {
    switch self {
    case .needsIntervention: .orange
    case .working, .monitoring: .blue
    case .failed: .red
    case .done: .green
    }
  }
}

#if DEBUG
  #Preview("One server") {
    ListingScreen(
      core: FakeCore(.paired), appVersion: "0.1.0 (1)",
      beginsIn: BeginsIn(
        lead: "New sessions begin in", place: "workstation", said: "New sessions begin in workstation"),
      invite: nil)
  }

  #Preview("Several servers") {
    ListingScreen(
      core: FakeCore(.several), appVersion: "0.1.0 (1)",
      beginsIn: BeginsIn(
        lead: "New sessions begin in", place: "workstation", said: "New sessions begin in workstation"),
      invite: nil)
  }
#endif
