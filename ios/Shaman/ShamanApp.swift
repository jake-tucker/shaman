import Foundation
import ShamanUI
import SwiftUI

@main
struct ShamanApp: App {
  private let opened = Opened.choose()

  var body: some Scene {
    WindowGroup {
      switch opened {
      case .core(let core): DeviceScreen(core: core, appVersion: Self.version)
      case .failed(let message): CoreFailedScreen(message: message)
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

/// The core the app runs over, or why there is none.
private enum Opened {
  case core(any Core)
  case failed(String)

  /// The live core, kept in its directory. In debug builds `--fake <scenario>` chooses the fake instead, and the
  /// environment's `SHAMAN_CORE_DIRECTORY` names a directory of its own for the live core, so a UI test starts afresh.
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
      let name = ProcessInfo.processInfo.environment["SHAMAN_CORE_DIRECTORY"] ?? CoreDirectory.defaultName
    #else
      let name = CoreDirectory.defaultName
    #endif
    let directory: URL
    do {
      directory = try CoreDirectory.prepare(named: name)
    } catch {
      return .failed("This phone's records could not be kept: \(error.localizedDescription)")
    }
    do {
      return .core(try LiveCore(directory: directory))
    } catch {
      return .failed(error.message)
    }
  }
}
