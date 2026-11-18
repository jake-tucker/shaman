import ShamanCore
import SwiftUI

/// What a phone holding no Remote offers, alone: pairing. Pasting an Invite pushes its confirmation screen.
public struct PairingScreen: View {
  private let core: any Core
  @State private var path: [String]

  /// The screen over `core`, already confirming `invite` where one is given, as one arriving by the launch environment
  /// in a debug build is.
  public init(core: any Core, invite: String? = nil) {
    self.core = core
    _path = State(initialValue: invite.map { [$0] } ?? [])
  }

  public var body: some View {
    NavigationStack(path: $path) {
      PairingStart(core: core) { path.append($0) }
        .navigationDestination(for: String.self) { invite in
          ConfirmationScreen(core: core, invite: invite) {
            path.removeAll()
          } paired: {
          }
        }
    }
  }
}

/// Where pairing begins: the offer to paste an Invite, and the word that this phone must pair again where it has a new
/// key.
struct PairingStart: View {
  let core: any Core
  /// Takes the Invite pasted.
  let pasted: (String) -> Void
  @State private var device: ThisDevice?
  @State private var failure: String?

  /// What the core says to do, its commands drawn as code.
  private var prompt: AttributedString {
    let words = core.pairingPrompt()
    return (try? AttributedString(markdown: words)) ?? AttributedString(words)
  }

  var body: some View {
    Form {
      if let pairAgain = device?.pairAgain {
        Section {
          Label {
            Text(pairAgain).accessibilityIdentifier("pairAgain")
          } icon: {
            Image(systemName: "key.horizontal")
          }
          Button("OK") { Task { await dismissPairAgain() } }
            .accessibilityIdentifier("pairAgain.dismiss")
        }
      }
      Section {
        VStack(alignment: .leading, spacing: 12) {
          Image(systemName: "link")
            .font(.largeTitle)
            .foregroundStyle(.tint)
          Text("Pair with a server")
            .font(.title2.bold())
          Text(prompt)
            .foregroundStyle(.secondary)
        }
        .padding(.vertical, 8)
        PasteButton(payloadType: String.self) { strings in
          guard let invite = strings.first else { return }
          Task { @MainActor in pasted(invite) }
        }
        .accessibilityIdentifier("pairing.paste")
      }
    }
    .navigationTitle("Pairing")
    .navigationBarTitleDisplayMode(.inline)
    .safeAreaInset(edge: .top) {
      if let failure {
        Banner(text: failure)
      }
    }
    .task {
      for await device in core.thisDevice() {
        self.device = device
      }
    }
  }

  private func dismissPairAgain() async {
    do {
      try await core.dismissPairAgain()
      failure = nil
    } catch {
      failure = error.message
    }
  }
}

/// The confirmation screen: what an Invite says of the Server before anything is dialled, the names the Remote and
/// this phone will go by, and Pair. A failed pairing stays here with the Server's words and what to do next.
struct ConfirmationScreen: View {
  @State private var model: ConfirmationModel
  /// Goes back for another Invite.
  private let scanAgain: () -> Void
  /// Told once the Remote is paired.
  private let paired: () -> Void

  init(core: any Core, invite: String, scanAgain: @escaping () -> Void, paired: @escaping () -> Void) {
    _model = State(initialValue: ConfirmationModel(core: core, invite: invite))
    self.scanAgain = scanAgain
    self.paired = paired
  }

  var body: some View {
    Form {
      switch model.reading {
      case .unreadable(let message):
        Section {
          Text(message)
            .foregroundStyle(.red)
            .accessibilityIdentifier("confirm.unreadable")
          Button("Scan again", action: scanAgain)
            .accessibilityIdentifier("confirm.scanAgain")
        }
      case .readable(let invite):
        readable(invite)
      }
    }
    .navigationTitle("Pair")
    .navigationBarTitleDisplayMode(.inline)
  }

  @ViewBuilder
  private func readable(_ invite: ReadInvite) -> some View {
    if let held = invite.held {
      Section {
        Label(held, systemImage: "arrow.triangle.2.circlepath")
          .accessibilityElement(children: .ignore)
          .accessibilityLabel(held)
          .accessibilityIdentifier("confirm.held")
      }
    }
    Section("Server") {
      LabeledContent("Hostname", value: invite.hostname)
        .accessibilityElement(children: .ignore)
        .accessibilityLabel("Hostname")
        .accessibilityValue(invite.hostname)
        .accessibilityIdentifier("confirm.hostname")
      VStack(alignment: .leading, spacing: 4) {
        Text("Fingerprint")
        Fingerprint(invite.fingerprint)
      }
      .accessibilityElement(children: .ignore)
      .accessibilityLabel("Fingerprint")
      .accessibilityValue(invite.fingerprint)
      .accessibilityIdentifier("confirm.fingerprint")
    }
    Section("Ways") {
      ForEach(Array(invite.ways.enumerated()), id: \.offset) { index, way in
        VStack(alignment: .leading, spacing: 2) {
          Text(verbatim: way.address)
            .font(.callout.monospaced())
          if let note = way.note {
            Text(note)
              .font(.footnote)
              .foregroundStyle(.secondary)
          }
        }
        .accessibilityElement(children: .combine)
        .accessibilityIdentifier("confirm.way.\(index)")
      }
    }
    Section("Names") {
      name("Server", text: $model.remoteName, refusal: model.remoteRefusal, identifier: "confirm.remoteName")
      name("This device", text: $model.deviceName, refusal: model.deviceRefusal, identifier: "confirm.deviceName")
    }
    Section {
      if let failure = model.failure {
        Text(failure.message)
          .foregroundStyle(.red)
          .accessibilityIdentifier("confirm.failure")
      }
      if model.pairing {
        HStack {
          ProgressView()
          Text("Pairing…")
        }
        .accessibilityIdentifier("confirm.pairing")
      } else {
        switch model.failure?.again {
        case .scanAgain:
          Button("Scan again", action: scanAgain)
            .accessibilityIdentifier("confirm.scanAgain")
        case .tryAgain:
          Button("Try again") { Task { await pair() } }
            .accessibilityIdentifier("confirm.tryAgain")
        case nil:
          Button("Pair") { Task { await pair() } }
            .bold()
            .accessibilityIdentifier("confirm.pair")
        }
      }
    }
  }

  private func name(_ label: String, text: Binding<String>, refusal: String?, identifier: String) -> some View {
    VStack(alignment: .leading, spacing: 6) {
      LabeledContent(label) {
        TextField(label, text: text)
          .multilineTextAlignment(.trailing)
          .textInputAutocapitalization(.never)
          .autocorrectionDisabled()
          .disabled(model.pairing)
          .accessibilityIdentifier(identifier)
      }
      if let refusal {
        Text(refusal)
          .font(.footnote)
          .foregroundStyle(.red)
          .accessibilityIdentifier("\(identifier).refusal")
      }
    }
  }

  private func pair() async {
    if await model.pair() {
      paired()
    }
  }
}

/// What the confirmation screen holds while it is up.
@MainActor @Observable
final class ConfirmationModel {
  private let core: any Core
  private let invite: String
  let reading: InviteReading
  var remoteName: String
  var deviceName: String
  private(set) var pairing = false
  /// Why the last pairing failed, and what it offers next.
  private(set) var failure: (message: String, again: TryInstead)?
  private(set) var remoteRefusal: String?
  private(set) var deviceRefusal: String?

  init(core: any Core, invite: String) {
    self.core = core
    self.invite = invite
    reading = core.readInvite(invite)
    if case .readable(let read) = reading {
      remoteName = read.remoteName
      deviceName = read.deviceName
    } else {
      remoteName = ""
      deviceName = ""
    }
  }

  /// Pairs as the names say, answering whether the Remote is paired.
  func pair() async -> Bool {
    pairing = true
    defer { pairing = false }
    failure = nil
    remoteRefusal = nil
    deviceRefusal = nil
    switch await core.confirmPairing(invite: invite, remoteName: remoteName, deviceName: deviceName) {
    case .paired:
      return true
    case .failed(let message, let again):
      failure = (message, again)
    case .remoteNameRefused(let message):
      remoteRefusal = message
    case .deviceNameRefused(let message):
      deviceRefusal = message
    }
    return false
  }
}

#if DEBUG
  #Preview("Pairing") {
    PairingScreen(core: FakeCore(.fresh))
  }

  #Preview("Confirming an Invite") {
    PairingScreen(core: FakeCore(.fresh), invite: FakeCore.invite)
  }

  #Preview("A malformed Invite") {
    PairingScreen(core: FakeCore(.fresh), invite: "shaman-v2-not-yet")
  }
#endif
