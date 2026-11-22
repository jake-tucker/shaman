import AuthenticationServices
import ShamanCore
import SwiftUI
import UIKit

/// What opens the address a login's code is entered at in a sheet of the system's inside the app, and closes it.
@MainActor
public protocol LoginBrowser: AnyObject {
  /// Opens `url`, closing whatever was opened before.
  func open(_ url: URL)

  /// Closes what was opened, where it is still open.
  func close()
}

/// The system's authentication sheet: GitHub opened inside the app, so the app is never left while a login waits on a
/// connection a suspended app would lose. Nothing calls back to it; it is closed once the Relay confirms the login, and
/// its user may close it sooner and approve the code from another device.
@MainActor
public final class WebLoginBrowser: NSObject, LoginBrowser {
  private var session: ASWebAuthenticationSession?

  public func open(_ url: URL) {
    close()
    // The scheme is never answered: the login ends at the Relay, which the core hears of.
    let session = ASWebAuthenticationSession(url: url, callback: .customScheme("shaman-login")) { _, _ in }
    session.presentationContextProvider = self
    self.session = session
    session.start()
  }

  public func close() {
    session?.cancel()
    session = nil
  }
}

extension WebLoginBrowser: ASWebAuthenticationPresentationContextProviding {
  public nonisolated func presentationAnchor(for session: ASWebAuthenticationSession) -> ASPresentationAnchor {
    MainActor.assumeIsolated {
      let scenes = UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }
      let windows = scenes.flatMap(\.windows)
      if let key = windows.first(where: \.isKeyWindow) ?? windows.first {
        return key
      }
      // The app shows a sheet only from a window, so there is one by the time this is asked.
      return UIWindow(windowScene: scenes[0])
    }
  }
}

/// A login at a Relay a sheet has been asked to carry out, for `sheet(item:)`.
struct LoginAsk: Identifiable, Hashable {
  let relay: String
  var id: String { relay }
}

/// The sheet a login at a Relay is carried out in: the code, copied and with GitHub opened in the system's sheet for it
/// to be pasted, standing with its address for approving from another device, until the Relay confirms the login,
/// when the sheet closes itself. Dismissing it gives the login up, as leaving the app does.
struct RelayLoginSheet: View {
  @State private var model: RelayLoginModel
  /// Told once the Relay confirmed the login, as the sheet closes itself.
  private let loggedIn: () -> Void
  @Environment(\.dismiss) private var dismiss
  /// Asks the core for a login again, after one failed.
  @State private var attempt = 0

  init(model: RelayLoginModel, loggedIn: @escaping () -> Void) {
    _model = State(initialValue: model)
    self.loggedIn = loggedIn
  }

  var body: some View {
    NavigationStack {
      Form {
        switch model.login {
        case nil:
          ProgressView()
        case .starting(_, let words):
          Section {
            HStack(spacing: 12) {
              ProgressView()
              Text(verbatim: words).foregroundStyle(.secondary)
            }
            .accessibilityElement(children: .combine)
            .accessibilityIdentifier("login.starting")
          }
        case .waiting(_, let code, let address, let words, let copy, let open):
          waiting(code: code, address: address, words: words, copy: copy, open: open)
        case .done(_, let words):
          Section {
            Label(words, systemImage: "checkmark.circle.fill")
              .foregroundStyle(.green)
              .accessibilityIdentifier("login.done")
          }
        case .failed(_, let message, let again):
          Section {
            Text(verbatim: message)
              .foregroundStyle(.red)
              .accessibilityIdentifier("login.failed")
            Button(again) { attempt += 1 }
              .accessibilityIdentifier("login.again")
          }
        }
      }
      .navigationTitle(model.title)
      .navigationBarTitleDisplayMode(.inline)
      .protoSheetStack()
      .toolbar {
        ToolbarItem(placement: .cancellationAction) {
          Button {
            dismiss()
          } label: {
            if ProtoS.shared.symbols {
              Label("Cancel", systemImage: "xmark")
            } else {
              Text("Cancel")
            }
          }
          .accessibilityIdentifier("login.cancel")
        }
      }
    }
    .protoSheet("login")
    .task(id: attempt) {
      if await model.follow() {
        loggedIn()
        dismiss()
      }
    }
  }

  private func waiting(code: String, address: String, words: String, copy: String, open: String) -> some View {
    Group {
      Section {
        VStack(spacing: 10) {
          Text(verbatim: code)
            .font(.system(.largeTitle, design: .monospaced).bold())
            .textSelection(.enabled)
            .frame(maxWidth: .infinity)
            .accessibilityIdentifier("login.code")
          if model.copied {
            Label("Copied", systemImage: "doc.on.doc")
              .font(.footnote)
              .foregroundStyle(.secondary)
              .accessibilityIdentifier("login.copied")
          }
        }
        .padding(.vertical, 8)
        Text(verbatim: words)
          .foregroundStyle(.secondary)
          .accessibilityIdentifier("login.words")
        Text(verbatim: address)
          .font(.callout.monospaced())
          .textSelection(.enabled)
          .accessibilityIdentifier("login.address")
      }
      Section {
        Button(copy, systemImage: "doc.on.doc") { model.copyAgain() }
          .accessibilityIdentifier("login.copy")
        Button(open, systemImage: "safari") { model.openAgain() }
          .accessibilityIdentifier("login.open")
      }
    }
  }
}

/// What the sheet a login is carried out in holds while it is up.
@MainActor @Observable
final class RelayLoginModel {
  private let core: any Core
  let relay: String
  private let browser: any LoginBrowser
  private let copy: @MainActor (String) -> Void
  /// How the login stands, as the core last said.
  private(set) var login: DeviceLogin?
  /// Whether the code shown has been copied.
  private(set) var copied = false
  /// The code whose address was opened, so each code is copied and opened once by itself.
  private var opened: String?

  /// The login at `relay` over `core`, opening the address its code is entered at in `browser` and copying the code
  /// by `copy`.
  init(
    core: any Core, relay: String, browser: any LoginBrowser,
    copy: @escaping @MainActor (String) -> Void = { UIPasteboard.general.string = $0 }
  ) {
    self.core = core
    self.relay = relay
    self.browser = browser
    self.copy = copy
  }

  /// The sheet's title, in the core's words once it has said them.
  var title: String {
    switch login {
    case .starting(let title, _), .waiting(let title, _, _, _, _, _), .done(let title, _), .failed(let title, _, _):
      title
    case nil: "Log in"
    }
  }

  /// Follows a login at the Relay until it ends, or until what follows it is cancelled, which gives it up: answers
  /// whether the Relay confirmed it. The first code shown is copied, and its address opened, by itself; the system's
  /// sheet closes as the login ends.
  func follow() async -> Bool {
    copied = false
    opened = nil
    defer { browser.close() }
    for await login in core.logIn(at: relay) {
      self.login = login
      switch login {
      case .waiting(_, let code, let address, _, _, _):
        if opened != code {
          opened = code
          copy(code)
          copied = true
          if let url = URL(string: address) {
            browser.open(url)
          }
        }
      case .done:
        return true
      case .failed:
        return false
      case .starting:
        break
      }
    }
    return false
  }

  /// Copies the code shown again.
  func copyAgain() {
    guard case .waiting(_, let code, _, _, _, _) = login else { return }
    copy(code)
    copied = true
  }

  /// Opens the address the code is entered at again.
  func openAgain() {
    guard case .waiting(_, _, let address, _, _, _) = login, let url = URL(string: address) else { return }
    browser.open(url)
  }
}

extension View {
  /// Presents the sheet a login at the Relay `ask` names is carried out in while it is set, telling `loggedIn` once the
  /// Relay confirms it.
  func relayLogin(_ ask: Binding<LoginAsk?>, core: any Core, loggedIn: @escaping (String) -> Void) -> some View {
    sheet(item: ask) { ask in
      LoginSheetHost(core: core, relay: ask.relay) { loggedIn(ask.relay) }
    }
  }
}

/// The sheet a login is carried out in, over the browser the environment makes.
private struct LoginSheetHost: View {
  let core: any Core
  let relay: String
  let loggedIn: () -> Void
  @Environment(\.loginBrowser) private var browser

  var body: some View {
    RelayLoginSheet(model: RelayLoginModel(core: core, relay: relay, browser: browser()), loggedIn: loggedIn)
  }
}

/// What makes the browser a login opens its address in: the system's authentication sheet, unless something stands in.
private struct LoginBrowserKey: EnvironmentKey {
  static let defaultValue: @MainActor @Sendable () -> any LoginBrowser = { WebLoginBrowser() }
}

extension EnvironmentValues {
  /// What makes the browser a login opens its address in.
  var loginBrowser: @MainActor @Sendable () -> any LoginBrowser {
    get { self[LoginBrowserKey.self] }
    set { self[LoginBrowserKey.self] = newValue }
  }
}

#if DEBUG
  #Preview("Logging in") {
    RelayLoginSheet(
      model: RelayLoginModel(core: FakeCore(.relayLogin), relay: FakeCore.relay, browser: WebLoginBrowser())
    ) {}
  }
#endif
