import ShamanCore

/// Everything the app asks of the thin Client's core: the one way it reaches it. `LiveCore` is the core itself; in
/// debug builds `FakeCore` stands in with canned values, chosen by a launch argument.
///
/// What a screen shows arrives as a finished model, whole on every change, beginning with what it is now. An act
/// returns once it is done, with nothing but success; what it changed arrives by the model.
public protocol Core: AnyObject, Sendable {
  /// What "This device" shows, now and on every change.
  func thisDevice() -> AsyncStream<ThisDevice>

  /// Renames this device, or fails with why, in the core's words.
  func renameDevice(_ name: String) async throws(ActError)

  /// Says no more that this phone must pair again.
  func dismissPairAgain() async throws(ActError)

  /// Where the Outlook stands, now and on every change: nowhere while the phone holds no Remote.
  func outlook() -> AsyncStream<Outlook>

  /// The Servers section of the sheet of Servers, Relays and This device, now and on every change.
  func servers() -> AsyncStream<Servers>

  /// Every Remote's Sessions in one listing, active above the settled, now and on every change.
  func listing() -> AsyncStream<Listing>

  /// Sets the Session a row names aside as done for now, once its Server has answered.
  func settle(_ session: String) async throws(ActError)

  /// Brings the Session a settled row names back among the active, once its Server has answered.
  func unsettle(_ session: String) async throws(ActError)

  /// Brings up the next of the settled shelf's rows.
  func showMoreSettled()

  /// Opens the Session a row names for a screen pushed to read it, framed at once by what the listing knows of it,
  /// and turns the Outlook toward its Origin and Workspace. Closing it cancels nothing on the Server.
  func openSession(_ session: String) -> any OpenedSession

  /// What the Landing shows, where the Outlook stands, now and on every change: the screen a Session is begun from.
  func landing() -> AsyncStream<Landing>

  /// Begins a Session with the Prompt `text` where the Outlook stands, on its Workspace's main Worktree or, where
  /// `newWorktree`, in a new one its Server makes first, under an identity the core chooses, and opens it at once as the
  /// Provisional Session the Landing's screen turns into: its Server's answer, the Session made or why it was refused,
  /// arrives by the frame and the conversation. The beginning goes on whatever becomes of the screen. Refused at once,
  /// in the core's words, where nothing is written or there is nowhere to begin.
  func beginSession(_ text: String, newWorktree: Bool) throws(ActError) -> any OpenedSession

  /// What the Workspace Picker shows, opened unsearched, now and on every change: every Remote's Workspaces, the
  /// Outlook's ticked.
  func workspaces() -> AsyncStream<WorkspacePicker>

  /// Searches the Workspace Picker by name; an empty text searches for nothing.
  func searchWorkspaces(_ text: String)

  /// Turns the Outlook toward the Workspace a row of the Workspace Picker carries, so the next Session begins there,
  /// leaving the listing's scope where it was. Refused where its Server cannot read it.
  func chooseWorkspace(_ workspace: String) async throws(ActError)

  /// Narrows the listing to the scope a choice of its title's menu carries. A Remote or a Workspace chosen turns the
  /// Outlook toward it; Everywhere turns nothing.
  func narrowListing(_ scope: String) async throws(ActError)

  /// Narrows the listing to the scope a listing said it stood at, as the scene kept it and hands it back after a cold
  /// start, turning nothing. A scope that is gone, as one naming a Remote no longer held or a Workspace its Remote no
  /// longer offers, narrows it to Everywhere.
  func restoreListing(_ scope: String)

  /// Searches the listing by Title; an empty text searches for nothing.
  func searchListing(_ text: String)

  /// The unsent text of the composer of the Session `session` names, asked for when its screen appears, with the
  /// version of it each change is handed back with.
  func draft(for session: String) -> Draft

  /// Hands the core the composer's text for the Session `session` names, on each change, written over `version` of
  /// the draft. Text the core gave back since that version stays ahead of it, and `drafts(for:)` says so.
  func setDraft(_ text: String, version: UInt64, for session: String)

  /// The draft of the composer of the Session `session` names, now and each time the core changes it on its own: a
  /// Prompt whose send failed, or that a stop withdrew, given back ahead of what is written, which the field takes.
  func drafts(for session: String) -> AsyncStream<Draft>

  /// Whether any act is under way, now and each time that changes: iOS background time is held only while one is.
  func actsUnderWay() -> AsyncStream<Bool>

  /// Tries an Unreachable Remote again, as its slim row offers, once it answers; or fails as Unreachable where it
  /// still does not.
  func tryAgain(_ remote: String) async throws(ActError)

  /// Tells the core the app has come to the front, so it checks every Remote, keeping what answers and redialling the
  /// rest at once.
  func cameToFront()

  /// Tells the core the app has left the front; it does nothing to its connections.
  func leftFront()

  /// Tells the core the network path changed, and whether there is one at all.
  func pathChanged(hasPath: Bool)

  /// What the screen where pairing begins says to do.
  func pairingPrompt() -> String

  /// Reads `invite` for the confirmation screen, dialling nothing.
  func readInvite(_ invite: String) -> InviteReading

  /// Pairs with the Server `invite` is from, naming it `remoteName` and this phone `deviceName`, replacing any Pairing
  /// with that Server, and turns the Outlook toward it. It keeps dialling for a few seconds while no way answers, and
  /// keeps nothing where it fails.
  func confirmPairing(invite: String, remoteName: String, deviceName: String) async -> PairingOutcome
}

/// One Session open on a pushed screen, read live until it is closed.
public protocol OpenedSession: AnyObject, Sendable {
  /// The Session, as the core is handed it back: for one being begun, under the identity the core chose.
  var reference: String { get }

  /// Asks again to begin the Session being begun, whose Server refused it, with the same Prompt, once its Server has
  /// answered: the refusal, or the Session made, arrives by the frame and the conversation.
  func tryAgain() async throws(ActError)

  /// What frames the screen, now and whole on every change.
  func frame() -> AsyncStream<SessionFrame>

  /// The conversation, whole at first and then each change to it, every one in the order the core made them.
  func conversation() -> AsyncStream<ConversationChange>

  /// The work of the Turn a card hands back, now and whole on every change, for as long as the stream is read: while
  /// its screen is up.
  func work(_ turn: String) -> AsyncStream<TurnWork>

  /// The whole of the Activity a row hands back, now and on every change, for as long as the stream is read: while
  /// its screen is up. A running command's output grows in it.
  func detail(_ activity: String) -> AsyncStream<ActivityDetail>

  /// The tree the Session belongs to, as its sheet shows it, now and whole on every change, for as long as the stream
  /// is read: while the sheet is up.
  func tree() -> AsyncStream<SubagentTree>

  /// Stops the working Subagent a tree's entry hands back, and whatever it delegated, once its Server has answered.
  func stopSubagent(_ session: String) async throws(ActError)

  /// Ends the Session's reading in the core; it cancels nothing on the Server.
  func close()

  /// Sends `text` as a Prompt, steering the working Turn or beginning one. It stands at once as a bubble marked as
  /// sending; where the send fails the bubble goes and the core gives the text back to the composer's draft. On a
  /// Session being begun whose Server refused it, it replaces the refused Prompt and asks again.
  func send(_ text: String) async throws(ActError)

  /// Queues `text` as a Prompt for after the working Turn, failing as a send does.
  func queue(_ text: String) async throws(ActError)

  /// Delivers the queued Prompt `prompt` names now, steering the working Turn.
  func steerNow(_ prompt: String) async throws(ActError)

  /// Withdraws the queued Prompt `prompt` names.
  func cancelQueued(_ prompt: String) async throws(ActError)

  /// Stops the Session's work. A Session Working only for a Prompt not yet delivered has that Prompt withdrawn, and
  /// the core gives its text back to the composer's draft, ahead of what is written there.
  func stop() async throws(ActError)

  /// Answers the Approval `approval` names with `decision`, once its Server has. One whose answer was lost is never
  /// sent again: it waits behind "Review", as one dismissed does, until the Session shows it answered.
  func decide(_ approval: String, _ decision: DecisionKind) async throws(ActError)

  /// Answers the Questionnaire `questionnaire` names with `answers`, one for each of its Questions in their order,
  /// once its Server has. Answers whose delivery was lost are never sent again: the Questionnaire waits behind
  /// "Review", as one dismissed does, until the Session shows it answered.
  func answer(_ questionnaire: String, _ answers: [GivenAnswer]) async throws(ActError)

  /// Declines the Questionnaire `questionnaire` names whole, once its Server has.
  func decline(_ questionnaire: String) async throws(ActError)

  /// Dismisses every one of the Session's own Interventions pending now, as swiping its sheet away does, for as long
  /// as this screen stays open: the bar offers them again, and one asked after presents itself.
  func dismiss()

  /// Presents the Interventions dismissed again, oldest first.
  func review()
}

extension ActError {
  /// The words the core wrote for the failure.
  public var message: String {
    switch self {
    case .Unreachable(let message), .Refused(let message), .Gone(let message), .Invalid(let message):
      message
    }
  }
}

extension OpenError {
  /// The words the core wrote for the failure.
  public var message: String {
    switch self {
    case .Failed(let message), .KeyRefused(let message): message
    }
  }
}
