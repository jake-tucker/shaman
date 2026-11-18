import ShamanCore
import SwiftUI

/// The sheet of Servers, Relays and This device: each Remote with its ways and how they stand, the offer to pair
/// another server, and this device.
public struct DeviceScreen: View {
  @State private var model: DeviceModel
  @State private var path: [DeviceRoute] = []
  @Environment(\.dismiss) private var dismiss
  private let core: any Core
  private let appVersion: String

  /// The screen over `core`, showing `appVersion` as the app's own version.
  public init(core: any Core, appVersion: String) {
    _model = State(initialValue: DeviceModel(core: core))
    self.core = core
    self.appVersion = appVersion
  }

  public var body: some View {
    NavigationStack(path: $path) {
      Form {
        if let device = model.device {
          if let pairAgain = device.pairAgain {
            self.pairAgain(pairAgain)
          }
          servers(model.servers?.remotes ?? [])
          thisDevice(device)
        } else {
          ProgressView()
        }
      }
      .navigationDestination(for: DeviceRoute.self) { route in
        switch route {
        case .pairAnother:
          PairingStart(core: core) { path.append(.confirm($0)) }
        case .confirm(let invite):
          ConfirmationScreen(core: core, invite: invite) {
            path.removeAll { $0 != .pairAnother }
          } paired: {
            dismiss()
          }
        }
      }
      .navigationTitle("Servers & device")
      .safeAreaInset(edge: .top) {
        if let failure = model.failure {
          Banner(text: failure)
        }
      }
    }
    .task { await model.follow() }
    .task { await model.followServers() }
  }

  /// Each Remote with its ways, and the offer to pair another server.
  private func servers(_ remotes: [ServerRow]) -> some View {
    Section("Servers") {
      ForEach(remotes, id: \.name) { remote in
        VStack(alignment: .leading, spacing: 6) {
          Text(remote.name)
            .font(.headline)
            .accessibilityIdentifier("server.\(remote.name)")
          ForEach(Array(remote.ways.enumerated()), id: \.offset) { index, way in
            HStack {
              Text(verbatim: way.address)
                .font(.footnote.monospaced())
              Spacer()
              if way.state == .checking {
                ProgressView()
                  .controlSize(.mini)
              } else {
                Text(way.words)
                  .font(.footnote)
                  .foregroundStyle(way.state == .answering ? .green : .secondary)
              }
            }
            .accessibilityElement(children: .combine)
            .accessibilityIdentifier("server.\(remote.name).way.\(index)")
          }
        }
      }
      NavigationLink(value: DeviceRoute.pairAnother) {
        Label("Pair another server", systemImage: "plus")
      }
      .accessibilityIdentifier("servers.pairAnother")
    }
  }

  /// Said once where this phone has a new key: until it is dismissed.
  private func pairAgain(_ words: String) -> some View {
    Section {
      Label {
        Text(words).accessibilityIdentifier("pairAgain")
      } icon: {
        Image(systemName: "key.horizontal")
      }
      Button("OK") { Task { await model.dismissPairAgain() } }
        .accessibilityIdentifier("pairAgain.dismiss")
    }
  }

  private func thisDevice(_ device: ThisDevice) -> some View {
    Section("This device") {
      VStack(alignment: .leading, spacing: 6) {
        LabeledContent("Name") {
          TextField("Name", text: $model.name)
            .multilineTextAlignment(.trailing)
            .textInputAutocapitalization(.never)
            .autocorrectionDisabled()
            .submitLabel(.done)
            .disabled(model.renaming)
            .onSubmit { Task { await model.rename() } }
            .accessibilityIdentifier("device.name")
        }
        if let refusal = model.refusal {
          Text(refusal)
            .font(.footnote)
            .foregroundStyle(.red)
            .accessibilityIdentifier("device.name.refusal")
        }
        Text(device.nameNote)
          .font(.footnote)
          .foregroundStyle(.secondary)
      }
      VStack(alignment: .leading, spacing: 4) {
        Text("Fingerprint")
        Fingerprint(device.fingerprint)
      }
      .accessibilityElement(children: .ignore)
      .accessibilityLabel("Fingerprint")
      .accessibilityValue(device.fingerprint)
      .accessibilityIdentifier("device.fingerprint")
      .contextMenu {
        Button("Copy", systemImage: "doc.on.doc") { UIPasteboard.general.string = device.fingerprint }
      }
      LabeledContent("Version", value: appVersion)
        .accessibilityIdentifier("device.version")
      LabeledContent("Protocol", value: String(device.protocolVersion))
        .accessibilityIdentifier("device.protocol")
    }
  }
}

/// Where the sheet of Servers, Relays and This device leads.
private enum DeviceRoute: Hashable {
  /// Pairing with another server, by pasting its Invite.
  case pairAnother
  /// Confirming the Invite pasted.
  case confirm(String)
}

/// What the sheet of Servers, Relays and This device holds while it is up.
@MainActor @Observable
final class DeviceModel {
  private let core: any Core
  private(set) var device: ThisDevice?
  private(set) var servers: Servers?
  /// The name as it is being written, until it is kept.
  var name = ""
  /// Why the core refused the name, shown beside it.
  private(set) var refusal: String?
  /// Why the last act failed otherwise, shown in a banner over the screen.
  private(set) var failure: String?
  private(set) var renaming = false

  init(core: any Core) {
    self.core = core
  }

  /// Follows what the core says of this device until the screen goes.
  func follow() async {
    for await device in core.thisDevice() {
      self.device = device
      name = device.name
    }
  }

  /// Follows what the core says of the Remotes until the screen goes.
  func followServers() async {
    for await servers in core.servers() {
      self.servers = servers
    }
  }

  func dismissPairAgain() async {
    do {
      try await core.dismissPairAgain()
      failure = nil
    } catch {
      failure = error.message
    }
  }

  func rename() async {
    renaming = true
    defer { renaming = false }
    do {
      try await core.renameDevice(name)
      refusal = nil
      failure = nil
    } catch .Invalid(let message) {
      refusal = message
      failure = nil
    } catch {
      refusal = nil
      failure = error.message
    }
  }
}

/// A key's fingerprint, whole and as a Serving Server shows it, in two halves of a line each, since a wrapped line
/// would be hyphenated with a character the fingerprint does not have.
struct Fingerprint: View {
  private let halves: [Substring]

  init(_ fingerprint: String) {
    let middle = fingerprint.index(fingerprint.startIndex, offsetBy: fingerprint.count / 2)
    halves = [fingerprint[..<middle], fingerprint[middle...]]
  }

  var body: some View {
    VStack(alignment: .leading, spacing: 0) {
      ForEach(halves.indices, id: \.self) { half in
        Text(verbatim: String(halves[half]))
          .lineLimit(1)
          .minimumScaleFactor(0.5)
      }
    }
    .font(.footnote.monospaced())
    .foregroundStyle(.secondary)
  }
}

/// An act's failure, over the screen that asked.
struct Banner: View {
  let text: String

  var body: some View {
    Text(text)
      .font(.callout)
      .frame(maxWidth: .infinity, alignment: .leading)
      .padding()
      .background(.red.opacity(0.15), in: .rect(cornerRadius: 12))
      .padding(.horizontal)
      .accessibilityIdentifier("banner")
  }
}

/// What the app shows where the Keychain gave up a blob the chip will not take: why, and the offer to start again
/// under a new key, which is the user's to take.
public struct KeyRefusedScreen: View {
  private let message: String
  private let startAgain: () -> Void

  public init(message: String, startAgain: @escaping () -> Void) {
    self.message = message
    self.startAgain = startAgain
  }

  public var body: some View {
    ContentUnavailableView {
      Label("This phone's key cannot be used", systemImage: "key.slash")
    } description: {
      Text(message)
      Text("Under a new key, every server this phone was paired with must be paired with again.")
    } actions: {
      Button("Start again under a new key", action: startAgain)
        .buttonStyle(.borderedProminent)
        .accessibilityIdentifier("key.startAgain")
    }
  }
}

/// What the app shows where the core could not be opened.
public struct CoreFailedScreen: View {
  private let message: String

  public init(message: String) {
    self.message = message
  }

  public var body: some View {
    ContentUnavailableView(
      "Shaman could not start",
      systemImage: "exclamationmark.triangle",
      description: Text(message)
    )
  }
}

#if DEBUG
  #Preview("A fresh phone") {
    DeviceScreen(core: FakeCore(.fresh), appVersion: "0.1.0 (1)")
  }

  #Preview("A named phone") {
    DeviceScreen(core: FakeCore(.named), appVersion: "0.1.0 (1)")
  }

  #Preview("A paired phone") {
    DeviceScreen(core: FakeCore(.paired), appVersion: "0.1.0 (1)")
  }

  #Preview("A phone that must pair again") {
    DeviceScreen(core: FakeCore(.pairAgain), appVersion: "0.1.0 (1)")
  }

  #Preview("A key the chip refuses") {
    KeyRefusedScreen(message: "The Secure Enclave will not take this phone's identity key: the data is corrupted.") {}
  }
#endif
