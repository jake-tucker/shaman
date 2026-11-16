// PROTOTYPE — throwaway. Variant C, "In the listing": the compose button opens nothing. The
// listing's bottom bar becomes a composer, with where the Session begins as chips above the
// field and the Workspace picker as a menu. Sending leaves the reader in the listing: the
// Session appears as a row at the head of the list, starting, and is opened like any other.
// So the listing with its composer up is the Landing, and several Sessions can be begun in a
// row without reading any of them.

import SwiftUI

struct VariantC: View {
    @Environment(Store.self) private var store
    @Binding var composing: Bool
    @FocusState private var focused: Bool

    var body: some View {
        @Bindable var store = store
        VStack(spacing: 8) {
            HStack(spacing: 6) {
                Menu {
                    ForEach(store.remotes) { remote in
                        Section(store.showsOrigin ? remote.id : "Workspaces") {
                            ForEach(store.choices(on: remote.id)) { workspace in
                                Button { store.choose(workspace) } label: {
                                    Label(
                                        workspace.unavailable == nil ? workspace.name : "\(workspace.name) (unavailable)",
                                        systemImage: workspace.glyph)
                                }
                                .disabled(workspace.unavailable != nil)
                            }
                        }
                    }
                } label: {
                    chip(on: false) {
                        Image(systemName: store.outlook.glyph)
                        Text(store.place(store.outlookWorkspace)).lineLimit(1)
                        Image(systemName: "chevron.down").font(.system(size: 9, weight: .bold))
                    }
                }
                if store.outlook.repo {
                    Button { store.newWorktree.toggle() } label: {
                        chip(on: store.newWorktree) {
                            Image(systemName: "arrow.triangle.branch")
                            Text(store.newWorktree ? "New worktree" : "main")
                        }
                    }
                }
                Spacer(minLength: 0)
                Button {
                    focused = false
                    composing = false
                } label: {
                    Image(systemName: "xmark.circle.fill").font(.title3).foregroundStyle(.tertiary)
                }
            }
            .buttonStyle(.plain)
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
        .onAppear { focused = true }
        .onChange(of: store.sendAsk) { send() }
    }

    private func send() {
        guard store.canBegin else { return }
        store.begin()
        focused = false
        composing = false
    }

    private func chip(on: Bool, @ViewBuilder content: () -> some View) -> some View {
        HStack(spacing: 4) { content() }
            .font(.footnote.weight(.medium))
            .padding(.horizontal, 10)
            .padding(.vertical, 6)
            .foregroundStyle(on ? Color.white : Color.primary)
            .background(on ? Color.accentColor : Color(.secondarySystemBackground), in: Capsule())
    }
}
