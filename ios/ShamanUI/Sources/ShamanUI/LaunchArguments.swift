/// What a debug build is asked at launch to show, which the app's first window shows alone: the process has one set
/// of launch arguments and launch environment however many windows it makes, and each window's root is handed the
/// same, so the first root whose first task runs claims it and every window after shows nothing of it.
@MainActor public final class LaunchArguments {
  /// What one window is asked to show at launch.
  struct Asked: Equatable {
    /// The Invite the first screen shown confirms at once, as one arriving by `SHAMAN_INVITE` is.
    var invite: String?
    /// The Session pushed over the listing at once, as `--open` names it.
    var opening: String?
    /// Whether the new-session screen is pushed over the listing at once, as `--compose` asks.
    var composing = false
    /// The kind of thing the Aside is shown at beside that Session, as `--aside` asks.
    var aside: AsideKind?
  }

  private var asked: Asked?

  /// Launch arguments asking the first window to confirm `invite` where one is given, and to push the Session
  /// `opening` names over the listing, with the Aside shown beside it at `aside`'s kind where one is given, or the
  /// new-session screen where it is `composing`. Asking nothing, as a release build does, they show nothing.
  public init(invite: String? = nil, opening: String? = nil, composing: Bool = false, aside: AsideKind? = nil) {
    asked = Asked(invite: invite, opening: opening, composing: composing, aside: aside)
  }

  /// What the window asking is to show at launch: all of it for the first to ask, which a window's root does as its
  /// first task runs, and nothing for any window after.
  func claimed() -> Asked {
    defer { asked = nil }
    return asked ?? Asked()
  }
}
