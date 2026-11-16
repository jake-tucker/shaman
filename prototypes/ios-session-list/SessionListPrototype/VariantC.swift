// PROTOTYPE — throwaway. Variant C, "Attention": the list is ordered by what a Session
// owes its reader, not by recency or place. Sections by Standing, loudest first, with the
// Intervention's own words on the row; Server chips for scope; an Unreachable server as a
// banner; servers and pairing on a tab of their own; and the Landing's composer standing
// at the foot of the list, so beginning a Session is typing, not navigating.

import SwiftUI

struct VariantC: View {
    var body: some View {
        TabView {
            Tab("Sessions", systemImage: "bubble.left.and.text.bubble.right") { Sessions() }
            Tab("Servers", systemImage: "server.rack") { NavigationStack { ServersList() } }
        }
    }

    private struct Sessions: View {
        @Environment(Store.self) private var store
        @State private var scope: Scope = .everywhere
        @State private var query = ""
        @State private var draft = ""
        @State private var begun: String?
        @State private var settledOpen = false

        private static let groups: [(String, [Standing])] = [
            ("Needs you", [.needsIntervention]),
            ("Working", [.working]),
            ("Monitoring", [.monitoring]),
            ("Finished, not yet read", [.failed, .done]),
            ("Idle", [.nothing]),
        ]

        var body: some View {
            let rows = store.sessions(in: scope, query: query)
            NavigationStack {
                List {
                    if store.showsOrigin {
                        chips.listRowInsets(EdgeInsets()).listRowSeparator(.hidden)
                    }
                    ForEach(store.unreachable(in: scope)) { remote in
                        VStack(alignment: .leading, spacing: 6) {
                            Label("\(remote.id) is unreachable", systemImage: "wifi.slash")
                                .font(.subheadline.weight(.semibold))
                            UnreachableActions(remote: remote)
                        }
                        .listRowBackground(Color.orange.opacity(0.12))
                    }
                    ForEach(Self.groups, id: \.0) { title, standings in
                        let group = rows.filter { !$0.settled && standings.contains($0.standing) }
                        if !group.isEmpty {
                            Section(title) { ForEach(group) { row($0) } }
                        }
                    }
                    let settled = rows.filter(\.settled)
                    if !settled.isEmpty {
                        Section {
                            DisclosureGroup("Settled (\(settled.count))", isExpanded: $settledOpen) {
                                ForEach(settled) { row($0) }
                            }
                            .font(.subheadline)
                            .foregroundStyle(.secondary)
                        }
                    }
                }
                .listStyle(.plain)
                .navigationTitle("Sessions")
                .navigationBarTitleDisplayMode(.inline)
                .searchable(text: $query, placement: .navigationBarDrawer, prompt: "Search by title")
                .navigationDestination(for: SessionRow.self) { SessionStub(session: $0) }
                .safeAreaInset(edge: .bottom) { composer }
                .sheet(item: $begun) { prompt in
                    NewSessionStub(
                        workspace: store.outlookWorkspace, prompt: prompt,
                        decided: "Everything: the prompt is already written, and it begins where the line above the composer says.")
                }
            }
        }

        private var chips: some View {
            ScrollView(.horizontal, showsIndicators: false) {
                HStack(spacing: 8) {
                    chip("Everywhere", "globe", .everywhere)
                    ForEach(store.remotes) { remote in
                        chip(
                            remote.id, store.reach(remote.id) == .reachable ? remote.glyph : "wifi.slash",
                            .server(remote.id))
                    }
                }
                .padding(.horizontal, 16)
                .padding(.vertical, 8)
            }
        }

        private func chip(_ title: String, _ symbol: String, _ value: Scope) -> some View {
            Button {
                scope = value
                if case .server(let id) = value { store.turnOutlook(toward: id) }
            } label: {
                Label(title, systemImage: symbol)
                    .font(.footnote.weight(.medium))
                    .padding(.horizontal, 12).padding(.vertical, 6)
                    .foregroundStyle(scope == value ? Color.white : Color.primary)
                    .background(scope == value ? Color.accentColor : Color(.secondarySystemFill), in: Capsule())
            }
            .buttonStyle(.plain)
        }

        private func row(_ session: SessionRow) -> some View {
            NavigationLink(value: session) {
                HStack(alignment: .top, spacing: 10) {
                    Group {
                        switch session.standing {
                        case .needsIntervention: Image(systemName: "exclamationmark.bubble.fill")
                        case .working: ProgressView().controlSize(.small)
                        case .monitoring: Image(systemName: "eye")
                        case .failed: Image(systemName: "xmark.circle.fill")
                        case .done: Image(systemName: "checkmark.circle.fill")
                        case .nothing: Image(systemName: session.symbol ?? "circle.dotted")
                        }
                    }
                    .foregroundStyle(session.standing == .nothing ? Color.secondary : session.standing.color)
                    .frame(width: 22)
                    VStack(alignment: .leading, spacing: 3) {
                        Text(session.title).lineLimit(2)
                            .foregroundStyle(session.settled ? .secondary : .primary)
                        if let hint = session.hint {
                            Text(hint).font(.footnote).foregroundStyle(.orange).lineLimit(2)
                        }
                        Text(caption(session)).font(.caption).foregroundStyle(.secondary).monospacedDigit()
                    }
                }
            }
            .opacity(store.isUnreachable(session) ? 0.45 : 1)
            .swipeActions(edge: .trailing) {
                Button(session.settled ? "Unsettle" : "Settle") { store.toggleSettled(session) }.tint(.indigo)
            }
        }

        private func caption(_ session: SessionRow) -> String {
            let time =
                switch session.standing {
                case .working, .monitoring: "for \(compact(session.running))"
                default: "\(compact(session.moved)) ago"
                }
            return "\(store.place(session.workspace)) · \(time)"
        }

        private var composer: some View {
            VStack(alignment: .leading, spacing: 6) {
                Menu {
                    ForEach(store.remotes) { remote in
                        Section(remote.id) {
                            ForEach(store.workspaces(on: remote.id)) { workspace in
                                Button(workspace.name, systemImage: workspace.glyph) {
                                    store.outlookWorkspace = workspace.id
                                }
                            }
                        }
                    }
                } label: {
                    HStack(spacing: 4) {
                        Image(systemName: store.workspace(store.outlookWorkspace).glyph)
                        Text(store.place(store.outlookWorkspace))
                        Image(systemName: "chevron.up.chevron.down").font(.caption2)
                    }
                    .font(.caption.weight(.medium))
                    .foregroundStyle(.secondary)
                }
                HStack(alignment: .bottom) {
                    TextField("Begin a session…", text: $draft, axis: .vertical)
                        .lineLimit(1...4)
                        .padding(.horizontal, 12).padding(.vertical, 8)
                        .background(Color(.secondarySystemFill), in: RoundedRectangle(cornerRadius: 18))
                    Button {
                        begun = draft
                        draft = ""
                    } label: {
                        Image(systemName: "arrow.up.circle.fill").font(.system(size: 30))
                    }
                    .disabled(draft.trimmingCharacters(in: .whitespaces).isEmpty)
                }
            }
            .padding(.horizontal, 14)
            .padding(.vertical, 8)
            .background(.bar)
        }
    }
}

extension String: @retroactive Identifiable {
    public var id: String { self }
}
