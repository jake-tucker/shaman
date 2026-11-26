#if DEBUG
  import Foundation
  import ShamanCore
  import Synchronization

  /// A window's half of the fake, a Client of its own over a `FakeCore`, as `LiveWindow` is over the core: its own
  /// Outlook, listing, Landing, Workspace Picker's search and notices, over the fake's one phone.
  public final class FakeWindow: WindowCore {
    /// The fake phone this window is over.
    let fake: FakeCore
    /// Which window it is, of all the fake has made.
    private let id: Int
    public var device: any DeviceCore { fake }

    /// The Remotes in the order this window last turned toward, the latest first, and the Workspace it last turned
    /// toward on each: begun as the phone's were.
    private let turned: Mutex<[String]>
    private let turnedWorkspace: Mutex<[String: String]>
    /// What its listing is narrowed to and searched for, and how many of the shelf's rows are on show, while it lives.
    private let scope = Mutex(FakeScope.everywhere)
    private let query = Mutex("")
    private let onShow = Mutex(FakeSession.shelfOpening)
    /// What its Landing says of a beginning left while it may have begun.
    private let landingNote = Mutex<String?>(nil)
    /// What its Workspace Picker is searched for, until it is opened again.
    private let workspaceQuery = Mutex("")
    /// What the core says of its own accord in this window, until it is seen.
    private let notice: Mutex<Notice?>

    private let outlookListeners = Listeners<Outlook>()
    private let listingListeners = Listeners<Listing>()
    private let landingListeners = Listeners<Landing>()
    private let workspacesListeners = Listeners<WorkspacePicker>()
    private let noticeListeners = Listeners<Notice?>()

    /// Every Session this window has opened, the latest last, for a test to see each closed as its screen goes.
    let opened = Mutex<[FakeOpenedSession]>([])

    /// A window of the fake beginning as `scenario` says, over a fake phone of its own, as a test or a preview asks
    /// for one, the phone's relay confirming a login after `loginConfirmsAfter` and its drafts kept in the file
    /// `keepingDraftsIn` names, where either is given.
    public convenience init(
      _ scenario: FakeCore.Scenario, loginConfirmsAfter: Duration? = nil, keepingDraftsIn drafts: URL? = nil
    ) {
      self.init(over: FakeCore(scenario, loginConfirmsAfter: loginConfirmsAfter, keepingDraftsIn: drafts))
    }

    /// A window over `fake`, beginning where the phone was last turned, its listing at Everywhere, saying what the
    /// phone has to say.
    init(over fake: FakeCore) {
      self.fake = fake
      id = fake.nextWindow()
      let (turned, workspaces) = fake.lastTurned()
      self.turned = Mutex(turned)
      turnedWorkspace = Mutex(workspaces)
      notice = Mutex(fake.newer)
      fake.hold(self)
    }

    // MARK: The Outlook

    public func outlook() -> AsyncStream<Outlook> {
      outlookListeners.stream(now: currentOutlook())
    }

    /// Turns this window's Outlook toward the Remote `remote`, first among those it turned toward, and toward its
    /// Workspace `workspace` where one is given; the phone keeps it as its latest.
    private func turn(_ remote: String, workspace: String? = nil) {
      turned.withLock { turned in
        turned.removeAll { $0 == remote }
        turned.insert(remote, at: 0)
      }
      if let workspace {
        turnedWorkspace.withLock { $0[remote] = workspace }
      }
      fake.keepTurn(remote, workspace: turnedWorkspace.withLock { $0[remote] })
    }

    /// The Remote this window's Outlook stands at, where the phone holds any: the first it turned toward that is held,
    /// or else the phone's, or else the first paired, which it stands at from then on.
    func currentRemote() -> String? {
      let names = fake.heldNames()
      if let own = turned.withLock({ $0 }).first(where: names.contains) {
        return own
      }
      guard let remote = fake.lastTurned().0.first(where: names.contains) ?? names.first else { return nil }
      turned.withLock { $0.insert(remote, at: 0) }
      return remote
    }

    private func currentOutlook() -> Outlook {
      guard let name = currentRemote() else {
        return .nowhere
      }
      let shown = fake.shown(name)
      let place = turnedWorkspace.withLock { $0[name] }.map { "\(shown) · \($0)" } ?? shown
      return .remote(
        name: shown,
        beginsIn: BeginsIn(lead: "New sessions begin in", place: place, said: "New sessions begin in \(place)"))
    }

    /// Where this window's Outlook stands now.
    func outlookNow() -> Outlook {
      currentOutlook()
    }

    /// Tells the Outlook's, the Landing's and the Workspace Picker's listeners where the Outlook stands now.
    func turnedOutlook() {
      outlookListeners.tell(currentOutlook())
      landingListeners.tell(currentLanding())
      workspacesListeners.tell(currentWorkspaces())
    }

    /// Forgets whatever this window remembered of the Remote `name`, gone, turning toward `latest` where it is given, as
    /// it is to where this window stood at `name`: the Remote the phone was most recently turned toward that is still
    /// held.
    func forget(_ name: String, turningTo latest: String?) {
      turned.withLock { turned in
        turned.removeAll { $0 == name }
        if let latest {
          turned.removeAll { $0 == latest }
          turned.insert(latest, at: 0)
        }
      }
      _ = turnedWorkspace.withLock { $0.removeValue(forKey: name) }
    }

    // MARK: The listing

    public func listing() -> AsyncStream<Listing> {
      listingListeners.stream(now: currentListing())
    }

    public func showMoreSettled() {
      onShow.withLock { $0 += FakeSession.shelfBatch }
      tellListing()
    }

    public func narrowListing(_ reference: String) async throws(ActError) {
      let chosen = FakeScope(reference).within(fake.heldNames(), sessions: fake.listed())
      scope.withLock { $0 = chosen }
      if let remote = chosen.remote {
        if case .workspace(_, let workspace) = chosen {
          turn(remote, workspace: workspace)
        } else {
          turn(remote)
        }
        turnedOutlook()
      }
      tellListing()
    }

    public func restoreListing(_ reference: String) {
      scope.withLock { $0 = FakeScope(reference).restored(fake.heldNames(), sessions: fake.listed()) }
      tellListing()
    }

    public func searchListing(_ text: String) {
      query.withLock { $0 = text }
      tellListing()
    }

    /// Where the Outlook stands, as `<remote>-key`, followed by `/<workspace>` where it stands in one, as the core keeps
    /// the Remote by its key.
    public func keptOutlook() -> String? {
      guard let remote = currentRemote() else { return nil }
      return ["\(remote)-key", turnedWorkspace.withLock { $0[remote] }].compactMap(\.self).joined(separator: "/")
    }

    /// Turns this window's Outlook back to where `kept`, what its scene kept, says it stood, as it is made.
    func restoreOutlook(_ kept: String) {
      let parts = kept.split(separator: "/", maxSplits: 1).map(String.init)
      guard let key = parts.first, let remote = fake.heldNames().first(where: { "\($0)-key" == key }) else { return }
      // Turned in this window alone: the phone keeps nothing of it.
      turned.withLock { turned in
        turned.removeAll { $0 == remote }
        turned.insert(remote, at: 0)
      }
      turnedWorkspace.withLock { $0[remote] = parts.count > 1 ? parts[1] : nil }
      turnedOutlook()
    }

    /// Tells this window's listing's listeners what it shows now.
    func tellListing() {
      listingListeners.tell(currentListing())
    }

    /// The listing as this window narrows it.
    private func currentListing() -> Listing {
      fake.listing(showing: onShow.withLock { $0 }, scope: scope.withLock { $0 }, query: query.withLock { $0 })
    }

    // MARK: The Landing

    public func landing() -> AsyncStream<Landing> {
      landingListeners.stream(now: currentLanding())
    }

    /// Tells this window's Landing's listeners what it shows now.
    func tellLanding() {
      landingListeners.tell(currentLanding())
    }

    public func beginSession(_ text: String, newWorktree: Bool) throws(ActError) -> any OpenedSession {
      let text = text.trimmingCharacters(in: .whitespacesAndNewlines)
      guard !text.isEmpty else { throw .Invalid(message: "Write something to send") }
      let landing = currentLanding()
      guard let remote = currentRemote(), let workspace = landing.workspace else {
        throw .Invalid(message: landing.asks ?? "Pair with a server to begin a session there")
      }
      if let workingCopy = landing.workingCopy {
        throw .Invalid(message: workingCopy)
      }
      if newWorktree && landing.worktree == nil {
        throw .Invalid(message: "No new worktree can be made in this workspace")
      }
      let session = "\(remote)-key/begun-\(fake.nextBegun())"
      let place = ((fake.heldNames().count > 1 ? [remote] : []) + [workspace.name]).joined(separator: " · ")
      // A Session begun anew says nothing more of one left before.
      landingNote.withLock { $0 = nil }
      tellLanding()
      let opened = FakeOpenedSession(
        beginning: session, text: text, place: place, newWorktree: newWorktree, acts: fake.acts
      ) { [weak self] told in
        guard let self else { return }
        switch told {
        case .began(let text): fake.list(session, begunWith: text, on: remote, in: workspace.name)
        case .leftUnsure:
          landingNote.withLock {
            $0 =
              "The server did not answer, so a session may have begun with this prompt. Look for it in the listing "
              + "before sending it again."
          }
          tellLanding()
        }
      }
      self.opened.withLock { $0.append(opened) }
      return opened
    }

    /// The Landing where this window's Outlook stands, as the core would show it: naming the Remote only beside
    /// others.
    private func currentLanding() -> Landing {
      let note = landingNote.withLock { $0 }
      guard let remote = currentRemote() else {
        return Landing(
          title: "New session", place: "", workspace: nil, asks: "Pair with a server to begin a session there",
          elsewhere: nil, worktree: nil, workingCopy: nil, agent: nil, unreachable: nil, note: note,
          placeholder: "Message", canSend: false,
          draft: FakeOpenedSession.landingDraft)
      }
      return fake.landing(on: remote, in: turnedWorkspace.withLock { $0[remote] }, note: note)
    }

    // MARK: The Workspace Picker

    public func workspaces() -> AsyncStream<WorkspacePicker> {
      workspaceQuery.withLock { $0 = "" }
      return workspacesListeners.stream(now: currentWorkspaces())
    }

    public func searchWorkspaces(_ text: String) {
      workspaceQuery.withLock { $0 = text }
      workspacesListeners.tell(currentWorkspaces())
    }

    /// Turns this window's Outlook toward the Workspace a picker row carries, after a moment, leaving the listing's
    /// scope.
    public func chooseWorkspace(_ workspace: String) async throws(ActError) {
      try? await Task.sleep(for: .milliseconds(150))
      guard let (remote, name) = FakeWorkspace.named(by: workspace), let listed = FakeWorkspace.find(remote, name),
        fake.heldNames().contains(remote)
      else {
        throw .Gone(message: "That workspace is no longer listed")
      }
      if let unavailable = listed.unavailable {
        throw .Invalid(message: unavailable)
      }
      turn(remote, workspace: name)
      turnedOutlook()
    }

    /// The Workspace Picker as the core would show it now in this window.
    private func currentWorkspaces() -> WorkspacePicker {
      let remote = currentRemote()
      return fake.picker(
        chosen: remote.flatMap { remote in turnedWorkspace.withLock { $0[remote] }.map { (remote, $0) } },
        query: workspaceQuery.withLock { $0 })
    }

    /// Tells this window's Workspace Picker's listeners what it shows now.
    func tellWorkspaces() {
      workspacesListeners.tell(currentWorkspaces())
    }

    // MARK: Sessions

    public func openSession(_ session: String) -> any OpenedSession {
      let (opened, found) = fake.open(session)
      if let found {
        // The Workspace remembered for the Remote stays; only where none is does the Session's own become it.
        turn(found.remote, workspace: turnedWorkspace.withLock { $0[found.remote] } ?? found.workspace)
        turnedOutlook()
      }
      self.opened.withLock { $0.append(opened) }
      return opened
    }

    /// Ends every Session this window has open on the Remote `name` names, which is held no more.
    func endSessions(on name: String) {
      for session in opened.withLock({ $0 }) where session.reference.hasPrefix("\(name)-key/") {
        session.end("This phone is no longer paired with the server this session is on")
      }
    }

    // MARK: Drafts, which are the phone's

    public func draft(for session: String) -> Draft {
      fake.acts.draft(for: session, in: id)
    }

    public func setDraft(_ text: String, version: UInt64, for session: String) {
      fake.acts.setDraft(text, version: version, for: session, in: id)
    }

    public func drafts(for session: String) -> AsyncStream<Draft> {
      fake.acts.drafts(for: session, in: id)
    }

    // MARK: Notices

    /// Followed, the window stands, and says what was said as no window stood.
    public func notices() -> AsyncStream<Notice?> {
      for words in fake.takeUnclaimed() {
        say(words)
      }
      return noticeListeners.stream(now: notice.withLock { $0 })
    }

    /// Says the notice no more in this window, as the core never says it again here.
    public func noticeSeen(_ id: UInt64) {
      let seen = notice.withLock { notice in
        guard notice?.id == id else { return false }
        notice = nil
        return true
      }
      if seen {
        noticeListeners.tell(nil)
      }
    }

    /// Says `words` in this window's notice banner, until they are seen.
    func say(_ words: String) {
      let said = notice.withLock { notice in
        let next = Notice(id: (notice?.id ?? 0) + 100, words: words)
        notice = next
        return next
      }
      noticeListeners.tell(said)
    }

    // MARK: Pairing

    public func confirmPairing(invite: String, remoteName: String, deviceName: String) async -> PairingOutcome {
      let outcome = await fake.pair(invite: invite, remoteName: remoteName, deviceName: deviceName)
      if case .paired(let name) = outcome {
        turn(name)
        fake.pairedTurned()
      }
      return outcome
    }
  }
#endif
