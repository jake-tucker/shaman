// PROTOTYPE — throwaway. Variant A, "Sheet": the compose button presents a sheet, as a mail
// app's does. Where it begins stands as header fields above a page to write on, and the
// Workspace picker is pushed inside the sheet. Sending keeps the sheet up until the Server
// answers: the reader never sees a Provisional Session, only a Session that already exists,
// and a refusal is read where the Prompt can still be changed.

import SwiftUI

struct VariantA: View {
    @Environment(Store.self) private var store
    @Environment(\.dismiss) private var dismiss
    @State private var awaited: String?  // the Session begun and not yet answered for
    @State private var sent = ""  // the Prompt, kept on the page while its Server is awaited
    @State private var refusal: String?
    @State private var picking = false
    @FocusState private var focused: Bool

    private enum Wait: Equatable { case idle, phase(Live.Phase), arrived }
    private var wait: Wait {
        guard let id = awaited, let live = store.begun(id) else { return .idle }
        return live.phase.map(Wait.phase) ?? .arrived
    }

    var body: some View {
        @Bindable var store = store
        NavigationStack {
            VStack(spacing: 0) {
                VStack(spacing: 0) {
                    Button { picking = true } label: {
                        field("In") {
                            Label(store.place(store.outlookWorkspace), systemImage: store.outlook.glyph)
                            Spacer()
                            Image(systemName: "chevron.right").font(.caption.weight(.semibold)).foregroundStyle(.tertiary)
                        }
                        .contentShape(Rectangle())
                    }
                    .buttonStyle(.plain)
                    Divider().padding(.leading, 16)
                    if store.outlook.repo {
                        Toggle(isOn: $store.newWorktree) {
                            field("On", padded: false) {
                                Label(store.newWorktree ? "a new worktree" : "main", systemImage: "arrow.triangle.branch")
                                    .lineLimit(1)
                                if !store.newWorktree {
                                    Spacer()
                                    Text("New worktree").font(.caption).foregroundStyle(.secondary)
                                }
                            }
                        }
                        .padding(.horizontal, 16)
                        .padding(.vertical, 6)
                        Divider().padding(.leading, 16)
                    }
                    field("Agent") { Text(store.agent).foregroundStyle(.secondary) }
                }
                .disabled(awaited != nil)
                Divider()
                if awaited != nil {
                    Text(sent)
                        .foregroundStyle(.secondary)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .padding(16)
                } else {
                    TextField("What should it do?", text: $store.draft, axis: .vertical)
                        .lineLimit(5...)
                        .focused($focused)
                        .padding(16)
                }
                Spacer(minLength: 0)
                status
            }
            .navigationTitle("New session")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { Button("Cancel") { dismiss() }.disabled(awaited != nil) }
                ToolbarItem(placement: .confirmationAction) {
                    if awaited != nil {
                        ProgressView()
                    } else {
                        Button { send() } label: { Image(systemName: "arrow.up") }
                            .buttonStyle(.borderedProminent)
                            .disabled(!store.canBegin)
                    }
                }
            }
            .navigationDestination(isPresented: $picking) { PlaceList() }
        }
        .interactiveDismissDisabled(awaited != nil)
        .onAppear { focused = true }
        .onChange(of: store.sendAsk) { send() }
        .onChange(of: store.pickerAsk) { picking = true }
        .onChange(of: wait) { _, now in
            guard let id = awaited else { return }
            switch now {
            case .arrived:
                awaited = nil
                dismiss()
                store.path.append(id)
            case .phase(.refused(let why)):
                refusal = why
                store.discard(id)  // the Prompt goes back onto the page
                awaited = nil
                focused = true
            default: break
            }
        }
    }

    private func send() {
        guard store.canBegin, awaited == nil else { return }
        refusal = nil
        sent = store.draft
        awaited = store.begin()
    }

    private func field(_ name: String, padded: Bool = true, @ViewBuilder value: () -> some View) -> some View {
        HStack(spacing: 8) {
            Text(name).foregroundStyle(.secondary).frame(width: 52, alignment: .leading)
            value()
            if !padded { Spacer(minLength: 0) }
        }
        .font(.callout)
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(.horizontal, padded ? 16 : 0)
        .padding(.vertical, padded ? 12 : 0)
    }

    @ViewBuilder private var status: some View {
        Group {
            if awaited != nil {
                HStack(spacing: 8) {
                    ProgressView().controlSize(.small)
                    Text(wait == .phase(.creatingWorktree) ? "Creating worktree" : "Starting")
                }
                .foregroundStyle(.secondary)
            } else if let refusal {
                Label("Could not create session: \(refusal)", systemImage: "exclamationmark.triangle.fill")
                    .foregroundStyle(.red)
            } else if store.outlookUnreachable {
                Label("\(store.outlook.remote) is unreachable. Nothing begins there until it answers.", systemImage: "wifi.slash")
                    .foregroundStyle(.secondary)
            }
        }
        .font(.footnote)
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(.horizontal, 16)
        .padding(.vertical, 10)
    }
}

/// A's Workspace picker: every Server's Workspaces as one searchable list, pushed in the sheet.
private struct PlaceList: View {
    @Environment(Store.self) private var store
    @Environment(\.dismiss) private var dismiss
    @State private var query = ""

    var body: some View {
        List {
            ForEach(store.remotes) { remote in
                let found = store.choices(on: remote.id).filter {
                    query.isEmpty || $0.name.localizedCaseInsensitiveContains(query)
                }
                if !found.isEmpty {
                    Section {
                        ForEach(found) { workspace in
                            Button {
                                store.choose(workspace)
                                dismiss()
                            } label: {
                                PlaceRow(workspace: workspace, chosen: workspace.id == store.outlookWorkspace)
                            }
                            .buttonStyle(.plain)
                            .disabled(workspace.unavailable != nil)
                        }
                    } header: {
                        if store.showsOrigin {
                            HStack {
                                Label(remote.id, systemImage: remote.glyph)
                                Spacer()
                                if store.reach(remote.id) != .reachable { Text("unreachable") }
                            }
                        }
                    }
                }
            }
        }
        .listStyle(.insetGrouped)
        .searchable(text: $query, placement: .navigationBarDrawer(displayMode: .always), prompt: "Search workspaces")
        .navigationTitle("Begin in")
        .navigationBarTitleDisplayMode(.inline)
    }
}

/// One Workspace as a picker offers it: its Icon, its name, and its Description or why it
/// cannot be chosen.
struct PlaceRow: View {
    let workspace: Workspace
    let chosen: Bool

    var body: some View {
        HStack(spacing: 12) {
            Image(systemName: workspace.glyph).foregroundStyle(.secondary).frame(width: 26)
            VStack(alignment: .leading, spacing: 2) {
                Text(workspace.name)
                if let unavailable = workspace.unavailable {
                    Text("Unavailable. \(unavailable)").font(.caption).foregroundStyle(.orange)
                } else if let about = workspace.about {
                    Text(about).font(.caption).foregroundStyle(.secondary).lineLimit(1)
                }
            }
            Spacer()
            if chosen { Image(systemName: "checkmark").foregroundStyle(.tint) }
        }
        .contentShape(Rectangle())
        .opacity(workspace.unavailable != nil ? 0.5 : 1)
    }
}
