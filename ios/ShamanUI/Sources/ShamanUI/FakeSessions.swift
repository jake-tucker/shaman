#if DEBUG
  import Foundation
  import ShamanCore

  /// One Session the fake's Remotes hold, from the listing's prototype, and how the fake makes the listing of them as
  /// the core would: merged across the Remotes held, the most recently begun first, active above the shelf, naming the
  /// Origin only where more than one Remote is held.
  struct FakeSession: Sendable {
    let id: String
    let remote: String
    let workspace: String
    let title: String
    /// An Icon's glyph, as the core resolves one from the Catalog.
    let glyph: String?
    let checkout: CheckoutLine?
    let standing: Standing?
    /// Seconds the Session has been Working or Monitoring.
    let running: TimeInterval
    /// Seconds since the Session last moved.
    var moved: TimeInterval
    var settled: Bool
    /// How its last Turn stands, which its open screen reads as.
    let reads: FakeConversation.Scenario

    static let shelfOpening = 10
    static let shelfBatch = 25

    /// The listing `sessions` make on the Remotes `remotes`, of which those `unanswered` have yet to answer, with as
    /// many as `onShow` of the shelf's rows on show, narrowed to `scope` and searched for `query`.
    static func listing(
      of sessions: [FakeSession], on remotes: [String], unanswered: [String], showing onShow: Int,
      scope: FakeScope = .everywhere, query: String = "", now: Date
    ) -> Listing {
      let scope = scope.within(remotes, sessions: sessions)
      let originNamed = remotes.count > 1
      let held = sessions.filter { remotes.contains($0.remote) && !unanswered.contains($0.remote) }
      let inScope = held.filter { scope.holds($0) }
      let active = inScope.filter { !$0.settled }.map { $0.active(originNamed: originNamed, now: now) }
      let settled = inScope.filter(\.settled).sorted { $0.moved < $1.moved }.map { $0.settledRow(now: now) }
      let found: [FoundRow]? =
        query.isEmpty
        ? nil
        : active.filter { $0.title.localizedCaseInsensitiveContains(query) }.map { FoundRow.active(row: $0) }
          + settled.filter { $0.title.localizedCaseInsensitiveContains(query) }.map { FoundRow.settled(row: $0) }
      let waiting = unanswered.filter { scope.reaches($0) }
      let nothing = found.map(\.isEmpty) ?? (active.isEmpty && settled.isEmpty)
      let empty: String?
      // A search says it found nothing while it goes on saying who is being reached.
      if !nothing || (!waiting.isEmpty && found == nil) {
        empty = nil
      } else {
        empty =
          switch (found, scope) {
          case (.some, _): "No sessions match"
          case (nil, .everywhere): "No sessions yet"
          case (nil, .workspace): "No sessions in this workspace"
          case (nil, .remote(let remote)): "No sessions on \(remote)"
          }
      }
      let hidden = settled.count - min(onShow, settled.count)
      return Listing(
        title: scope.title(originNamed: originNamed),
        scopes: menu(on: remotes, sessions: sessions, chosen: scope, originNamed: originNamed),
        searchPrompt: "Search by title",
        active: found == nil ? active : [],
        waiting: reaching(waiting, originNamed: originNamed),
        settledHeader: "Settled",
        settled: found == nil ? Array(settled.prefix(onShow)) : [],
        showMore: found == nil && hidden > 0 ? "Show \(min(hidden, shelfBatch)) more" : nil,
        found: found,
        empty: empty)
    }

    /// The Workspaces each Remote's Server lists, as the fake's Remotes would list them.
    static let registry: [String: [String]] = [
      "workstation": ["shaman", "dotfiles", "notes"],
      "laptop": ["design-system", "billing-api", "shaman"],
      "homelab": ["home-assistant", "relay-infra"],
    ]

    /// The glyphs of the Icons the fake's Workspaces carry.
    private static let workspaceGlyphs: [String: String] = [
      "shaman": "\u{f06a9}", "dotfiles": "\u{f018d}", "home-assistant": "\u{f04e6}", "design-system": "\u{f03d8}",
    ]

    /// The Workspaces the menu offers of the Remote `remote`: those its Server lists, then any other a Session of it is
    /// in, but never its Sidekick Workspace.
    static func workspaces(on remote: String, sessions: [FakeSession]) -> [String] {
      var offered: [String] = []
      for workspace in (registry[remote] ?? []) + sessions.filter({ $0.remote == remote }).map(\.workspace)
      where workspace != "Sidekick" && !offered.contains(workspace) {
        offered.append(workspace)
      }
      return offered
    }

    /// The menu the listing's title opens, as the core makes it.
    private static func menu(
      on remotes: [String], sessions: [FakeSession], chosen: FakeScope, originNamed: Bool
    ) -> [ScopeSection] {
      func choice(_ scope: FakeScope, _ kind: ScopeKind, _ glyph: String?, _ words: String) -> ScopeChoice {
        ScopeChoice(scope: scope.reference, kind: kind, glyph: glyph, words: words, chosen: scope == chosen)
      }
      let everywhere = ScopeSection(
        header: nil,
        choices: [choice(.everywhere, .everywhere, nil, FakeScope.everywhere.title(originNamed: originNamed))])
      return [everywhere]
        + remotes.map { remote in
          let whole = originNamed ? [choice(.remote(remote), .remote, nil, "All of \(remote)")] : []
          let workspaces = Self.workspaces(on: remote, sessions: sessions).map {
            choice(.workspace(remote, $0), .workspace, workspaceGlyphs[$0], $0)
          }
          return ScopeSection(header: originNamed ? remote : nil, choices: whole + workspaces)
        }
    }

    /// What the core says while the Remotes `unanswered` have yet to answer.
    private static func reaching(_ unanswered: [String], originNamed: Bool) -> String? {
      guard let last = unanswered.last else { return nil }
      guard originNamed else { return "Reaching the server…" }
      let named = unanswered.count == 1 ? last : "\(unanswered.dropLast().joined(separator: ", ")) and \(last)"
      return "Reaching \(named)…"
    }

    private func active(originNamed: Bool, now: Date) -> ActiveRow {
      let slot: Slot =
        switch standing {
        case .working: .running(words: "Working", since: now.addingTimeInterval(-running))
        case .monitoring: .running(words: "Monitoring", since: now.addingTimeInterval(-running))
        case .needsIntervention: .said(words: "Needs intervention")
        case .failed: .said(words: "Failed")
        case .done: .said(words: "Done")
        case nil: .moved(at: now.addingTimeInterval(-moved))
        }
      return ActiveRow(
        session: id, place: originNamed ? "\(remote) · \(workspace)" : workspace, glyph: glyph, title: title,
        checkout: checkout, standing: standing, slot: slot, settle: "Settle")
    }

    private func settledRow(now: Date) -> SettledRow {
      SettledRow(
        session: id, glyph: glyph, title: title, ended: now.addingTimeInterval(-moved), unsettle: "Unsettle")
    }

    private static let branch = "\u{ec6f}"
    private static let worktree = "\u{ec7e}"

    private static func on(_ name: String, linked: Bool = false) -> CheckoutLine {
      CheckoutLine(glyph: linked ? worktree : branch, words: name)
    }

    private static func session(
      _ number: Int, _ remote: String, _ workspace: String, _ title: String, _ glyph: String?,
      _ standing: Standing? = nil, checkout: CheckoutLine? = nil, running: TimeInterval = 0, moved: TimeInterval,
      settled: Bool = false, reads: FakeConversation.Scenario? = nil
    ) -> FakeSession {
      let byStanding: FakeConversation.Scenario =
        switch standing {
        case .working, .monitoring, .needsIntervention: .working
        case .failed: .failed
        case .done, nil: .settled
        }
      let read = reads ?? byStanding
      return FakeSession(
        id: "\(remote)-key/\(number)", remote: remote, workspace: workspace, title: title, glyph: glyph,
        checkout: checkout, standing: standing, running: running, moved: moved, settled: settled, reads: read)
    }

    /// What frames this Session's open screen at once, as the core frames it from what the listing knows.
    func frame(originNamed: Bool) -> SessionFrame {
      let place = ((originNamed ? [remote] : []) + [workspace] + [checkout?.words].compactMap { $0 })
        .joined(separator: " · ")
      return SessionFrame(title: title, place: place, gone: nil)
    }

    static let canned: [FakeSession] =
      [
        session(
          1, "workstation", "shaman", "Draw the invite as a QR code in /serve", "\u{f14de}", .working,
          checkout: on("feature/serve-qr", linked: true), running: 252, moved: 5),
        session(
          2, "homelab", "home-assistant", "Upgrade Home Assistant to 2026.11", "\u{f04e6}", .working,
          checkout: on("main"), running: 740, moved: 12),
        session(
          3, "workstation", "shaman", "Relax the exact-version rule for peers", "\u{f0339}", .needsIntervention,
          checkout: on("feature/peer-version-floor", linked: true), moved: 40),
        session(
          4, "laptop", "design-system", "Token colours for the button palette", "\u{f03d8}", .needsIntervention,
          checkout: on("tokens/buttons", linked: true), moved: 610),
        session(
          5, "laptop", "billing-api", "Port the invoice rounding fix to v2", "\u{f00e4}", .failed,
          checkout: on("fix/rounding-v2", linked: true), moved: 1_320),
        session(
          6, "homelab", "relay-infra", "Why the nightly backup skipped Tuesday", "\u{f01bc}", .monitoring,
          checkout: on("main"), running: 2_280, moved: 2_280),
        session(
          7, "workstation", "dotfiles", "Tidy zsh startup time", "\u{f018d}", .done, checkout: on("main"), moved: 3_700),
        session(
          8, "workstation", "shaman", "Stop /serve offering link-local addresses", "\u{f06f3}",
          checkout: on("fix/serve-link-local", linked: true), moved: 3 * 3_600, reads: .stopped),
        session(
          9, "workstation", "Sidekick", "Find the workspace that holds the deploy scripts", nil, .failed,
          moved: 5 * 3_600),
        session(
          10, "laptop", "shaman", "Sketch the thin client glossary entry", "\u{f0219}", checkout: on("main"),
          moved: 26 * 3_600),
      ] + settledHistory

    /// A long history of settled work, so the shelf has more than it opens on.
    private static let settledHistory: [FakeSession] = [
      ("workstation", "shaman", "Drop the /compact hint from the cache warning", "\u{f0a79}"),
      ("workstation", "shaman", "Let a peer withdraw from a pairing", "\u{f033e}"),
      ("homelab", "relay-infra", "Rotate the relay's TLS certificate", "\u{f030b}"),
      ("laptop", "billing-api", "Explain the March invoice discrepancy", nil),
      ("workstation", "shaman", "Move the session picker to semantic command ids", "\u{f0493}"),
      ("laptop", "design-system", "Add a dark variant of the focus ring token", "\u{f03d8}"),
      ("workstation", "shaman", "Pin the toolchain to 1.93", "\u{f05b7}"),
      ("workstation", "dotfiles", "Symlink the new ghostty config", "\u{f0339}"),
      ("homelab", "home-assistant", "Why the Zigbee dongle drops overnight", "\u{f06f3}"),
      ("workstation", "shaman", "Write ADR-0047 on relay compatibility", "\u{f0219}"),
      ("workstation", "shaman", "Name the glyph crate at its call sites", "\u{f03eb}"),
      ("workstation", "dotfiles", "Teach git to sign with the SSH key", "\u{f030b}"),
      ("workstation", "shaman", "Bundle the Symbols Nerd Font", "\u{f03d8}"),
      ("laptop", "shaman", "Measure the listing with several hundred sessions", "\u{f012a}"),
      ("workstation", "shaman", "Rename the release workflow", "\u{f14de}"),
      ("workstation", "shaman", "Trim the TUI's startup", "\u{f018d}"),
      ("workstation", "shaman", "Prune the stale worktrees", "\u{f0a79}"),
    ].enumerated().map { index, canned in
      session(
        100 + index, canned.0, canned.1, canned.2, canned.3, moved: TimeInterval(2 + index) * 86_400, settled: true)
    }
  }

  /// What the fake's listing is narrowed to, by the names of its Remotes and Workspaces, and the plain string it
  /// crosses as, in the core's shape: a Remote known by its key, which the fake's are named for.
  enum FakeScope: Equatable, Sendable {
    case everywhere
    case remote(String)
    case workspace(String, String)

    var reference: String {
      switch self {
      case .everywhere: "everywhere"
      case .remote(let remote): "remote/\(remote)-key"
      case .workspace(let remote, let workspace): "workspace/\(remote)-key/\(workspace)"
      }
    }

    /// The scope `reference` names, and Everywhere where it names none.
    init(_ reference: String) {
      let parts = reference.split(separator: "/", maxSplits: 2).map(String.init)
      let remote = parts.count > 1 && parts[1].hasSuffix("-key") ? String(parts[1].dropLast(4)) : nil
      switch (parts.first, remote, parts.count) {
      case ("remote", .some(let remote), 2): self = .remote(remote)
      case ("workspace", .some(let remote), 3): self = .workspace(remote, parts[2])
      default: self = .everywhere
      }
    }

    var remote: String? {
      switch self {
      case .everywhere: nil
      case .remote(let remote), .workspace(let remote, _): remote
      }
    }

    /// The scope as it stands among the Remotes `remotes`: Everywhere where it names one not held, or the one held,
    /// and the whole of its Remote where it names a Workspace that Remote does not offer.
    func within(_ remotes: [String], sessions: [FakeSession]) -> FakeScope {
      guard let remote, remotes.contains(remote) else { return .everywhere }
      let whole: FakeScope = remotes.count == 1 ? .everywhere : .remote(remote)
      switch self {
      case .everywhere, .remote: return whole
      case .workspace(_, let workspace):
        return FakeSession.workspaces(on: remote, sessions: sessions).contains(workspace) ? self : whole
      }
    }

    func holds(_ session: FakeSession) -> Bool {
      switch self {
      case .everywhere: true
      case .remote(let remote): session.remote == remote
      case .workspace(let remote, let workspace): session.remote == remote && session.workspace == workspace
      }
    }

    func reaches(_ remote: String) -> Bool {
      self.remote.map { $0 == remote } ?? true
    }

    func title(originNamed: Bool) -> String {
      switch self {
      case .everywhere: originNamed ? "Everywhere" : "All workspaces"
      case .remote(let remote): remote
      case .workspace(let remote, let workspace): originNamed ? "\(remote) · \(workspace)" : workspace
      }
    }
  }
#endif
