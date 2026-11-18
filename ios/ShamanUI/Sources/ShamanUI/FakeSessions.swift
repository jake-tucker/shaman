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

    static let shelfOpening = 10
    static let shelfBatch = 25

    /// The listing `sessions` make on the Remotes `remotes`, of which those `unanswered` have yet to answer, with as
    /// many as `onShow` of the shelf's rows on show.
    static func listing(
      of sessions: [FakeSession], on remotes: [String], unanswered: [String], showing onShow: Int, now: Date
    ) -> Listing {
      let held = sessions.filter { remotes.contains($0.remote) && !unanswered.contains($0.remote) }
      let originNamed = remotes.count > 1
      let active = held.filter { !$0.settled }
      let settled = held.filter(\.settled).sorted { $0.moved < $1.moved }
      let hidden = settled.count - min(onShow, settled.count)
      return Listing(
        title: originNamed ? "Everywhere" : "All workspaces",
        active: active.map { $0.active(originNamed: originNamed, now: now) },
        waiting: reaching(unanswered, originNamed: originNamed),
        settledHeader: "Settled",
        settled: settled.prefix(onShow).map { session in
          SettledRow(
            session: session.id, glyph: session.glyph, title: session.title,
            ended: now.addingTimeInterval(-session.moved), unsettle: "Unsettle")
        },
        showMore: hidden > 0 ? "Show \(min(hidden, shelfBatch)) more" : nil,
        empty: held.isEmpty && unanswered.isEmpty ? "No sessions yet" : nil)
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

    private static let branch = "\u{ec6f}"
    private static let worktree = "\u{ec7e}"

    private static func on(_ name: String, linked: Bool = false) -> CheckoutLine {
      CheckoutLine(glyph: linked ? worktree : branch, words: name)
    }

    private static func session(
      _ number: Int, _ remote: String, _ workspace: String, _ title: String, _ glyph: String?,
      _ standing: Standing? = nil, checkout: CheckoutLine? = nil, running: TimeInterval = 0, moved: TimeInterval,
      settled: Bool = false
    ) -> FakeSession {
      FakeSession(
        id: "\(remote)-key/\(number)", remote: remote, workspace: workspace, title: title, glyph: glyph,
        checkout: checkout, standing: standing, running: running, moved: moved, settled: settled)
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
          checkout: on("fix/serve-link-local", linked: true), moved: 3 * 3_600),
        session(
          9, "workstation", "Sidekick", "Find the workspace that holds the deploy scripts", nil, moved: 5 * 3_600),
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
#endif
