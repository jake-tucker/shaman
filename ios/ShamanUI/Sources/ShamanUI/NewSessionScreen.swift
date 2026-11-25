import ShamanCore
import SwiftUI

/// The new-session screen, which the listing's compose button pushes: the Landing, shaped as an empty open Session.
/// The navigation bar reads "New session" over where it will begin, the middle names the Workspace, and the composer
/// at its foot is up with the keyboard on arrival, but where the display is short, as the keyboard would cover the
/// middle. A first Prompt turns this same screen into the Session: the Provisional Session at once, and the real one in
/// its place once its Server answers, back still leading to the listing. "Begin somewhere else" pushes the Workspace
/// Picker over it, and where the Workspace is a Repository the Worktree choice offers its main Worktree, chosen as the
/// screen opens, or a new one. Where nothing can begin, as on an Unreachable Remote, a bare Repository or where no
/// Workspace is remembered, the composer takes a draft and send is disabled.
struct NewSessionScreen: View {
  let model: NewSessionModel
  let stack: SessionStack
  /// Whether the reader arrives writing, as one beginning a Session does; but not at rest, while nothing is open.
  var writing = true

  var body: some View {
    if let begun = model.begun {
      SessionScreen(model: begun, stack: stack)
    } else {
      LandingView(model: model, arrivesWriting: writing)
    }
  }
}

/// The Landing as the screen draws it before the send.
private struct LandingView: View {
  @Bindable var model: NewSessionModel
  let arrivesWriting: Bool
  @FocusState private var writing: Bool
  @Environment(\.pushScreen) private var pushScreen
  @Environment(\.verticalSizeClass) private var height

  var body: some View {
    ScrollView {
      if let landing = model.landing {
        LandingMiddle(landing: landing, model: model) {
          // The Workspace Picker, pushed over the screen, which the path does not name.
          pushScreen(WorkspacePickerScreen(model: model.picker()).noticeBanner())
        }
        .padding(.horizontal, 24)
        .padding(.vertical, 28)
        .frame(maxWidth: .infinity)
      }
    }
    .scrollDismissesKeyboard(.interactively)
    .accessibilityIdentifier("landing.screen")
    .safeAreaInset(edge: .bottom, spacing: 0) {
      if let landing = model.landing {
        LandingFoot(landing: landing, model: model, writing: $writing)
      }
    }
    .safeAreaInset(edge: .top) {
      if let banner = model.banner {
        Banner(text: banner)
          .padding(.vertical, 4)
          .background(Color(.systemBackground))
          .onTapGesture { model.dismissBanner() }
          .accessibilityAddTraits(.isButton)
          .accessibilityHint("Dismisses this")
      }
    }
    .navigationBarTitleDisplayMode(.inline)
    .toolbar {
      ToolbarItem(placement: .principal) {
        VStack(spacing: 1) {
          Text(verbatim: model.landing?.title ?? "")
            .font(.headline)
            .accessibilityIdentifier("landing.title")
          if let place = model.landing?.place, !place.isEmpty {
            Text(verbatim: place)
              .font(.caption)
              .foregroundStyle(.secondary)
              .lineLimit(1)
              .truncationMode(.middle)
              .accessibilityIdentifier("landing.place")
          }
        }
        .accessibilityElement(children: .contain)
      }
    }
    .task { await model.follow() }
    .onAppear {
      if arrivesWriting, height != .compact {
        writing = true
      }
    }
  }
}

/// The middle of the Landing: the Workspace the Session will begin in, by its Icon, name, Origin and Description, or,
/// where none is remembered for the Outlook's Remote, the line asking for one; the offer to begin somewhere else; the
/// Worktree choice, or why a bare Repository offers none; and the Agent a Session begun here would get, where its
/// Server says.
private struct LandingMiddle: View {
  let landing: Landing
  @Bindable var model: NewSessionModel
  let elsewhere: () -> Void

  var body: some View {
    VStack(spacing: 10) {
      if let workspace = landing.workspace {
        Group {
          if let glyph = workspace.glyph {
            Glyph(glyph: glyph, style: .largeTitle)
          } else {
            Image(systemName: "folder")
              .font(.largeTitle)
          }
        }
        .foregroundStyle(.secondary)
        .accessibilityHidden(true)
        Text(verbatim: workspace.name)
          .font(.title2.weight(.semibold))
          .accessibilityIdentifier("landing.workspace")
        Label {
          Text(verbatim: workspace.origin)
        } icon: {
          Image(systemName: "server.rack")
        }
        .font(.subheadline)
        .foregroundStyle(.secondary)
        .accessibilityElement(children: .combine)
        .accessibilityIdentifier("landing.origin")
        if let description = workspace.description {
          Text(verbatim: description)
            .font(.footnote)
            .foregroundStyle(.secondary)
            .multilineTextAlignment(.center)
            .fixedSize(horizontal: false, vertical: true)
            .accessibilityIdentifier("landing.description")
        }
      }
      if let asks = landing.asks {
        Image(systemName: "questionmark.folder")
          .font(.largeTitle)
          .foregroundStyle(.secondary)
          .accessibilityHidden(true)
        Text(verbatim: asks)
          .font(.callout)
          .foregroundStyle(.secondary)
          .multilineTextAlignment(.center)
          .fixedSize(horizontal: false, vertical: true)
          .accessibilityIdentifier("landing.asks")
      }
      if let offer = landing.elsewhere {
        Button(action: elsewhere) {
          Text(verbatim: offer)
            .font(.callout)
        }
        .buttonStyle(.bordered)
        .buttonBorderShape(.capsule)
        .padding(.top, 4)
        .accessibilityIdentifier("landing.elsewhere")
      }
      if let worktree = landing.worktree {
        Picker(selection: $model.newWorktree) {
          Text(verbatim: worktree.main)
            .tag(false)
            .accessibilityIdentifier("landing.worktree.main")
          Text(verbatim: worktree.new)
            .tag(true)
            .accessibilityIdentifier("landing.worktree.new")
        } label: {
          Text("Worktree")
        }
        .pickerStyle(.segmented)
        .padding(.top, 8)
        .accessibilityIdentifier("landing.worktree")
      }
      if let workingCopy = landing.workingCopy {
        Label {
          Text(verbatim: workingCopy)
        } icon: {
          Image(systemName: "exclamationmark.triangle")
        }
        .font(.footnote)
        .foregroundStyle(.secondary)
        .fixedSize(horizontal: false, vertical: true)
        .padding(.top, 8)
        .accessibilityElement(children: .combine)
        .accessibilityIdentifier("landing.workingCopy")
      }
      // Shown, not chosen: what the Server says a Session begun here would get.
      if let agent = landing.agent {
        Label {
          Text(verbatim: agent)
        } icon: {
          Image(systemName: "sparkle")
        }
        .font(.footnote)
        .foregroundStyle(.secondary)
        .padding(.top, 8)
        .accessibilityElement(children: .combine)
        .accessibilityIdentifier("landing.agent")
      }
    }
  }
}

/// The foot of the Landing: the line an Unreachable Remote leaves, and the composer, whose send begins the Session
/// where the Landing offers it.
private struct LandingFoot: View {
  let landing: Landing
  @Bindable var model: NewSessionModel
  var writing: FocusState<Bool>.Binding

  var body: some View {
    let empty = model.text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    VStack(alignment: .leading, spacing: 8) {
      if let line = landing.unreachable {
        Label {
          Text(verbatim: line)
        } icon: {
          Image(systemName: "wifi.slash")
        }
        .font(.caption)
        .foregroundStyle(.secondary)
        .frame(maxWidth: .infinity, alignment: .leading)
        .accessibilityElement(children: .combine)
        .accessibilityIdentifier("landing.unreachable")
      }
      HStack(alignment: .bottom, spacing: 8) {
        TextField(text: $model.text, axis: .vertical) {
          Text(verbatim: landing.placeholder)
        }
        .lineLimit(1...6)
        .focused(writing)
        .padding(.horizontal, 14)
        .padding(.vertical, 9)
        .background(Color(.secondarySystemBackground), in: RoundedRectangle(cornerRadius: 20))
        .accessibilityIdentifier("landing.composer")
        Button {
          model.send()
        } label: {
          Image(systemName: "arrow.up.circle.fill")
            .font(.system(size: 32))
        }
        .disabled(empty || !landing.canSend)
        .accessibilityLabel(Text("Send"))
        .accessibilityIdentifier("landing.send")
      }
      if let note = landing.note {
        Text(verbatim: note)
          .font(.caption)
          .foregroundStyle(.secondary)
          .fixedSize(horizontal: false, vertical: true)
          .accessibilityIdentifier("landing.note")
      }
      if let invalid = model.invalid {
        Text(verbatim: invalid)
          .font(.caption)
          .foregroundStyle(.red)
          .accessibilityIdentifier("landing.invalid")
      }
    }
    .padding(.horizontal, 12)
    .padding(.vertical, 8)
    .background(.bar)
  }
}

/// What the new-session screen holds while it is up: the Landing, the composer's own draft, and once a first Prompt
/// is sent, the Session it began, which the screen turns into.
@MainActor @Observable
final class NewSessionModel {
  private let core: any Core
  private(set) var landing: Landing?
  /// The Session the first Prompt began, which the screen shows from then on.
  private(set) var begun: SessionModel?
  /// What the composer's field holds, each change of which the core is handed as the Landing's draft.
  var text = "" {
    didSet {
      guard text != oldValue, let draft = landing?.draft else { return }
      invalid = nil
      core.setDraft(text, version: draftVersion, for: draft)
    }
  }
  @ObservationIgnored private var draftVersion: UInt64 = 0
  /// Whether the Session is to begin in a new Worktree rather than on the main one: the main one each time the screen
  /// opens, and again each time it turns to another Workspace.
  var newWorktree = false
  /// What the core refused of the input, shown beside it.
  private(set) var invalid: String?
  /// Why beginning failed otherwise, in the core's words, shown over the screen.
  private(set) var banner: String?
  /// Told of the Session begun, so what is open holds it as it holds a Session pushed.
  @ObservationIgnored var adopted: ((SessionModel) -> Void)?

  init(core: any Core) {
    self.core = core
  }

  /// Follows the Landing, and its draft, until the screen turns into the Session or goes.
  func follow() async {
    var drafts: Task<Void, Never>?
    defer { drafts?.cancel() }
    for await landing in core.landing() {
      let first = self.landing == nil
      if !Self.sameWorkspace(self.landing, landing) || landing.worktree == nil {
        newWorktree = false
      }
      self.landing = landing
      if first {
        take(core.draft(for: landing.draft))
        let key = landing.draft
        drafts = Task { [weak self] in await self?.followDraft(key) }
      }
    }
  }

  /// Whether `was` and `now` begin in the same Workspace, as the core tells one from another.
  private static func sameWorkspace(_ was: Landing?, _ now: Landing) -> Bool {
    was?.workspace?.id == now.workspace?.id
  }

  /// The Workspace Picker "Begin somewhere else" pushes.
  func picker() -> WorkspacePickerModel {
    WorkspacePickerModel(core: core)
  }

  private func followDraft(_ key: String) async {
    for await draft in core.drafts(for: key) {
      take(draft)
    }
  }

  /// Takes the draft the core holds into the field: what it gave back always wins over what is typed meanwhile.
  private func take(_ draft: Draft) {
    draftVersion = draft.version
    if text != draft.text {
      text = draft.text
    }
  }

  /// Begins the Session with what the composer holds, where the Landing offers send: the screen turns into it at once.
  func send() {
    guard let landing, landing.canSend else { return }
    let sent = text
    guard !sent.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { return }
    do {
      let opened = try core.beginSession(sent, newWorktree: newWorktree && landing.worktree != nil)
      text = ""
      let model = SessionModel(core: core, route: SessionRoute(session: opened.reference), opened: opened)
      adopted?(model)
      begun = model
    } catch {
      switch error {
      case .Invalid(let message): invalid = message
      case .Unreachable(let message), .Refused(let message), .Gone(let message): banner = message
      }
    }
  }

  /// Takes the banner away.
  func dismissBanner() {
    banner = nil
  }
}

#if DEBUG
  #Preview("Where the Outlook stands") {
    NavigationStack {
      NewSessionScreen(
        model: NewSessionModel(core: FakeCore(.composing)), stack: SessionStack(core: FakeCore(.composing)))
    }
  }

  #Preview("Where no workspace is remembered") {
    NavigationStack {
      NewSessionScreen(model: NewSessionModel(core: FakeCore(.paired)), stack: SessionStack(core: FakeCore(.paired)))
    }
  }
#endif
