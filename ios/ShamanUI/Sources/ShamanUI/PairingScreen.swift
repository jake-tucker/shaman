import ShamanCore
import SwiftUI
import UIKit

/// What a phone holding no Remote offers, alone, across the whole window: pairing. Scan pushes the scanner, and the
/// Invite it reads its confirmation screen over it; pasting an Invite pushes its confirmation screen.
///
/// It is the root's, where it stands its screens, so the root's alone are laid out so: the sheet of "Servers & device"
/// pushes the same screens, which the system places. Where pairing begins and the confirmation read in a column, as a
/// conversation does, and the scanner's picture keeps the whole window. How the window stands about the fold is read
/// here as it is above the columns: while the device is partly folded, the scanner stands whole in the half before the
/// fold, the leading half partly folded like a book and the upper half sat like a laptop, and the forms stand in the
/// leading half partly folded like a book, and sat like a laptop scroll through the fold. A fold tells the scanner
/// nothing new.
public struct PairingScreen: View {
  private let core: any WindowCore
  @State private var path: [PairingRoute]
  @State private var scanning: ScanningModel
  /// How the window stands about the fold, as it is read here.
  @State private var pose: Pose

  /// The screen over `core`, already confirming `invite` where one is given, as one arriving by the launch environment
  /// in a debug build is.
  public init(core: any WindowCore, invite: String? = nil) {
    self.init(core: core, path: invite.map { [.confirm($0)] } ?? [], pose: Pose())
  }

  /// The screen over `core`, at `path`, reading how the window stands into `pose`.
  init(core: any WindowCore, path: [PairingRoute], pose: Pose) {
    self.core = core
    _path = State(initialValue: path)
    _scanning = State(initialValue: ScanningModel(core: core))
    _pose = State(initialValue: pose)
  }

  public var body: some View {
    NavigationStack(path: $path) {
      PairingStart(core: core) {
        path.append(.scan)
      } pasted: {
        path.append(.confirm($0))
      }
      // Where a server ended this phone's last pairing, what the core says of it stands here.
      .noticeBanner()
      .readsInAColumn(pose)
      .navigationDestination(for: PairingRoute.self) { route in
        switch route {
        case .scan:
          ScanScreen(model: scanning, pose: pose) { path.append(.confirm($0)) }
        case .confirm(let invite):
          // Back to the scanner where it read the Invite, and otherwise to where pairing begins.
          ConfirmationScreen(core: core, invite: invite) {
            path.removeLast()
          } paired: {
          }
          .readsInAColumn(pose)
        }
      }
    }
    // The scanner scans only while it stands at the top of the path.
    .onChange(of: path, initial: true) { scanning.stands(atTop: path.last == .scan) }
    .reads(pose)
    .readsTheBooksFold(pose)
  }
}

extension View {
  /// Has a form pairing stands at the root read in a column at most as wide as a conversation reads, in the middle of
  /// the window, on the grouped ground; and stand in the leading half partly folded like a book, as `pose` says, and
  /// sat like a laptop scroll through the fold.
  fileprivate func readsInAColumn(_ pose: Pose) -> some View {
    frame(maxWidth: ReadingSides.widest)
      .frame(maxWidth: .infinity)
      .background { Color(.systemGroupedBackground).ignoresSafeArea() }
      .standsBeforeTheFold(pose, scrollsThroughItSatLikeALaptop: true)
  }

  /// Stands it whole in the half of the window before the fold, the other half plain ground, while `pose` says the
  /// device is partly folded: the leading half like a book, and the upper half sat like a laptop, unless it
  /// `scrollsThroughItSatLikeALaptop`. With no pose, or none folded, it stands as it would without.
  func standsBeforeTheFold(_ pose: Pose?, scrollsThroughItSatLikeALaptop: Bool = false) -> some View {
    modifier(StandsBeforeTheFold(pose: pose, scrollsThrough: scrollsThroughItSatLikeALaptop))
  }
}

/// The half after the fold as plain ground, as wide or tall as it is from where the band begins, so that what it holds
/// ends at the fold, scrolling or not: the window's own safe area at that side is ignored, so the ground stands on the
/// window's edge, and goes under the vertical bar standing there partly folded like a book. It pads what it holds, as a
/// list runs on under an inset of the safe area and would scroll across the fold.
private struct StandsBeforeTheFold: ViewModifier {
  let pose: Pose?
  let scrollsThrough: Bool

  func body(content: Content) -> some View {
    let book = pose?.book
    let fold = scrollsThrough ? nil : pose?.fold
    var edges: Edge.Set = []
    if book != nil { edges.insert(.trailing) }
    if fold != nil { edges.insert(.bottom) }
    return
      content
      .padding(.trailing, book?.after ?? 0)
      .padding(.bottom, fold?.below ?? 0)
      .ignoresSafeArea(.container, edges: edges)
      .background {
        if !edges.isEmpty {
          Color(.systemGroupedBackground).ignoresSafeArea()
        }
      }
  }
}

/// Where pairing leads: the scanner, and the confirmation screen of an Invite scanned or pasted.
enum PairingRoute: Hashable {
  case scan
  case confirm(String)
}

/// Where pairing begins: the offers to scan an Invite and to paste one, and the word that this phone must pair again
/// where it has a new key.
struct PairingStart: View {
  let core: any WindowCore
  /// Opens the scanner, once the camera may be used.
  let scan: () -> Void
  /// Takes the Invite pasted.
  let pasted: (String) -> Void
  @State private var device: ThisDevice?
  @State private var failure: String?
  @State private var offer = ScanOffer()
  @Environment(\.codeScanner) private var scanner
  @Environment(\.openURL) private var openURL

  /// What the core says to do, its commands drawn as code.
  private var prompt: AttributedString {
    let words = core.device.pairingPrompt()
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
        Button {
          Task {
            if await offer.scan(with: scanner) {
              scan()
            }
          }
        } label: {
          Label("Scan", systemImage: "qrcode.viewfinder")
        }
        .disabled(offer.asking)
        .accessibilityIdentifier("pairing.scan")
        switch offer.access {
        case .refused:
          VStack(alignment: .leading, spacing: 8) {
            Text(core.device.scanWords().cameraRefused)
              .foregroundStyle(.secondary)
              .accessibilityIdentifier("pairing.scan.refused")
            Button("Open Settings") {
              if let settings = URL(string: UIApplication.openSettingsURLString) {
                openURL(settings)
              }
            }
            // Only the button takes a tap, not the words beside it in the row.
            .buttonStyle(.borderless)
            .accessibilityIdentifier("pairing.openSettings")
          }
        case .unavailable:
          Text(core.device.scanWords().noCamera)
            .foregroundStyle(.secondary)
            .accessibilityIdentifier("pairing.scan.unavailable")
        case .granted, nil:
          EmptyView()
        }
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
      for await device in core.device.thisDevice() {
        self.device = device
      }
    }
  }

  private func dismissPairAgain() async {
    do {
      try await core.device.dismissPairAgain()
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

  init(core: any WindowCore, invite: String, scanAgain: @escaping () -> Void, paired: @escaping () -> Void) {
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
    .relayLogin($model.loggingIn, core: model.core) { _ in
      Task {
        if await model.loggedIn() {
          paired()
        }
      }
    }
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
        HStack {
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
          if let login = way.login {
            Spacer()
            Button(login.words) { model.logIn(login) }
              .buttonStyle(.bordered)
              .controlSize(.small)
              .disabled(model.pairing)
              .accessibilityIdentifier("confirm.way.\(index).login")
          }
        }
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
        case .logIn(let login, let tryAgain):
          Button(login.words) { model.logIn(login) }
            .bold()
            .accessibilityIdentifier("confirm.login")
          Button(tryAgain) { Task { await pair() } }
            .accessibilityIdentifier("confirm.tryAgain")
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
  let core: any WindowCore
  private let invite: String
  /// What the Invite reads as, read again once a login it offers is done.
  private(set) var reading: InviteReading
  var remoteName: String
  var deviceName: String
  private(set) var pairing = false
  /// Why the last pairing failed, and what it offers next.
  private(set) var failure: (message: String, again: Again)?
  /// The login at a Relay the screen's sheet is carrying out, while it is up.
  var loggingIn: LoginAsk?

  /// What a failed pairing offers next.
  enum Again {
    case scanAgain
    case tryAgain
    /// A login at a Relay, after which the pairing goes on by itself, or trying again as it is, in the words given.
    case logIn(LoginOffer, tryAgain: String)
  }
  private(set) var remoteRefusal: String?
  private(set) var deviceRefusal: String?

  init(core: any WindowCore, invite: String) {
    self.core = core
    self.invite = invite
    let reading = core.device.readInvite(invite)
    self.reading = reading
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
      failure = (message, again == .scanAgain ? .scanAgain : .tryAgain)
    case .loginNeeded(let message, let login, let tryAgain):
      failure = (message, .logIn(login, tryAgain: tryAgain))
    case .remoteNameRefused(let message):
      remoteRefusal = message
    case .deviceNameRefused(let message):
      deviceRefusal = message
    }
    return false
  }

  /// Opens the sheet a login at the Relay `login` offers is carried out in.
  func logIn(_ login: LoginOffer) {
    loggingIn = LoginAsk(relay: login.relay)
  }

  /// A login done, what the Invite offers is read again, and where pairing waited on that login it goes on by itself:
  /// answers whether the Remote is paired.
  func loggedIn() async -> Bool {
    reading = core.device.readInvite(invite)
    guard case .logIn = failure?.again else { return false }
    return await pair()
  }
}

#if DEBUG
  #Preview("Pairing") {
    PairingScreen(core: FakeWindow(.fresh))
  }

  #Preview("Scanning") {
    PairingScreen(core: FakeWindow(.fresh))
      .environment(\.codeScanner, .reading(["https://example.com", FakeCore.invite]))
  }

  #Preview("Camera refused") {
    PairingScreen(core: FakeWindow(.fresh))
      .environment(\.codeScanner, .reading([], access: .refused))
  }

  #Preview("Confirming an Invite") {
    PairingScreen(core: FakeWindow(.fresh), invite: FakeCore.invite)
  }

  #Preview("A malformed Invite") {
    PairingScreen(core: FakeWindow(.fresh), invite: "shaman-v2-not-yet")
  }
#endif
