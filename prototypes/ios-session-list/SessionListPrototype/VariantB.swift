// PROTOTYPE — throwaway. Variant B, "Places": the phone opens on where work lives rather
// than on the work. Servers, then their Workspaces with a tally of what stands in each,
// then that Workspace's Sessions. A Session begins from inside a Workspace, so the way in
// has already chosen the Server and the Workspace. Pairing is a row at the foot of the root.

import SwiftUI

struct VariantB: View {
    @Environment(Store.self) private var store
    @State private var query = ""
    @State private var pairing = false

    var body: some View {
        NavigationStack {
            List {
                if !query.isEmpty {
                    ForEach(store.sessions(in: .everywhere, query: query)) { session in
                        NavigationLink(value: session) {
                            VStack(alignment: .leading, spacing: 2) {
                                Text(session.title).lineLimit(1)
                                Text(store.place(session.workspace)).font(.caption).foregroundStyle(.secondary)
                            }
                        }
                    }
                } else {
                    ForEach(store.remotes) { remote in
                        Section {
                            ForEach(store.workspaces(on: remote.id)) { workspace in
                                NavigationLink(value: workspace) { PlaceRow(workspace: workspace) }
                            }
                            if store.reach(remote.id) != .reachable {
                                UnreachableActions(remote: remote)
                            }
                        } header: {
                            HStack {
                                Label(remote.id, systemImage: remote.glyph)
                                Spacer()
                                if store.reach(remote.id) != .reachable {
                                    Label("unreachable", systemImage: "wifi.slash").foregroundStyle(.orange)
                                }
                            }
                        }
                    }
                    Section {
                        Button { pairing = true } label: {
                            Label("Pair another server", systemImage: "qrcode.viewfinder")
                        }
                    }
                }
            }
            .navigationTitle(store.showsOrigin ? "Servers" : store.remotes[0].id)
            .searchable(text: $query, prompt: "Search every session by title")
            .navigationDestination(for: Workspace.self) { WorkspaceSessions(workspace: $0) }
            .navigationDestination(for: SessionRow.self) { SessionStub(session: $0) }
            .sheet(isPresented: $pairing) { PairStub() }
        }
    }

    /// A Workspace and a tally of the Standings inside it, loudest first.
    private struct PlaceRow: View {
        @Environment(Store.self) private var store
        let workspace: Workspace

        var body: some View {
            let active = store.sessions(in: .workspace(workspace.id)).filter { !$0.settled }
            let dimmed = store.reach(workspace.remote) != .reachable
            HStack {
                Label(workspace.name, systemImage: workspace.glyph)
                Spacer()
                ForEach([Standing.needsIntervention, .working, .monitoring, .failed, .done], id: \.self) { standing in
                    let count = active.filter { $0.standing == standing }.count
                    if count > 0 {
                        Text("\(count)")
                            .font(.caption.weight(.bold)).monospacedDigit()
                            .padding(.horizontal, 7).padding(.vertical, 2)
                            .foregroundStyle(.white)
                            .background(standing.color, in: Capsule())
                    }
                }
                Text(active.isEmpty ? "–" : "\(active.count)")
                    .font(.subheadline).foregroundStyle(.secondary).monospacedDigit()
            }
            .opacity(dimmed ? 0.45 : 1)
        }
    }

    private struct WorkspaceSessions: View {
        @Environment(Store.self) private var store
        let workspace: Workspace
        @State private var showNew = false

        var body: some View {
            let rows = store.sessions(in: .workspace(workspace.id))
            let unreachable = store.reach(workspace.remote) != .reachable
            List {
                if unreachable {
                    Section { UnreachableActions(remote: store.remote(workspace.remote)) }
                }
                Section {
                    ForEach(rows.filter { !$0.settled }) { row($0) }
                }
                let settled = rows.filter(\.settled)
                if !settled.isEmpty {
                    Section("Settled") { ForEach(settled) { row($0) } }
                }
            }
            .navigationTitle(workspace.name)
            .toolbar {
                ToolbarItem(placement: .primaryAction) {
                    Button { showNew = true } label: { Image(systemName: "plus") }.disabled(unreachable)
                }
            }
            .sheet(isPresented: $showNew) {
                NewSessionStub(
                    workspace: workspace.id,
                    decided: "Server and workspace both: the reader was already standing in them. No picker at all.")
            }
        }

        private func row(_ session: SessionRow) -> some View {
            NavigationLink(value: session) {
                VStack(alignment: .leading, spacing: 3) {
                    HStack(alignment: .firstTextBaseline, spacing: 6) {
                        if let symbol = session.symbol {
                            Image(systemName: symbol).font(.subheadline).foregroundStyle(.secondary)
                        }
                        Text(session.title).lineLimit(2)
                            .foregroundStyle(session.settled ? .secondary : .primary)
                    }
                    HStack(spacing: 6) {
                        if let word = session.standing.word {
                            Text(session.standing == .working || session.standing == .monitoring
                                ? "\(word) \(compact(session.running))" : word)
                                .foregroundStyle(session.standing.color)
                        } else {
                            Text(compact(session.moved))
                        }
                        if let checkout = session.checkout, !session.settled {
                            Text("· \(checkout)").lineLimit(1)
                        }
                    }
                    .font(.caption)
                    .foregroundStyle(.secondary)
                }
            }
            .opacity(store.isUnreachable(session) ? 0.45 : 1)
            .swipeActions(edge: .trailing) {
                Button(session.settled ? "Unsettle" : "Settle") { store.toggleSettled(session) }.tint(.indigo)
            }
        }
    }
}
