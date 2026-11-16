import AuthenticationServices
import UIKit

/// The system's authentication sheet, opened on a device login's page and closed by the app when the Relay says
/// the login is done. A device login redirects nowhere, so the sheet never completes by itself.
@MainActor
final class LoginSheet: NSObject, ASWebAuthenticationPresentationContextProviding {
    private var session: ASWebAuthenticationSession?

    func open(_ url: URL, log: SpikeLog) {
        let session = ASWebAuthenticationSession(url: url, callback: .customScheme("shaman-spike")) { _, error in
            log.line("sheet: completed with \(String(describing: error))")
        }
        session.presentationContextProvider = self
        session.prefersEphemeralWebBrowserSession = false
        let started = session.start()
        log.line("sheet: start gave \(started) for \(url.absoluteString)")
        self.session = session
    }

    func close() {
        session?.cancel()
        session = nil
    }

    func presentationAnchor(for session: ASWebAuthenticationSession) -> ASPresentationAnchor {
        let scenes = UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }
        return scenes.flatMap(\.windows).first { $0.isKeyWindow } ?? ASPresentationAnchor()
    }
}
