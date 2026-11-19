import Foundation

/// What the scene keeps in Swift's scene storage of where the reader was, so that a launch after iOS ended the app
/// returns there, fetched afresh: the scope the listing stood at, and the Sessions pushed over it, a Subagent's over its
/// parent included, or the new-session screen. Sheets and the screens of a Turn's work and an Activity are not kept.
/// Every reference is the plain string the core gave, handed back unread: the core answers that a Session or Remote is
/// gone, and Everywhere for a scope that is. A launch into a new scene, whose storage holds nothing, starts at the
/// listing under Everywhere. A launch after the user swipes the app away is one where iOS gives the scene up on that
/// swipe; the iOS 27.2 simulator keeps it.
struct KeptScene: Codable, Equatable, Sendable {
  /// A screen pushed over the listing, as the scene keeps it.
  enum Screen: Codable, Equatable, Sendable {
    /// A Session's screen, by the reference the core gave.
    case session(String)
    /// The new-session screen, before its first Prompt.
    case landing
  }

  /// The scope the listing last stood at, as the core said it, or none before it has said.
  var scope: String?
  /// The screens pushed over the listing, the one on top last.
  var pushed: [Screen] = []

  init(scope: String? = nil, pushed: [Screen] = []) {
    self.scope = scope
    self.pushed = pushed
  }

  /// What `stored` keeps, or nothing at all where it keeps nothing this build reads, as when the scene is new.
  init(stored: String) {
    self = (try? JSONDecoder().decode(Self.self, from: Data(stored.utf8))) ?? Self()
  }

  /// The plain string the scene's storage keeps.
  var stored: String {
    guard let encoded = try? JSONEncoder().encode(self) else { return "" }
    return String(decoding: encoded, as: UTF8.self)
  }
}
