// PROTOTYPE — throwaway. The screens the listing leads to besides a Session, stubbed just far
// enough to show where each way in lands.

import SwiftUI

/// The Remotes the phone holds, their ways, and the way into pairing another.
struct ServersList: View {
    @Environment(Store.self) private var store
    @State private var pairing = false
    @State private var login: Remote?

    var body: some View {
        List {
            ForEach(store.remotes) { remote in
                Section {
                    ForEach(store.ways(remote.id)) { way in
                        HStack {
                            Image(systemName: way.relay ? "arrow.triangle.swap" : "point.3.connected.trianglepath.dotted")
                                .foregroundStyle(.secondary)
                            VStack(alignment: .leading) {
                                Text(way.address).font(.callout.monospaced())
                                Text(way.relay ? "Relay" : "Direct").font(.caption).foregroundStyle(.secondary)
                            }
                            Spacer()
                            switch way.state {
                            case .answering:
                                Text("answering").font(.caption).foregroundStyle(.green)
                            case .notAnswering:
                                Text("not answering").font(.caption).foregroundStyle(.secondary)
                            case .loginNeeded:
                                Text("login needed").font(.caption).foregroundStyle(.orange)
                                Button("Log in") { login = remote }
                                    .buttonStyle(.bordered)
                                    .controlSize(.small)
                            }
                        }
                    }
                    if store.reach(remote.id) != .reachable {
                        UnreachableActions(remote: remote)
                    }
                } header: {
                    Label(remote.id, systemImage: remote.glyph)
                } footer: {
                    if store.workspace(store.outlookWorkspace).remote == remote.id {
                        Text("New sessions begin here.")
                    }
                }
            }
            Section {
                Button { pairing = true } label: { Label("Pair another server", systemImage: "qrcode.viewfinder") }
            }
            Section("This phone") {
                LabeledContent("Goes by", value: "My iPhone")
            }
        }
        .navigationTitle("Servers")
        .navigationBarTitleDisplayMode(.inline)
        .sheet(isPresented: $pairing) { PairStub() }
        .sheet(item: $login) { DeviceLoginStub(remote: $0) }
    }
}

struct ServersSheet: View {
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        NavigationStack {
            ServersList()
                .toolbar {
                    ToolbarItem(placement: .confirmationAction) { Button("Done") { dismiss() } }
                }
        }
    }
}

struct PairStub: View {
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        NavigationStack {
            VStack(spacing: 16) {
                Image(systemName: "qrcode.viewfinder").font(.system(size: 64)).foregroundStyle(.secondary)
                Text("Run /serve on the machine to pair with, then scan the code it draws.")
                    .multilineTextAlignment(.center)
                    .foregroundStyle(.secondary)
                Button("Scan") { dismiss() }.buttonStyle(.borderedProminent).controlSize(.large)
                Button("Paste an invite") { dismiss() }
            }
            .padding(32)
            .navigationTitle("Pair a server")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { Button("Cancel") { dismiss() } }
            }
        }
        .presentationDetents([.medium])
    }
}

/// An Unreachable Remote's offer to try again, in its three variants.
struct UnreachableActions: View {
    @Environment(Store.self) private var store
    let remote: Remote
    @State private var trying = false
    @State private var login = false

    var body: some View {
        let why: Reach.Why = if case .unreachable(let why) = store.reach(remote.id) { why } else { .plain }
        VStack(alignment: .leading, spacing: 10) {
            switch why {
            case .plain:
                Text("Not answering by any way it offers. Still trying.")
            case .loginNeeded:
                Text("Its direct address is not answering, and its relay needs a login from this phone.")
            case .localNetwork:
                Text("Not answering. This phone may have refused Shaman the local network.")
            }
            HStack {
                Button {
                    trying = true
                    Task {
                        try? await Task.sleep(for: .seconds(1))
                        trying = false
                        if why == .plain, store.scenario != .offline { store.healed.insert(remote.id) }
                    }
                } label: {
                    if trying { ProgressView() } else { Text("Try again") }
                }
                .buttonStyle(.bordered)
                if why == .loginNeeded {
                    Button("Log in at relay.shaman.dev") { login = true }.buttonStyle(.borderedProminent)
                }
                if why == .localNetwork {
                    Button("Open Settings") { store.healed.insert(remote.id) }.buttonStyle(.borderedProminent)
                }
            }
            .controlSize(.small)
        }
        .font(.footnote)
        .foregroundStyle(.secondary)
        .sheet(isPresented: $login) { DeviceLoginStub(remote: remote) }
    }
}

struct UnreachableSheet: View {
    let remote: Remote

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            Label("\(remote.id) is unreachable", systemImage: "wifi.slash").font(.headline)
            UnreachableActions(remote: remote)
            Text("Its sessions stay listed as it last gave them, dimmed.")
                .font(.footnote)
                .foregroundStyle(.tertiary)
            Spacer()
        }
        .padding(24)
        .presentationDetents([.height(240)])
    }
}

struct DeviceLoginStub: View {
    @Environment(Store.self) private var store
    @Environment(\.dismiss) private var dismiss
    let remote: Remote

    var body: some View {
        VStack(spacing: 18) {
            Text("Log in at relay.shaman.dev").font(.headline)
            Text("WDJB-MJHT").font(.system(size: 34, weight: .semibold, design: .monospaced))
            Text("Enter this code at GitHub. It opens in a sheet over this screen.")
                .font(.footnote)
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.center)
            Button("Open GitHub") {
                store.healed.insert(remote.id)
                dismiss()
            }
            .buttonStyle(.borderedProminent)
        }
        .padding(28)
        .presentationDetents([.medium])
    }
}
