import ShamanCore
import SwiftUI

/// What the app shows when no Session is open: every Remote's Sessions in one flat list, active rows above the shelf
/// of settled ones, under a title that is a menu narrowing it to a Remote or a Workspace and a field searching it by
/// Title, and the bottom bar, with the sheet of Servers, Relays and This device behind its button.
struct ListingScreen: View {
  let core: any Core
  let appVersion: String
  let beginsIn: BeginsIn
  @State private var model: ListingModel
  @State private var sheet: ListingSheet?
  /// The Sessions pushed over the listing, each closed in the core as its screen is popped.
  @State private var stack: SessionStack
  /// What the search field holds, which the core is handed as it changes and keeps no longer than the core runs.
  @State private var query = ""

  init(core: any Core, appVersion: String, beginsIn: BeginsIn, invite: String?, opening: String? = nil) {
    self.core = core
    self.appVersion = appVersion
    self.beginsIn = beginsIn
    _model = State(initialValue: ListingModel(core: core))
    _sheet = State(initialValue: invite.map(ListingSheet.pairing))
    _stack = State(initialValue: SessionStack(core: core, path: opening.map { [SessionRoute(session: $0)] } ?? []))
  }

  var body: some View {
    NavigationStack(path: $stack.path) {
      List {
        if let listing = model.listing {
          rows(listing)
        }
      }
      .listStyle(.plain)
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
      .toolbar {
        ToolbarItem(placement: .principal) {
          if let listing = model.listing {
            ScopeMenu(listing: listing) { scope in Task { await model.narrow(scope) } }
          }
        }
        bottomBar
      }
      .navigationDestination(for: SessionRoute.self) { route in
        SessionScreen(model: stack.model(for: route))
      }
    }
    .task { await model.follow() }
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
    } else {
      shelves(listing)
    }
  }

  @ViewBuilder
  private func shelves(_ listing: Listing) -> some View {
    Section {
      ForEach(listing.active, id: \.session) { row in active(row) }
      if let waiting = listing.waiting {
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

  private func active(_ row: ActiveRow) -> some View {
    NavigationLink(value: SessionRoute(session: row.session)) {
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
    NavigationLink(value: SessionRoute(session: row.session)) {
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
      // Beginning a Session from the phone comes with the new-session screen.
      Button {
      } label: {
        Label("New session", systemImage: "square.and.pencil")
      }
      .disabled(true)
      .accessibilityIdentifier("home.compose")
    }
  }
}

/// A sheet over the listing.
private enum ListingSheet: Hashable, Identifiable {
  case servers
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

  /// Follows what the core lists until the screen goes.
  func follow() async {
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

/// The listing's title, as the menu that narrows it: Everywhere, then each Remote with all of it and its Workspaces,
/// the scope chosen ticked.
struct ScopeMenu: View {
  let listing: Listing
  let narrow: (String) -> Void

  var body: some View {
    Menu {
      ForEach(Array(listing.scopes.enumerated()), id: \.offset) { _, section in
        Section {
          Picker(selection: chosen) {
            ForEach(section.choices, id: \.scope) { choice in
              Label {
                Text(verbatim: choice.words)
              } icon: {
                choice.glyph.flatMap(Glyph.image) ?? Image(systemName: choice.kind.symbol)
              }
              .tag(choice.scope)
            }
          } label: {
            EmptyView()
          }
          .pickerStyle(.inline)
        } header: {
          if let header = section.header {
            Text(verbatim: header)
          }
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

  /// The scope chosen now, and choosing another narrows the listing to it.
  private var chosen: Binding<String> {
    Binding {
      listing.scopes.flatMap(\.choices).first(where: \.chosen)?.scope ?? ""
    } set: { scope in
      narrow(scope)
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
