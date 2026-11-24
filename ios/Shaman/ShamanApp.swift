import Foundation
import ShamanUI
import SwiftUI

@main
struct ShamanApp: App {
  @State private var opened: Opened
  /// What is followed once for the app, however many windows it shows.
  @State private var follower: AppFollower

  init() {
    let follower = AppFollower()
    let opened = Opened.choose()
    opened.followed(by: follower)
    _follower = State(initialValue: follower)
    _opened = State(initialValue: opened)
  }

  var body: some Scene {
    WindowGroup {
      switch opened {
      case .core(let core):
        #if DEBUG
          Group {
            if Launch.serversAndDevice {
              DeviceScreen(core: core, appVersion: Self.version)
            } else {
              RootScreen(
                core: core, appVersion: Self.version, invite: Launch.invite, opening: Launch.opening,
                composing: Launch.composing, forgetting: Launch.forgettingScene,
                noticeStands: Launch.noticeStands ?? .seconds(8)
              )
              .environment(\.interventionArming, Launch.arming ?? EnvironmentValues().interventionArming)
            }
          }
          .environment(\.codeScanner, Launch.scanner ?? .camera)
        #else
          RootScreen(core: core, appVersion: Self.version)
        #endif
      case .failed(let message): CoreFailedScreen(message: message)
      case .keyRefused(let message, let directory, let item):
        KeyRefusedScreen(message: message) {
          opened = Opened.open(directory, IdentityKey.startAgain(in: item), item)
          opened.followed(by: follower)
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

    /// The Session `--open <reference>` asks to be pushed over the listing at launch, so a screenshot or a UI test
    /// reaches it without a tap: over the fake, `workstation-key/1` is the working one, 7 the settled one, 8 the
    /// stopped one and 9 the failed one.
    static var opening: String? {
      let arguments = ProcessInfo.processInfo.arguments
      guard let flag = arguments.firstIndex(of: "--open"), arguments.indices.contains(flag + 1) else { return nil }
      return arguments[flag + 1]
    }

    /// How long `--arming-moment <seconds>` asks an Intervention presented to take no tap for, so a UI test can see
    /// it refuse one before it takes one: up to an hour, and the usual moment for anything else, NaN, an infinity or
    /// a negative included.
    static var arming: Duration? {
      let arguments = ProcessInfo.processInfo.arguments
      guard let flag = arguments.firstIndex(of: "--arming-moment"), arguments.indices.contains(flag + 1),
        let seconds = Double(arguments[flag + 1]), (0...3_600).contains(seconds)
      else { return nil }
      return .milliseconds(Int(seconds * 1_000))
    }

    /// How long `--notice-stands <seconds>` asks a notice to stand on screen before it goes by itself, so a UI test
    /// can see it outlast a sheet over it and then go: up to an hour, and the usual while for anything else.
    static var noticeStands: Duration? {
      let arguments = ProcessInfo.processInfo.arguments
      guard let flag = arguments.firstIndex(of: "--notice-stands"), arguments.indices.contains(flag + 1),
        let seconds = Double(arguments[flag + 1]), (0...3_600).contains(seconds)
      else { return nil }
      return .milliseconds(Int(seconds * 1_000))
    }

    /// Whether `--compose` asks for the new-session screen to be pushed over the listing at launch, as its compose
    /// button pushes it.
    static var composing: Bool {
      ProcessInfo.processInfo.arguments.contains("--compose")
    }

    /// Whether `--forget-scene` asks for what the scene kept to be forgotten, so the listing opens as in a new scene
    /// and the scene keeps afresh from there, as a UI test that left a Session open asks to leave nothing behind.
    static var forgettingScene: Bool {
      ProcessInfo.processInfo.arguments.contains("--forget-scene")
    }

    /// The fake scanner `--scan <code>`, given once for each code, and `--camera <granted|refused|unavailable|fails>`
    /// ask for in place of the camera, which the simulator has none of: one that finds the camera as `--camera` says,
    /// granted where it says nothing or that it fails, which it does as it starts, and holds each code up in turn while
    /// it scans, a second apart, or as many seconds as `--scan-every <seconds>` says, up to a minute, so a UI test can
    /// see what it shows between two.
    static var scanner: CodeScanner? {
      let arguments = ProcessInfo.processInfo.arguments
      let codes = arguments.indices.dropLast().filter { arguments[$0] == "--scan" }.map { arguments[$0 + 1] }
      let camera = arguments.firstIndex(of: "--camera").flatMap { arguments.dropFirst($0 + 1).first }
      let access: ScanAccess? =
        switch camera {
        case "granted", "fails": .granted
        case "refused": .refused
        case "unavailable": .unavailable
        default: nil
        }
      guard !codes.isEmpty || access != nil else { return nil }
      let every = arguments.firstIndex(of: "--scan-every").flatMap { arguments.dropFirst($0 + 1).first }
        .flatMap(Double.init).flatMap { (0...60).contains($0) ? Duration.milliseconds(Int($0 * 1_000)) : nil }
      return .reading(codes, access: access ?? .granted, every: every ?? .seconds(1), failing: camera == "fails")
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
  /// `--fake <scenario>` chooses the fake instead, which keeps its drafts in the directory `SHAMAN_CORE_DIRECTORY`
  /// names where it names one, and the environment's `SHAMAN_CORE_DIRECTORY` names a directory of
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
        // Where a directory is named, the fake keeps its drafts there from one run to the next, as the core does.
        let kept = ProcessInfo.processInfo.environment["SHAMAN_CORE_DIRECTORY"].flatMap {
          try? CoreDirectory.prepare(named: $0).appending(path: "drafts.json")
        }
        return .core(FakeCore(scenario, keepingDraftsIn: kept))
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

  /// Has `follower` follow for the core, where there is one.
  @MainActor func followed(by follower: AppFollower) {
    if case .core(let core) = self {
      follower.follow(core)
    }
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
