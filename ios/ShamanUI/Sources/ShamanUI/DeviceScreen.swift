import ShamanCore
import SwiftUI

/// The sheet of Servers, Relays and This device. It holds "This device" alone until there are Servers to list.
public struct DeviceScreen: View {
  @State private var model: DeviceModel
  private let appVersion: String

  /// The screen over `core`, showing `appVersion` as the app's own version.
  public init(core: any Core, appVersion: String) {
    _model = State(initialValue: DeviceModel(core: core))
    self.appVersion = appVersion
  }

  public var body: some View {
    NavigationStack {
      Form {
        if let device = model.device {
          thisDevice(device)
        } else {
          ProgressView()
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
      LabeledContent("Version", value: appVersion)
        .accessibilityIdentifier("device.version")
      LabeledContent("Protocol", value: String(device.protocolVersion))
        .accessibilityIdentifier("device.protocol")
    }
  }
}

/// What "This device" holds while its screen is up.
@MainActor @Observable
final class DeviceModel {
  private let core: any Core
  private(set) var device: ThisDevice?
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
#endif
