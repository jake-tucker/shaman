import ShamanCore
import SwiftUI

#if DEBUG
  /// The sheet of Servers, Relays and This device alone at a window's root, as a debug build is asked for it: the
  /// window asks the device's core for its own half once, as it is first drawn, as `RootScreen` does.
  public struct DeviceRoot: View {
    @State private var held: HeldWindow
    private let appVersion: String

    /// The sheet over a window of the device's `core`, showing `appVersion` as the app's own version.
    public init(core: any DeviceCore, appVersion: String) {
      _held = State(initialValue: HeldWindow(over: core))
      self.appVersion = appVersion
    }

    public var body: some View {
      DeviceScreen(core: held.core, appVersion: appVersion)
    }
  }
#endif

/// The sheet of Servers, Relays and This device: each Remote with its ways and how they stand, the offer to pair
/// another server, and this device.
public struct DeviceScreen: View {
  @State private var model: DeviceModel
  @State private var path: [DeviceRoute] = []
  @State private var scanning: ScanningModel
  /// The Remote whose name is being written, while one is.
  @FocusState private var naming: String?
  @Environment(\.dismiss) private var dismiss
  private let core: any WindowCore
  private let appVersion: String

  /// The screen over `core`, showing `appVersion` as the app's own version.
  public init(core: any WindowCore, appVersion: String) {
    _model = State(initialValue: DeviceModel(core: core))
    _scanning = State(initialValue: ScanningModel(core: core))
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
          relays(model.servers?.relays ?? [])
          thisDevice(device)
        } else {
          ProgressView()
        }
      }
      .navigationDestination(for: DeviceRoute.self) { route in
        switch route {
        case .pairAnother:
          PairingStart(core: core) {
            path.append(.scan)
          } pasted: {
            path.append(.confirm($0))
          }
        case .scan:
          ScanScreen(model: scanning) { path.append(.confirm($0)) }
        case .confirm(let invite):
          // Back to the scanner where it read the Invite, and otherwise to where pairing begins.
          ConfirmationScreen(core: core, invite: invite) {
            path.removeLast()
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
      .noticeBanner(covered: model.loggingIn != nil)
    }
    // The scanner scans only while it stands at the top of the path.
    .onChange(of: path, initial: true) { scanning.stands(atTop: path.last == .scan) }
    .task { await model.follow() }
    .task { await model.followServers() }
    .onChange(of: naming) { was, _ in
      // A name left unkept goes back to the one the core holds.
      if let was { model.leftNaming(was) }
    }
    .confirmationDialog(
      "Remove server", isPresented: model.removingRemoteShown, titleVisibility: .visible,
      presenting: model.removingRemote
    ) { row in
      Button("Remove", role: .destructive) { Task { await model.remove(row) } }
        .accessibilityIdentifier("server.remove.confirm")
    } message: { row in
      Text(verbatim: row.removal)
    }
    .relayLogin($model.loggingIn, core: core) { _ in }
    .confirmationDialog(
      "Remove relay", isPresented: model.removingShown, titleVisibility: .visible, presenting: model.removing
    ) { row in
      Button("Remove", role: .destructive) { Task { await model.remove(row) } }
        .accessibilityIdentifier("relay.remove.confirm")
    } message: { row in
      Text(verbatim: row.removal)
    }
  }

  /// Each Remote with its ways, and the offer to pair another server.
  private func servers(_ remotes: [ServerRow]) -> some View {
    Section("Servers") {
      ForEach(remotes, id: \.remote) { remote in
        VStack(alignment: .leading, spacing: 6) {
          HStack {
            TextField("Name", text: model.name(of: remote))
              .font(.headline)
              .textInputAutocapitalization(.never)
              .autocorrectionDisabled()
              .submitLabel(.done)
              .focused($naming, equals: remote.remote)
              .disabled(model.renamingRemote == remote.remote || model.removingNow == remote.remote)
              .onSubmit { model.rename(remote) }
              .accessibilityIdentifier("server.\(remote.name)")
            if model.removingNow == remote.remote {
              ProgressView()
                .accessibilityIdentifier("server.\(remote.name).removing")
            } else if naming != remote.remote {
              // The name is written over where it stands.
              Image(systemName: "pencil")
                .foregroundStyle(.tertiary)
                .accessibilityHidden(true)
            }
          }
          if let refusal = model.nameRefusals[remote.remote] {
            Text(refusal)
              .font(.footnote)
              .foregroundStyle(.red)
              .accessibilityIdentifier("server.\(remote.name).refusal")
          }
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
            if let login = way.login {
              Button(login.words) { model.loggingIn = LoginAsk(relay: login.relay) }
                .buttonStyle(.bordered)
                .controlSize(.small)
                .frame(maxWidth: .infinity, alignment: .trailing)
                .accessibilityIdentifier("server.\(remote.name).way.\(index).login")
            }
          }
        }
        .swipeActions {
          Button("Remove", role: .destructive) { model.removingRemote = remote }
            .accessibilityIdentifier("server.\(remote.name).remove")
        }
      }
      NavigationLink(value: DeviceRoute.pairAnother) {
        Label("Pair another server", systemImage: "plus")
      }
      .accessibilityIdentifier("servers.pairAnother")
    }
  }

  /// Each Relay this phone holds a Login at with its Account and how it stands, removed by a swipe.
  @ViewBuilder
  private func relays(_ relays: [RelayRow]) -> some View {
    if !relays.isEmpty {
      Section("Relays") {
        ForEach(relays, id: \.relay) { relay in
          VStack(alignment: .leading, spacing: 4) {
            HStack {
              Text(verbatim: relay.address)
                .font(.callout.monospaced())
              Spacer()
              Text(relay.words)
                .font(.footnote)
                .foregroundStyle(relay.state == .loggedIn ? .green : .secondary)
            }
            if let account = relay.account {
              Text(verbatim: account)
                .font(.footnote)
                .foregroundStyle(.secondary)
            }
            if let login = relay.login {
              Button(login.words) { model.loggingIn = LoginAsk(relay: login.relay) }
                .buttonStyle(.bordered)
                .controlSize(.small)
                .accessibilityIdentifier("relay.\(relay.address).login")
            }
          }
          .accessibilityElement(children: relay.login == nil ? .combine : .contain)
          .accessibilityIdentifier("relay.\(relay.address)")
          .swipeActions {
            Button("Remove", role: .destructive) { model.removing = relay }
              .accessibilityIdentifier("relay.\(relay.address).remove")
          }
        }
      }
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
          .accessibilityIdentifier("device.name.note")
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
  /// Pairing with another server, by scanning its Invite or pasting it.
  case pairAnother
  /// Scanning an Invite.
  case scan
  /// Confirming the Invite scanned or pasted.
  case confirm(String)
}

/// What the sheet of Servers, Relays and This device holds while it is up.
@MainActor @Observable
final class DeviceModel {
  private let core: any WindowCore
  private(set) var device: ThisDevice?
  private(set) var servers: Servers?
  /// The name as it is being written, until it is kept.
  var name = ""
  /// Why the core refused the name, shown beside it.
  private(set) var refusal: String?
  /// Why the last act failed otherwise, shown in a banner over the screen.
  private(set) var failure: String?
  private(set) var renaming = false
  /// The login at a Relay the sheet's own sheet is carrying out, while it is up.
  var loggingIn: LoginAsk?
  /// The Relay entry whose removal is being confirmed, while it is.
  var removing: RelayRow?
  /// Each Remote's name as it is being written, by the Remote, until it is kept or left.
  private var remoteNames: [String: String] = [:]
  /// Why the core refused a Remote's name, by the Remote, shown beside it.
  private(set) var nameRefusals: [String: String] = [:]
  /// The Remote whose renaming is under way, while one is.
  private(set) var renamingRemote: String?
  /// The Remote whose removal is being confirmed, while it is.
  var removingRemote: ServerRow?
  /// The Remote whose removal is under way, while one is.
  private(set) var removingNow: String?

  /// Whether the removal of a Remote is being confirmed.
  var removingRemoteShown: Binding<Bool> {
    Binding {
      self.removingRemote != nil
    } set: {
      if !$0 { self.removingRemote = nil }
    }
  }

  /// The name of `remote` as it is being written, or as the core holds it.
  func name(of remote: ServerRow) -> Binding<String> {
    Binding {
      self.remoteNames[remote.remote] ?? remote.name
    } set: {
      self.remoteNames[remote.remote] = $0
    }
  }

  /// Whether the removal of a Relay entry is being confirmed.
  var removingShown: Binding<Bool> {
    Binding {
      self.removing != nil
    } set: {
      if !$0 { self.removing = nil }
    }
  }

  init(core: any WindowCore) {
    self.core = core
  }

  /// Follows what the core says of this device until the screen goes. The core says it whole whatever of it changed, so
  /// a name being written stays, as does why it was refused, unless the name the core holds has become another.
  func follow() async {
    for await device in core.device.thisDevice() {
      if device.name != self.device?.name {
        name = device.name
        refusal = nil
      }
      self.device = device
    }
  }

  /// Follows what the core says of the Remotes until the screen goes.
  func followServers() async {
    for await servers in core.device.servers() {
      self.servers = servers
    }
  }

  /// Renames `remote` to the name written for it, keeping its Pairing: begun at once, so the name is taken before the
  /// field is left.
  func rename(_ remote: ServerRow) {
    guard let name = remoteNames[remote.remote], name != remote.name else {
      nameRefusals[remote.remote] = nil
      remoteNames[remote.remote] = nil
      return
    }
    renamingRemote = remote.remote
    Task { await rename(remote, to: name) }
  }

  private func rename(_ remote: ServerRow, to name: String) async {
    defer { renamingRemote = nil }
    do {
      try await core.device.renameRemote(remote.remote, to: name)
      remoteNames[remote.remote] = nil
      nameRefusals[remote.remote] = nil
      failure = nil
    } catch .Invalid(let message) {
      nameRefusals[remote.remote] = message
      failure = nil
    } catch {
      nameRefusals[remote.remote] = nil
      failure = error.message
    }
  }

  /// The name of the Remote `remote` was left unkept: it reads as the core holds it again, unless the core refused it,
  /// when what was written stays beside why.
  func leftNaming(_ remote: String) {
    guard renamingRemote != remote, nameRefusals[remote] == nil else { return }
    remoteNames[remote] = nil
  }

  /// Removes the Remote `row`, ending its Pairing.
  func remove(_ row: ServerRow) async {
    removingRemote = nil
    removingNow = row.remote
    defer { removingNow = nil }
    do {
      try await core.device.removeRemote(row.remote)
      remoteNames[row.remote] = nil
      nameRefusals[row.remote] = nil
      failure = nil
    } catch {
      failure = error.message
    }
  }

  /// Removes the Relay entry `row`, giving up this phone's Login there.
  func remove(_ row: RelayRow) async {
    removing = nil
    do {
      try await core.device.removeRelay(row.relay)
      failure = nil
    } catch {
      failure = error.message
    }
  }

  func dismissPairAgain() async {
    do {
      try await core.device.dismissPairAgain()
      failure = nil
    } catch {
      failure = error.message
    }
  }

  func rename() async {
    renaming = true
    defer { renaming = false }
    do {
      try await core.device.renameDevice(name)
      // As the core kept it, which it may have trimmed to the name it held, and says first to whoever asks.
      for await device in core.device.thisDevice() {
        self.device = device
        name = device.name
        break
      }
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

/// An act's failure, over the screen that asked; or, `quiet`, what the core says of its own accord, which is no
/// failure.
struct Banner: View {
  let text: String
  var quiet = false

  var body: some View {
    Text(text)
      .font(.callout)
      .frame(maxWidth: .infinity, alignment: .leading)
      .padding()
      .background {
        if quiet {
          RoundedRectangle(cornerRadius: 12).fill(.regularMaterial)
        } else {
          RoundedRectangle(cornerRadius: 12).fill(.red.opacity(0.15))
        }
      }
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
    DeviceScreen(core: FakeWindow(.fresh), appVersion: "0.1.0 (1)")
  }

  #Preview("A named phone") {
    DeviceScreen(core: FakeWindow(.named), appVersion: "0.1.0 (1)")
  }

  #Preview("A paired phone") {
    DeviceScreen(core: FakeWindow(.paired), appVersion: "0.1.0 (1)")
  }

  #Preview("A phone that must pair again") {
    DeviceScreen(core: FakeWindow(.pairAgain), appVersion: "0.1.0 (1)")
  }

  #Preview("A key the chip refuses") {
    KeyRefusedScreen(message: "The Secure Enclave will not take this phone's identity key: the data is corrupted.") {}
  }
#endif
