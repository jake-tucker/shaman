// PROTOTYPE — throwaway. Variant A, "Inbox": the TUI's Sidebar turned into a phone screen.
// One flat list by recency under Everywhere, a scope menu in the title, the settled shelf
// beneath, an Unreachable server as a slim row at the foot of the active list, and one
// compose button that begins a Session wherever the Outlook stands.

import SwiftUI

struct VariantA: View {
    @Environment(Store.self) private var store
    @State private var scope: Scope = .everywhere
    @State private var query = ""
    @State private var shelfShown = 4
    @State private var showServers = false
    @State private var showNew = false
    @State private var tappedUnreachable: Remote?

    private var rows: [SessionRow] { store.sessions(in: scope, query: query) }

    var body: some View {
        NavigationStack {
            List {
                if !query.isEmpty {
                    ForEach(rows) { link($0) }
                } else {
                    Section {
                        ForEach(rows.filter { !$0.settled }) { link($0) }
                        ForEach(store.unreachable(in: scope)) { remote in
                            Button { tappedUnreachable = remote } label: {
                                HStack(spacing: 8) {
                                    Image(systemName: "wifi.slash")
                                    Text(remote.id)
                                    Text("unreachable")
                                    Spacer()
                                    Text("Try again").foregroundStyle(.tint)
                                }
                                .font(.footnote)
                                .foregroundStyle(.secondary)
                            }
                        }
                    }
                    let settled = rows.filter(\.settled)
                    if !settled.isEmpty {
                        Section("Settled") {
                            ForEach(settled.prefix(shelfShown)) { link($0) }
                            if settled.count > shelfShown {
                                Button("Show \(min(5, settled.count - shelfShown)) more") { shelfShown += 5 }
                                    .font(.footnote)
                            }
                        }
                    }
                }
            }
            .listStyle(.plain)
            .overlay {
                if rows.isEmpty {
                    ContentUnavailableView(
                        query.isEmpty ? "No sessions yet" : "No titles match",
                        systemImage: query.isEmpty ? "square.and.pencil" : "magnifyingglass",
                        description: Text(query.isEmpty ? "Begin one with the compose button." : ""))
                }
            }
            .navigationBarTitleDisplayMode(.inline)
            .searchable(text: $query, prompt: "Search by title")
            .toolbar {
                ToolbarItem(placement: .principal) { scopeMenu }
                ToolbarItem(placement: .bottomBar) {
                    Button { showServers = true } label: { Image(systemName: "server.rack") }
                }
                ToolbarItem(placement: .status) {
                    VStack(spacing: 1) {
                        Text("New sessions begin in").font(.caption2).foregroundStyle(.secondary)
                        Text(store.place(store.outlookWorkspace)).font(.caption.weight(.medium))
                    }
                }
                ToolbarItem(placement: .bottomBar) {
                    Button { showNew = true } label: { Image(systemName: "square.and.pencil") }
                }
            }
            .navigationDestination(for: SessionRow.self) { SessionStub(session: $0) }
            .sheet(isPresented: $showServers) { ServersSheet() }
            .sheet(isPresented: $showNew) {
                NewSessionStub(
                    workspace: store.outlookWorkspace,
                    decided: "Server and workspace: wherever the scope menu last pointed. Change either here.")
            }
            .sheet(item: $tappedUnreachable) { UnreachableSheet(remote: $0) }
            .onChange(of: scope) {
                switch scope {
                case .everywhere: break
                case .server(let id): store.turnOutlook(toward: id)
                case .workspace(let id): store.outlookWorkspace = id
                }
            }
        }
    }

    private var scopeMenu: some View {
        Menu {
            Picker("Scope", selection: $scope) {
                Label(store.title(of: .everywhere), systemImage: "globe").tag(Scope.everywhere)
            }
            .pickerStyle(.inline)
            .labelsHidden()
            ForEach(store.remotes) { remote in
                Picker(store.showsOrigin ? remote.id : "Workspaces", selection: $scope) {
                    if store.showsOrigin {
                        Label("All of \(remote.id)", systemImage: remote.glyph).tag(Scope.server(remote.id))
                    }
                    ForEach(store.workspaces(on: remote.id)) { workspace in
                        Label(workspace.name, systemImage: workspace.glyph).tag(Scope.workspace(workspace.id))
                    }
                }
                .pickerStyle(.inline)
            }
        } label: {
            HStack(spacing: 4) {
                Text(store.title(of: scope)).font(.headline)
                Image(systemName: "chevron.down").font(.caption2.weight(.bold))
            }
            .foregroundStyle(.primary)
        }
    }

    @ViewBuilder
    private func link(_ session: SessionRow) -> some View {
        NavigationLink(value: session) {
            if session.settled {
                HStack(spacing: 8) {
                    if let symbol = session.symbol {
                        Image(systemName: symbol).frame(width: 20)
                    }
                    Text(session.title).lineLimit(1)
                    Spacer()
                    Text(compact(session.moved)).font(.caption).monospacedDigit()
                }
                .font(.subheadline)
                .foregroundStyle(.secondary)
            } else {
                ActiveRow(session: session)
            }
        }
        .opacity(store.isUnreachable(session) ? 0.45 : 1)
        .swipeActions(edge: .trailing) {
            Button(session.settled ? "Unsettle" : "Settle") { store.toggleSettled(session) }.tint(.indigo)
        }
    }

    /// The Sidebar's three lines: place, Title, Checkout State, with the Rail and the right slot.
    private struct ActiveRow: View {
        @Environment(Store.self) private var store
        let session: SessionRow

        var body: some View {
            HStack(spacing: 10) {
                RoundedRectangle(cornerRadius: 2).fill(session.standing.color).frame(width: 4)
                VStack(alignment: .leading, spacing: 3) {
                    HStack {
                        Text(store.place(session.workspace)).font(.caption).foregroundStyle(.secondary)
                        Spacer()
                        rightSlot
                    }
                    HStack(alignment: .firstTextBaseline, spacing: 6) {
                        if let symbol = session.symbol {
                            Image(systemName: symbol).font(.subheadline).foregroundStyle(.secondary)
                        }
                        Text(session.title).font(.body.weight(.medium)).lineLimit(2)
                    }
                    if let checkout = session.checkout {
                        Label(checkout, systemImage: session.linked ? "arrow.triangle.branch" : "point.topleft.down.to.point.bottomright.curvepath")
                            .labelStyle(.titleAndIcon)
                            .font(.caption)
                            .foregroundStyle(.tertiary)
                            .lineLimit(1)
                    }
                }
            }
            .padding(.vertical, 2)
        }

        @ViewBuilder private var rightSlot: some View {
            switch session.standing {
            case .working, .monitoring:
                Text("\(session.standing.word!) \(compact(session.running))")
                    .font(.caption.weight(.medium)).foregroundStyle(session.standing.color).monospacedDigit()
            case .nothing:
                Text(compact(session.moved)).font(.caption).foregroundStyle(.secondary).monospacedDigit()
            default:
                Text(session.standing.word!).font(.caption.weight(.medium)).foregroundStyle(session.standing.color)
            }
        }
    }
}
