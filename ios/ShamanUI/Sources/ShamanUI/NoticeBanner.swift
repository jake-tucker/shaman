import ShamanCore
import SwiftUI

/// What the core says of its own accord, as the banner beneath a screen's bar shows it, once in a window: each notice
/// until it is tapped away or has stood on screen a while, when the core is told it was seen, so it shows the next and
/// never says that one again. Its while runs only while a banner shows it, so a notice hidden beneath a sheet, or while
/// no screen shows it, is never seen for it.
@MainActor @Observable
final class NoticeModel {
  private let core: any WindowCore
  private(set) var notice: Notice?
  /// How long a notice stands on screen before it goes by itself.
  private let stands: Duration
  /// How many banners show the notice now.
  private(set) var showing = 0
  /// How long the notice has stood on screen before the banner showing it last went.
  private var stood = Duration.zero
  /// When a banner last began showing it, while one does.
  private var since: ContinuousClock.Instant?
  /// What takes the notice away once it has stood its while on screen.
  private var clearing: Task<Void, Never>?

  init(core: any WindowCore, stands: Duration = .seconds(8)) {
    self.core = core
    self.stands = stands
  }

  /// Follows what the core says until the screen goes.
  func follow() async {
    for await notice in core.notices() {
      show(notice)
    }
  }

  /// Takes the notice away, telling the core it was seen.
  func dismiss() {
    clearing?.cancel()
    clearing = nil
    guard let notice else { return }
    self.notice = nil
    core.noticeSeen(notice.id)
  }

  /// A banner began showing the notice.
  func appeared() {
    showing += 1
    guard showing == 1, notice != nil else { return }
    since = .now
    wait()
  }

  /// A banner showing the notice went.
  func disappeared() {
    showing = max(0, showing - 1)
    guard showing == 0, let since else { return }
    stood += ContinuousClock.now - since
    self.since = nil
    clearing?.cancel()
    clearing = nil
  }

  private func show(_ next: Notice?) {
    guard next?.id != notice?.id else { return }
    clearing?.cancel()
    clearing = nil
    notice = next
    stood = .zero
    since = showing > 0 && next != nil ? .now : nil
    if since != nil {
      wait()
    }
  }

  /// Takes the notice away once it has stood the rest of its while on screen.
  private func wait() {
    guard let notice else { return }
    let left = stands - stood
    clearing?.cancel()
    clearing = Task { [weak self] in
      try? await Task.sleep(for: left)
      guard !Task.isCancelled, let self, self.notice?.id == notice.id else { return }
      self.dismiss()
    }
  }
}

/// Which screen of a window's columns says a notice, as `Columns` says they stand, so that it is said once.
enum NoticeSaid {
  /// One column stands alone, and whichever screen stands in it says it: the listing, or what is open over it.
  case oneColumn
  /// The listing, which stands over the main view.
  case listing
  /// The main view, the listing standing beside it or put away.
  case mainView
}

extension EnvironmentValues {
  /// Whether the screen says a notice, as the columns hosting it say. A sheet says its own, as one of the listing's
  /// does where the main view would otherwise; the Aside never does.
  @Entry var saysNotices = true
}

/// Shows what the core says of its own accord beneath the bar of the screen it modifies, in the banner an act's
/// failure is shown in, quietly, where the screen is not `covered` by a sheet of its own and is the one in the window
/// to say it.
private struct NoticeBanner: ViewModifier {
  @Environment(NoticeModel.self) private var notices: NoticeModel?
  @Environment(\.saysNotices) private var says
  let covered: Bool

  func body(content: Content) -> some View {
    content.safeAreaInset(edge: .top) {
      if let notices, let notice = notices.notice, !covered, says {
        Banner(text: notice.words, quiet: true)
          .padding(.vertical, 4)
          .onTapGesture { notices.dismiss() }
          .accessibilityAddTraits(.isButton)
          .accessibilityHint("Dismisses this")
          .accessibilityIdentifier("notice")
          .onAppear { notices.appeared() }
          .onDisappear { notices.disappeared() }
      }
    }
  }
}

extension View {
  /// Shows what the core says of its own accord beneath this screen's bar, where it is the screen in the window to say
  /// it, but while a sheet of its own covers it.
  func noticeBanner(covered: Bool = false) -> some View {
    modifier(NoticeBanner(covered: covered))
  }
}
