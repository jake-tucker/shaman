#if DEBUG
  import ShamanCore

  /// One Workspace the fake's Remotes list, from the new-session prototype's picker, and how the fake makes the
  /// Workspace Picker and the Landing's Worktree choice of them as the core would.
  struct FakeWorkspace: Sendable {
    /// What a Session begun in it may begin on.
    enum Source: Sendable {
      /// A Repository whose main Worktree stands on the Checkout State named, in which a new one can be made.
      case repository(String)
      /// Outside source control.
      case directory
      /// A bare Repository, which has no main Worktree.
      case bare
    }

    let remote: String
    let name: String
    let glyph: String?
    let description: String?
    let source: Source
    /// Why its Server cannot read it, where it cannot.
    var unavailable: String? = nil

    /// The Workspace a picker row carries, as the core is handed it back.
    var reference: String { Self.reference(remote, name) }

    static func reference(_ remote: String, _ name: String) -> String {
      "workspace/\(remote)/\(name)"
    }

    /// The Remote and the Workspace a picker row's `reference` names.
    static func named(by reference: String) -> (remote: String, name: String)? {
      let parts = reference.split(separator: "/", maxSplits: 2).map(String.init)
      guard parts.count == 3, parts[0] == "workspace" else { return nil }
      return (parts[1], parts[2])
    }

    static func find(_ remote: String, _ name: String) -> FakeWorkspace? {
      canned.first { $0.remote == remote && $0.name == name }
    }

    static let canned: [FakeWorkspace] = [
      FakeWorkspace(
        remote: "workstation", name: "shaman", glyph: "\u{f0238}",
        description: "A terminal client for coding agents across Codex, Copilot and Claude.",
        source: .repository("main")),
      FakeWorkspace(
        remote: "workstation", name: "dotfiles", glyph: "\u{f018d}",
        description: "Shell, editor and terminal configuration.", source: .repository("main")),
      FakeWorkspace(
        remote: "workstation", name: "notes", glyph: "\u{f0219}", description: "Loose notes and drafts.",
        source: .directory),
      FakeWorkspace(
        remote: "workstation", name: "photo-archive", glyph: nil, description: "Every photo since 2009.",
        source: .directory, unavailable: "Unavailable: its directory is gone"),
      FakeWorkspace(
        remote: "workstation", name: "mirror.git", glyph: nil, description: "A bare mirror of the release repository.",
        source: .bare),
      FakeWorkspace(
        remote: "laptop", name: "billing-api", glyph: "\u{f0070}",
        description: "Invoices, rounding and the ledger export.", source: .repository("main")),
      FakeWorkspace(
        remote: "laptop", name: "design-system", glyph: "\u{f03d8}", description: nil, source: .repository("trunk")),
      FakeWorkspace(
        remote: "laptop", name: "shaman", glyph: "\u{f0238}",
        description: "A terminal client for coding agents across Codex, Copilot and Claude.",
        source: .repository("detached 1a2b3c4d")),
      FakeWorkspace(
        remote: "homelab", name: "home-assistant", glyph: "\u{f07d0}",
        description: "The house's automations and dashboards.", source: .repository("main")),
      FakeWorkspace(
        remote: "homelab", name: "relay-infra", glyph: "\u{f01bc}", description: "Deploy scripts for the relay.",
        source: .repository("main")),
    ]

    /// What the Landing says where its Workspace is a bare Repository.
    static let workingCopy =
      "This workspace is a bare repository. A session needs a working copy of it to begin in, which this phone "
      + "cannot choose yet."

    /// The Worktree choice the Landing offers in this Workspace, where it offers one.
    var worktree: WorktreeChoice? {
      guard case .repository(let main) = source else { return nil }
      return WorktreeChoice(main: "On \(main)", new: "In a new worktree")
    }

    /// The Workspace Picker over the Remotes `remotes`, of which those `unanswered` have yet to list their Workspaces
    /// and those `unreachable` read Unreachable, the Outlook standing in `chosen`, searched for `query`.
    static func picker(
      on remotes: [String], unanswered: [String], unreachable: Set<String>, chosen: (remote: String, name: String)?,
      query: String
    ) -> WorkspacePicker {
      let originNamed = remotes.count > 1
      let query = query.trimmingCharacters(in: .whitespaces)
      let sections = remotes.compactMap { remote -> WorkspaceSection? in
        let rows =
          unanswered.contains(remote)
          ? []
          : canned.filter { $0.remote == remote && (query.isEmpty || $0.name.localizedCaseInsensitiveContains(query)) }
            .map {
              WorkspaceRow(
                workspace: $0.reference, glyph: $0.glyph, name: $0.name, description: $0.description,
                chosen: chosen?.remote == remote && chosen?.name == $0.name, unavailable: $0.unavailable)
            }
        if !query.isEmpty && rows.isEmpty {
          return nil
        }
        return WorkspaceSection(
          header: originNamed ? remote : nil,
          unreachable: unreachable.contains(remote) ? (originNamed ? "Unreachable" : "The server is unreachable") : nil,
          rows: rows,
          line: rows.isEmpty ? (unanswered.contains(remote) ? "Loading workspaces…" : "No workspaces") : nil)
      }
      return WorkspacePicker(
        title: "Begin in", searchPrompt: "Search workspaces", sections: sections,
        empty: !query.isEmpty && sections.isEmpty ? "No workspaces found" : nil)
    }
  }
#endif
