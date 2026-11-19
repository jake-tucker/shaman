#if DEBUG
  import Foundation
  import ShamanCore

  /// The tree of a fake Sidekick's Session on the workstation, as the core would make it: headed "Sessions", the
  /// Sidekick's own Session first, then its Subsessions and the Sessions it acted on, working first and then the one
  /// it acted on most recently. The workstation names its Remotes its own way — it calls the laptop "build" — and the
  /// phone opens each entry on another Server through its own Pairing with that Server, under its own name for it;
  /// one on a Server the phone is not paired with opens nothing and says so. A Subsession's row among the Sidekick's
  /// Turn's work opens by the same rule.
  enum FakeSidekick {
    /// The Sidekick's Session.
    static let top = "workstation-key/20"
    static let title = "Plan the week"
    static let prompt = "Plan this week's work across the machines, and start on what can start now."

    /// A Subsession on the workstation, which no listing carries, its Sidekick's row speaking for it.
    static let docs = "workstation-key/21"
    /// A Session it acted on, on the laptop.
    static let parser = "laptop-key/22"
    /// A Subsession on the home lab, which the phone cannot reach though the workstation can.
    static let certs = "homelab-key/23"
    /// A Subsession on the laptop, whose tree there runs past what the workstation keeps of it.
    static let crate = "laptop-key/24"
    /// A Session on the laptop the Sidekick's act on is not known to have reached.
    static let unconfirmed = "laptop-key/26"

    /// The Sessions it has a hand in that a listing carries.
    static let listed: [FakeSession] = [
      FakeSession(
        id: top, remote: "workstation", workspace: "Sidekick", title: title, glyph: nil, checkout: nil,
        standing: .working, running: 95, moved: 1, settled: false, reads: .settled),
      FakeSession(
        id: parser, remote: "laptop", workspace: "billing-api", title: "Write the parser", glyph: nil,
        checkout: CheckoutLine(glyph: nil, words: "main"), standing: .working, running: 40, moved: 2, settled: false,
        reads: .working),
    ]

    /// The Sessions it has a hand in that no listing carries, by what frames each once read.
    static let unlisted: [String: (title: String, place: String)] = [
      docs: ("Tidy the docs", "workstation · shaman"),
      certs: ("Rotate the relay certs", "homelab · relay-infra"),
      crate: ("Map the crate", "laptop · shaman"),
      unconfirmed: ("Cover the empty input", "laptop · billing-api"),
    ]

    /// Whether `session` is one whose tree is the Sidekick's: its own Session, or a Subsession on its own Server.
    static func holds(_ session: String) -> Bool {
      session == top || session == docs
    }

    /// The frame of the unlisted Session `session`, a top-level Session taking Prompts, once read; where its Remote
    /// reads `unreachable`, its composer takes nothing and says why.
    static func frame(of session: String, unreachable: String?) -> SessionFrame? {
      guard let (title, place) = unlisted[session] else { return nil }
      return SessionFrame(
        title: title, place: place, gone: nil, bar: nil,
        composer: FakeOpenedSession.composer(working: false, unreachable: unreachable), queued: [],
        intervention: nil, tree: holds(session) ? button : nil, waiting: [])
    }

    /// The button the tree is opened by.
    static let button = TreeButton(working: "1", label: "7 sessions, 1 working")

    /// The glyph of a folder, which a Workspace with no Icon of its own is led by.
    private static let folder = "\u{ea83}"

    /// One entry of the sheet.
    private static func entry(
      _ id: String, _ session: String?, title: String, place: String, line: String, marker: Tone?,
      trailing: String? = nil, since: Date? = nil, note: String? = nil, dimmed: Bool = false, open: String
    ) -> TreeRow {
      TreeRow(
        id: id, session: session, depth: 1, marker: marker, title: title, place: place, placeGlyph: folder,
        line: line, note: note, dimmed: dimmed, trailing: trailing, needsYou: false, since: since,
        open: session == open, stop: nil)
    }

    /// The sheet over `open`, as of `now`.
    static func sheet(for open: String, now: Date = Date()) -> SubagentTree {
      let rows = [
        TreeRow(
          id: top, session: top, depth: 0, marker: .running, title: title, place: "Sidekick", placeGlyph: folder,
          line: "Claude · opus", note: nil, dimmed: false, trailing: nil, needsYou: false,
          since: now.addingTimeInterval(-95), open: open == top, stop: nil),
        entry(
          "22@build", parser, title: "Write the parser", place: "laptop · billing-api", line: "Codex · gpt-5",
          marker: .running, since: now.addingTimeInterval(-40), open: open),
        entry(
          docs, docs, title: "Tidy the docs", place: "shaman", line: "Claude · sonnet", marker: .succeeded,
          trailing: "41s", open: open),
        entry(
          "24@build", crate, title: "Map the crate", place: "laptop · shaman",
          line: "Codex · gpt-5 · open for all subagents", marker: .succeeded, trailing: "2m 5s", open: open),
        entry(
          "23@homelab", certs, title: "Rotate the relay certs", place: "homelab · relay-infra",
          line: "Claude · opus", marker: .succeeded, trailing: "5m 2s", open: open),
        entry(
          "31@attic", nil, title: "Fix the lights", place: "attic · home-assistant", line: "Claude · sonnet",
          marker: .failed, trailing: "12s", note: "This phone is not paired with attic", open: open),
        entry(
          "26@build", unconfirmed, title: "Cover the empty input", place: "laptop · billing-api",
          line: "not confirmed", marker: nil, dimmed: true, open: open),
        entry(
          "32@nas", nil, title: "Back up the photos", place: "nas · photos", line: "not answering", marker: nil,
          note: "This phone is not paired with nas", dimmed: true, open: open),
      ]
      return SubagentTree(title: "Sessions", count: "7 · 1 working", loading: nil, failed: nil, rows: rows)
    }

    /// The conversation of the Sidekick's Session.
    static let items: [ConversationItem] = [
      FakeConversation.user("open", prompt),
      FakeConversation.card("work", .working, "Map the crate", "4 subsessions"),
    ]

    /// The work of the Sidekick's Turn: the Subsessions it began, each opening by the rule its tree's entries do.
    static func work() -> TurnWork {
      func subsession(_ id: String, _ title: String, _ line: String, _ trailing: String?, _ tap: Tap) -> WorkItem {
        WorkItem(
          id: "activity:\(id)",
          kind: .row(
            row: WorkRow(
              glyph: "\u{eb14}", title: title, titleStyle: .prose, line: line, trailing: trailing,
              needsYou: false, tone: .quiet, tap: tap)))
      }
      return TurnWork(
        title: "Working",
        items: [
          subsession("docs", "Tidy the docs", "Tidy the docs, and say what you changed.", nil, .session(session: docs)),
          subsession(
            "crate", "Map the crate", "Map the crate's modules and what each owns.", "laptop",
            .session(session: crate)),
          subsession(
            "certs", "Rotate the relay certs", "Rotate the relay's certificates before Friday.", "homelab",
            .session(session: certs)),
          subsession(
            "lights", "Fix the lights", "This phone is not paired with attic", "attic",
            .detail(activity: "lights")),
        ], empty: nil)
    }
  }
#endif
