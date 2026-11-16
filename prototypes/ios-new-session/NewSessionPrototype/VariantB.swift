// PROTOTYPE — throwaway. Variant B, "Screen": the compose button pushes a screen in the shape
// an open Session has, empty: the glossary's Landing kept as a view of its own. The middle
// names where the Session will begin, the composer at the foot is the open Session's own, and
// the Workspace picker is a half sheet that asks for a Server and then a Workspace. Sending
// changes nothing but the contents: the same screen becomes the Provisional Session and then
// the Session, and back still leads to the listing.

import SwiftUI

struct DraftRoute: Hashable {}

struct VariantB: View {
    @Environment(Store.self) private var store
    /// What was chosen: this screen, with A's one searchable list pushed over it as the picker.
    var chosen = false
    @State private var begun: String?
    @State private var picking = false
    @FocusState private var focused: Bool

    var body: some View {
        Group {
            if let id = begun, let live = store.begun(id) {
                OpenSession(live: live, session: store.row(id))
            } else {
                landing
            }
        }
        .onChange(of: store.sendAsk) { send() }
        .onChange(of: store.pickerAsk) { picking = true }
        .onDisappear {
            // Leaving a refused Provisional Session discards it and keeps its text as the draft.
            if let id = begun, case .refused = store.begun(id)?.phase { store.discard(id) }
        }
    }

    private func send() {
        guard store.canBegin, begun == nil else { return }
        begun = store.begin()
    }

    private var landing: some View {
        @Bindable var store = store
        return ScrollView {
            VStack(spacing: 12) {
                Image(systemName: store.outlook.glyph).font(.system(size: 32)).foregroundStyle(.secondary)
                Text(store.outlook.name).font(.title2.weight(.semibold))
                if store.showsOrigin {
                    Label(store.outlook.remote, systemImage: store.remote(store.outlook.remote).glyph)
                        .font(.subheadline)
                        .foregroundStyle(.secondary)
                }
                if let about = store.outlook.about {
                    Text(about).font(.footnote).foregroundStyle(.secondary).multilineTextAlignment(.center)
                }
                Button("Begin somewhere else") { picking = true }
                    .buttonStyle(.bordered)
                    .controlSize(.small)
                    .padding(.top, 4)
                if store.outlook.repo {
                    Picker("Worktree", selection: $store.newWorktree) {
                        Text("On main").tag(false)
                        Text("In a new worktree").tag(true)
                    }
                    .pickerStyle(.segmented)
                    .padding(.top, 14)
                }
                Label(store.agent, systemImage: "sparkle").font(.caption).foregroundStyle(.secondary).padding(.top, 4)
            }
            .padding(.horizontal, 32)
            .padding(.top, 20)
        }
        .scrollDismissesKeyboard(.interactively)
        .safeAreaInset(edge: .bottom, spacing: 0) { composer }
        .navigationBarTitleDisplayMode(.inline)
        .toolbar {
            ToolbarItem(placement: .principal) {
                Button { picking = true } label: {
                    VStack(spacing: 1) {
                        Text("New session").font(.subheadline.weight(.semibold))
                        HStack(spacing: 3) {
                            Text(store.place(store.outlookWorkspace))
                            Image(systemName: "chevron.down").font(.system(size: 8, weight: .bold))
                        }
                        .font(.caption2)
                        .foregroundStyle(.secondary)
                    }
                }
                .buttonStyle(.plain)
            }
        }
        .sheet(isPresented: Binding(get: { picking && !chosen }, set: { picking = $0 })) { PlaceSheet() }
        .navigationDestination(isPresented: Binding(get: { picking && chosen }, set: { picking = $0 })) {
            PlaceList().toolbar(.hidden, for: .bottomBar)
        }
        .onAppear { focused = true }
    }

    /// The open Session's composer, word for word: a field at the foot with a send button.
    private var composer: some View {
        @Bindable var store = store
        return VStack(spacing: 8) {
            if store.outlookUnreachable {
                Label("\(store.outlook.remote) is unreachable. Nothing begins there until it answers.", systemImage: "wifi.slash")
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .frame(maxWidth: .infinity, alignment: .leading)
            }
            HStack(alignment: .bottom, spacing: 8) {
                TextField("What should it do?", text: $store.draft, axis: .vertical)
                    .lineLimit(1...6)
                    .focused($focused)
                    .padding(.horizontal, 14)
                    .padding(.vertical, 9)
                    .background(Color(.secondarySystemBackground), in: RoundedRectangle(cornerRadius: 20))
                Button { send() } label: { Image(systemName: "arrow.up.circle.fill").font(.system(size: 34)) }
                    .disabled(!store.canBegin)
            }
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 8)
        .background(.bar)
    }
}

/// B's Workspace picker: a half sheet that asks which Server, then which of its Workspaces.
private struct PlaceSheet: View {
    @Environment(Store.self) private var store
    @Environment(\.dismiss) private var dismiss
    @State private var remote = ""

    var body: some View {
        NavigationStack {
            List {
                if store.reach(remote) != .reachable {
                    Label("\(remote) is unreachable. A session begun here waits until it answers.", systemImage: "wifi.slash")
                        .font(.footnote)
                        .foregroundStyle(.secondary)
                }
                ForEach(store.choices(on: remote)) { workspace in
                    Button {
                        store.choose(workspace)
                        dismiss()
                    } label: {
                        PlaceRow(workspace: workspace, chosen: workspace.id == store.outlookWorkspace)
                    }
                    .buttonStyle(.plain)
                    .disabled(workspace.unavailable != nil)
                }
            }
            .listStyle(.plain)
            .safeAreaInset(edge: .top, spacing: 0) {
                if store.showsOrigin {
                    Picker("Server", selection: $remote) {
                        ForEach(store.remotes) { Text($0.id).tag($0.id) }
                    }
                    .pickerStyle(.segmented)
                    .padding(.horizontal, 16)
                    .padding(.vertical, 8)
                    .background(.bar)
                }
            }
            .navigationTitle("Begin in")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { Button("Cancel") { dismiss() } }
            }
        }
        .presentationDetents([.medium, .large])
        .presentationBackground(Color(.systemBackground))
        .onAppear { remote = store.outlook.remote }
    }
}
