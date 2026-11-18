import Foundation
import ShamanUI
import SwiftUI

@main
struct ShamanApp: App {
  @State private var opened = Opened.choose()

  var body: some Scene {
    WindowGroup {
      switch opened {
      case .core(let core):
        #if DEBUG
          if Launch.serversAndDevice {
            DeviceScreen(core: core, appVersion: Self.version)
          } else {
            RootScreen(core: core, appVersion: Self.version, invite: Launch.invite)
          }
        #else
          RootScreen(core: core, appVersion: Self.version)
        #endif
      case .failed(let message): CoreFailedScreen(message: message)
      case .keyRefused(let message, let directory, let item):
        KeyRefusedScreen(message: message) {
          opened = Opened.open(directory, IdentityKey.startAgain(in: item), item)
        }
      #if DEBUG
        case .removed(let name):
          Text("Removed the core's directory \"\(name)\".").accessibilityIdentifier("core.removed")
      #endif
      }
    }
  }

  /// The app's own version and its build number, as the Info settings give them.
  private static var version: String {
    let info = Bundle.main.infoDictionary ?? [:]
    let version = info["CFBundleShortVersionString"] as? String ?? "?"
    let build = info["CFBundleVersion"] as? String ?? "?"
    return "\(version) (\(build))"
  }
}

#if DEBUG
  /// What a debug build is asked at launch beyond which core to run over.
  private enum Launch {
    /// An Invite arriving by the launch environment's `SHAMAN_INVITE`, confirmed at once, as a pasted one is.
    static var invite: String? {
      ProcessInfo.processInfo.environment["SHAMAN_INVITE"].flatMap { $0.isEmpty ? nil : $0 }
    }

    /// Whether `--servers-and-device` asks for the sheet of Servers, Relays and This device alone, which a phone
    /// holding no Remote offers no way to, so a UI test can reach "This device" before pairing.
    static var serversAndDevice: Bool {
      ProcessInfo.processInfo.arguments.contains("--servers-and-device")
    }
  }
#endif

/// The core the app runs over, or why there is none.
private enum Opened {
  case core(any Core)
  case failed(String)
  /// The key cannot prove this phone, as where the Keychain gave up a blob the chip will not take: the core in
  /// `directory` opens once the user chooses to start again under a new key, kept in `item`.
  case keyRefused(String, directory: URL, item: KeyItem)
  #if DEBUG
    /// The directory a run kept its records in was removed, as `--remove-core-directory` asks.
    case removed(String)
  #endif

  /// The live core, kept in its directory and proving this phone by the identity key in the Keychain. In debug builds
  /// `--fake <scenario>` chooses the fake instead, and the environment's `SHAMAN_CORE_DIRECTORY` names a directory of
  /// its own for the live core and `SHAMAN_KEY_ITEM` a Keychain item of its own for its key, so a UI test starts
  /// afresh; `--remove-core-directory` removes that directory, and that item where one is named, and opens nothing, so
  /// the test leaves nothing behind. `--spoil-key` puts a blob the chip will not take in the named item first.
  static func choose() -> Self {
    #if DEBUG
      let arguments = ProcessInfo.processInfo.arguments
      if let flag = arguments.firstIndex(of: "--fake") {
        let named = arguments.indices.contains(flag + 1) ? arguments[flag + 1] : FakeCore.Scenario.fresh.rawValue
        guard let scenario = FakeCore.Scenario(rawValue: named) else {
          let known = FakeCore.Scenario.allCases.map(\.rawValue).joined(separator: ", ")
          return .failed("No fake scenario is called \"\(named)\"; there are \(known).")
        }
        return .core(FakeCore(scenario))
      }
      let environment = ProcessInfo.processInfo.environment
      let name = environment["SHAMAN_CORE_DIRECTORY"] ?? CoreDirectory.defaultName
      let ownItem = environment["SHAMAN_KEY_ITEM"].map(KeyItem.init(account:))
      if arguments.contains("--remove-core-directory") {
        do {
          try CoreDirectory.remove(named: name)
          try ownItem?.remove()
        } catch {
          return .failed("The core's directory \"\(name)\" could not be removed: \(error.localizedDescription)")
        }
        return .removed(name)
      }
      if arguments.contains("--spoil-key"), let ownItem {
        try? ownItem.remove()
        try? ownItem.add(Data("not a key".utf8))
      }
      let item = ownItem ?? .identity
    #else
      let name = CoreDirectory.defaultName
      let item = KeyItem.identity
    #endif
    let directory: URL
    do {
      directory = try CoreDirectory.prepare(named: name)
    } catch {
      return .failed("This phone's records could not be kept: \(error.localizedDescription)")
    }
    return open(directory, IdentityKey.find(in: item), item)
  }

  /// The live core in `directory`, over the key `found`, kept in `item`.
  static func open(_ directory: URL, _ found: IdentityKey.Found, _ item: KeyItem) -> Self {
    switch Opening.of(found, in: directory) {
    case .core(let core): .core(core)
    case .failed(let message): .failed(message)
    case .keyRefused(let message): .keyRefused(message, directory: directory, item: item)
    }
  }
}
