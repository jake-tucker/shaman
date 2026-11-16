// PROTOTYPE — throwaway. Canned data and the in-memory state every variant reads.
// Answers https://github.com/jake-tucker/shaman/issues/642; kept as the host of the open Session.

import SwiftUI

enum Standing: Int, CaseIterable, Hashable {
    case needsIntervention, working, failed, monitoring, done, nothing

    var word: String? {
        switch self {
        case .needsIntervention: "Needs intervention"
        case .working: "Working"
        case .failed: "Failed"
        case .monitoring: "Monitoring"
        case .done: "Done"
        case .nothing: nil
        }
    }

    var color: Color {
        switch self {
        case .needsIntervention: .orange
        case .working, .monitoring: .blue
        case .failed: .red
        case .done: .green
        case .nothing: .clear
        }
    }
}

enum Scenario: String, CaseIterable, Identifiable {
    case oneServer, threeServers, unreachable, loginNeeded, localNetwork, offline, empty
    var id: String { rawValue }

    var label: String {
        switch self {
        case .oneServer: "One server"
        case .threeServers: "Three servers"
        case .unreachable: "homelab unreachable"
        case .loginNeeded: "work-laptop needs a relay login"
        case .localNetwork: "homelab: local network refused"
        case .offline: "No network at all"
        case .empty: "One server, no sessions"
        }
    }
}

enum Reach: Equatable {
    case reachable
    case unreachable(Why)
    enum Why: Equatable { case plain, loginNeeded, localNetwork }
}

enum WayState { case answering, notAnswering, loginNeeded }

struct Way: Identifiable {
    let id = UUID()
    let relay: Bool
    let address: String
    let state: WayState
}

struct Remote: Identifiable, Hashable {
    let id: String  // the name its user gave it
    let symbol: String?  // stand-in for a chosen Icon; nil draws the plain monitor
    var glyph: String { symbol ?? "display" }
}

struct Workspace: Identifiable, Hashable {
    let id: String
    let name: String
    let symbol: String?
    let remote: String
    var glyph: String { symbol ?? "folder" }
}

struct SessionRow: Identifiable, Hashable {
    let id: String
    let title: String
    let symbol: String?
    var standing: Standing
    let workspace: String
    let checkout: String?
    let linked: Bool  // a linked Worktree rather than the main one
    let running: Int  // seconds Working or Monitoring
    let moved: Int  // seconds since it last moved
    var settled: Bool
    let hint: String?  // what the Intervention asks; the listing does not carry this today
}

enum Scope: Hashable {
    case everywhere
    case server(String)
    case workspace(String)
}

func compact(_ seconds: Int) -> String {
    switch seconds {
    case ..<60: "\(seconds)s"
    case ..<3600: "\(seconds / 60)m"
    case ..<86400: "\(seconds / 3600)h"
    case ..<(86400 * 7): "\(seconds / 86400)d"
    default: "\(seconds / (86400 * 7))w"
    }
}

@Observable
final class Store {
    private(set) var scenario: Scenario = .threeServers
    var sessions: [SessionRow] = Canned.sessions
    var healed: Set<String> = []
    var outlookWorkspace = "studio/shaman"
    var path = NavigationPath()
    /// The Sessions opened so far, each with its Transcript and toy engine.
    @ObservationIgnored private var lives: [String: Live] = [:]

    func apply(_ next: Scenario) {
        scenario = next
        sessions = Canned.sessions
        healed = []
        outlookWorkspace = "studio/shaman"
        lives = [:]
    }

    func live(_ id: String) -> Live {
        if let live = lives[id] { return live }
        let live = Canned.live(id)
        lives[id] = live
        return live
    }

    func row(_ id: String) -> SessionRow { sessions.first { $0.id == id }! }

    /// Opens straight into one of the canned states.
    func open(_ state: String) {
        guard let state = Canned.states.first(where: { $0.key == state }) else { return }
        apply(state.key == "unreachable" ? .unreachable : .threeServers)
        path = NavigationPath([state.session])
        // For checking the engine without a finger: `-decide YES` accepts or answers after a moment.
        guard UserDefaults.standard.bool(forKey: "decide") else { return }
        let live = live(state.session)
        Task { @MainActor in
            try? await Task.sleep(for: .seconds(2))
            guard let entry = live.pending.first else { return }
            if entry.isApprovalRow { live.decide(entry.id, .acceptOnce) } else { live.answer(entry.id, ["palette": ["Neutral"], "states": ["Hover", "Pressed"]]) }
        }
    }

    var remotes: [Remote] {
        switch scenario {
        case .oneServer, .empty: [Canned.remotes[0]]
        default: Canned.remotes
        }
    }

    /// A row names its Origin only where Sessions of more than one Server stand together.
    var showsOrigin: Bool { remotes.count > 1 }

    func reach(_ remote: String) -> Reach {
        if healed.contains(remote) { return .reachable }
        switch (scenario, remote) {
        case (.unreachable, "homelab"): return .unreachable(.plain)
        case (.loginNeeded, "work-laptop"): return .unreachable(.loginNeeded)
        case (.localNetwork, "homelab"): return .unreachable(.localNetwork)
        case (.offline, _): return .unreachable(.plain)
        default: return .reachable
        }
    }

    func ways(_ remote: String) -> [Way] {
        let reach = reach(remote)
        let direct: WayState = reach == .reachable ? .answering : .notAnswering
        let relay: WayState =
            switch reach {
            case .reachable: .answering
            case .unreachable(.loginNeeded): .loginNeeded
            default: .notAnswering
            }
        switch remote {
        case "studio":
            return [
                Way(relay: false, address: "192.168.1.20:7441", state: direct),
                Way(relay: true, address: "relay.shaman.dev", state: relay),
            ]
        case "work-laptop":
            return [
                Way(relay: false, address: "100.84.12.7:7441", state: direct),
                Way(relay: true, address: "relay.shaman.dev", state: relay),
            ]
        default:
            return [Way(relay: false, address: "192.168.1.40:7441", state: direct)]
        }
    }

    func remote(_ id: String) -> Remote { Canned.remotes.first { $0.id == id }! }
    func workspace(_ id: String) -> Workspace { Canned.workspaces.first { $0.id == id }! }
    func workspaces(on remote: String) -> [Workspace] { Canned.workspaces.filter { $0.remote == remote } }
    func origin(_ session: SessionRow) -> Remote { remote(workspace(session.workspace).remote) }
    func isUnreachable(_ session: SessionRow) -> Bool { reach(origin(session).id) != .reachable }

    func unreachable(in scope: Scope) -> [Remote] {
        remotes.filter { remote in
            guard reach(remote.id) != .reachable else { return false }
            switch scope {
            case .everywhere: return true
            case .server(let id): return id == remote.id
            case .workspace(let id): return workspace(id).remote == remote.id
            }
        }
    }

    /// The one flat order the shelves always draw: by recency.
    func sessions(in scope: Scope, query: String = "") -> [SessionRow] {
        guard scenario != .empty else { return [] }
        let names = Set(remotes.map(\.id))
        return
            sessions
            .filter { names.contains(workspace($0.workspace).remote) }
            .filter { session in
                switch scope {
                case .everywhere: true
                case .server(let id): workspace(session.workspace).remote == id
                case .workspace(let id): session.workspace == id
                }
            }
            .filter { query.isEmpty || $0.title.localizedCaseInsensitiveContains(query) }
            .map { row in
                // A Session opened here stands as its Transcript now says.
                guard let live = lives[row.id] else { return row }
                var row = row
                row.standing = live.standing
                return row
            }
            .sorted { $0.moved < $1.moved }
    }

    func toggleSettled(_ session: SessionRow) {
        guard let index = sessions.firstIndex(where: { $0.id == session.id }) else { return }
        sessions[index].settled.toggle()
    }

    func place(_ workspaceID: String) -> String {
        let workspace = workspace(workspaceID)
        return showsOrigin ? "\(workspace.remote) · \(workspace.name)" : workspace.name
    }

    func title(of scope: Scope) -> String {
        switch scope {
        case .everywhere: showsOrigin ? "Everywhere" : "All workspaces"
        case .server(let id): id
        case .workspace(let id): place(id)
        }
    }

    func turnOutlook(toward remote: String) {
        guard workspace(outlookWorkspace).remote != remote else { return }
        outlookWorkspace = workspaces(on: remote)[0].id
    }
}

enum Canned {
    static let remotes = [
        Remote(id: "studio", symbol: "macstudio"),
        Remote(id: "work-laptop", symbol: "laptopcomputer"),
        Remote(id: "homelab", symbol: nil),
    ]

    static let workspaces = [
        Workspace(id: "studio/shaman", name: "shaman", symbol: "flame", remote: "studio"),
        Workspace(id: "studio/dotfiles", name: "dotfiles", symbol: "gearshape.2", remote: "studio"),
        Workspace(id: "studio/sidekick", name: "Sidekick", symbol: "figure.wave", remote: "studio"),
        Workspace(id: "work/billing-api", name: "billing-api", symbol: "creditcard", remote: "work-laptop"),
        Workspace(id: "work/shaman", name: "shaman", symbol: "flame", remote: "work-laptop"),
        Workspace(id: "work/design-system", name: "design-system", symbol: nil, remote: "work-laptop"),
        Workspace(id: "lab/relay-infra", name: "relay-infra", symbol: "antenna.radiowaves.left.and.right", remote: "homelab"),
        Workspace(id: "lab/home-assistant", name: "home-assistant", symbol: "house", remote: "homelab"),
    ]

    private static func row(
        _ id: Int, _ title: String, _ symbol: String?, _ standing: Standing, _ workspace: String,
        _ checkout: String?, linked: Bool = false, running: Int = 0, moved: Int, settled: Bool = false,
        hint: String? = nil
    ) -> SessionRow {
        SessionRow(
            id: "s\(id)", title: title, symbol: symbol, standing: standing, workspace: workspace,
            checkout: checkout, linked: linked, running: running, moved: moved, settled: settled, hint: hint)
    }

    static let sessions: [SessionRow] = [
        row(1, "Relax the exact-version rule for peers", "arrow.left.arrow.right", .needsIntervention, "studio/shaman",
            "feature/peer-version-floor", linked: true, moved: 40,
            hint: "Approval: cargo nextest run -p shaman-relay"),
        row(2, "Draw the invite as a QR code in /serve", "qrcode", .working, "studio/shaman",
            "feature/serve-qr", linked: true, running: 252, moved: 5),
        row(3, "Why the nightly backup skipped Tuesday", "externaldrive.badge.questionmark", .monitoring,
            "lab/relay-infra", "main", running: 2280, moved: 2280),
        row(4, "Port the invoice rounding fix to v2", "dollarsign.circle", .failed, "work/billing-api",
            "fix/rounding-v2", linked: true, moved: 1320),
        row(5, "Tidy zsh startup time", "terminal", .done, "studio/dotfiles", "main", moved: 3700),
        row(6, "Token colours for the button palette", "paintpalette", .needsIntervention, "work/design-system",
            "tokens/buttons", linked: true, moved: 610, hint: "Question: which palette should buttons use?"),
        row(7, "Stop /serve offering link-local addresses", "network", .nothing, "studio/shaman",
            "fix/serve-link-local", linked: true, moved: 3 * 3600),
        row(8, "Find the workspace that holds the relay deploy scripts", nil, .nothing, "studio/sidekick", nil,
            moved: 5 * 3600),
        row(9, "Upgrade Home Assistant to 2026.11", "house", .needsIntervention, "lab/home-assistant", "main",
            moved: 12, hint: "Approval: configuration.yaml"),
        row(10, "Sketch the thin client glossary entry", "text.book.closed", .nothing, "work/shaman", "main",
            moved: 26 * 3600),
        row(11, "Drop the /compact hint from the cache warning", "exclamationmark.triangle", .nothing,
            "studio/shaman", "main", moved: 2 * 86400, settled: true),
        row(12, "Let a peer withdraw from a pairing", "person.2.slash", .nothing, "studio/shaman", "main",
            moved: 3 * 86400, settled: true),
        row(13, "Rotate the relay's TLS certificate", "lock.rotation", .nothing, "lab/relay-infra", "main",
            moved: 4 * 86400, settled: true),
        row(14, "Explain the March invoice discrepancy", nil, .nothing, "work/billing-api", "main",
            moved: 5 * 86400, settled: true),
        row(15, "Move the session picker to semantic command ids", "command", .nothing, "studio/shaman", "main",
            moved: 6 * 86400, settled: true),
        row(16, "Add a dark variant of the focus ring token", "circle.lefthalf.filled", .nothing,
            "work/design-system", "main", moved: 8 * 86400, settled: true),
        row(17, "Pin the toolchain to 1.93", "pin", .nothing, "studio/shaman", "main", moved: 9 * 86400,
            settled: true),
        row(18, "Symlink the new ghostty config", "link", .nothing, "studio/dotfiles", "main",
            moved: 12 * 86400, settled: true),
        row(19, "Why the Zigbee dongle drops overnight", "wifi.exclamationmark", .nothing, "lab/home-assistant",
            "main", moved: 15 * 86400, settled: true),
        row(20, "Write ADR-0047 on relay compatibility", "doc.text", .nothing, "studio/shaman", "main",
            moved: 21 * 86400, settled: true),
    ]
}
