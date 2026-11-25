import XCTest

/// The Sidebar beside the main view, on a turned iPhone 18 Pro Max: what is chosen in it is the main view's root, and
/// with one column alone what is open stands over the listing again.
final class SidebarTests: TurningTestCase {
  @MainActor
  func testASessionChosenFromTheSidebarIsTheMainViewsRootDrawnAfresh() {
    let app = launched("longSession", ["--open", "workstation-key/12"], turned: true)
    titled("Find what holds the session lock", in: app)
    columnsStand(app)
    XCTAssertFalse(app.back.exists, "the main view's root has nothing to go back to")
    attach("The Sidebar beside a session")

    // What is pushed over it goes with it, as a Subagent's Session from the tree in the Aside.
    app.buttons["session.tree"].tap()
    let explorer = element("tree.row", holding: "Find what held the lock", in: app)
    app.scrolled(to: explorer, over: element("tree.list", in: app)).tap()
    titled("Find what held the lock", in: app)
    app.buttons["session.aside"].tap()
    columnsStand(app)
    XCTAssertTrue(app.back.isHittable, "pushed in the main view")
    attach("The Sidebar beside a subagent's session")
    open("Port the transcript to the phone", in: app)
    XCTAssertFalse(app.back.exists, "it is the root in the other's place")

    // Scrolled away from its foot, and opened again, it stands at its foot, as one drawn afresh does.
    let last = element("session.agentMessage", holding: "That's everything", in: app)
    XCTAssertTrue(last.waitForExistence(timeout: 10))
    let conversation = element("session.conversation", in: app)
    for _ in 0..<2 { conversation.swipeDown(velocity: .fast) }
    XCTAssertTrue(last.waitForNonExistence(timeout: 10), "away from its foot")
    open("Profile the transcript on the phone", in: app)
    XCTAssertFalse(app.back.exists)
    open("Port the transcript to the phone", in: app)
    XCTAssertTrue(last.waitForExistence(timeout: 10), "at its foot")
    XCTAssertFalse(app.back.exists)
    attach("A session opened again from the Sidebar")
    columnsStand(app)
  }

  @MainActor
  func testASessionGoneRestsTheMainViewAtTheNewSessionScreen() {
    let app = launched("paired", ["--open", "workstation-key/7"], turned: true)
    titled("Tidy zsh startup time", in: app)
    columnsStand(app)
    let composer = element("session.composer", in: app)
    composer.tap()
    composer.typeText("Is it gone?")
    app.buttons["session.send"].tap()
    XCTAssertTrue(app.staticTexts["session.title"].waitForNonExistence(timeout: 20), "the Session gone")
    columnsStand(app, title: "landing.title")
    XCTAssertFalse(app.back.exists)
    XCTAssertFalse(holdsTheKeyboard(element("landing.composer", in: app)), "at rest, its keyboard down")
    attach("A session gone, beside the listing")
    // At rest is not open.
    turn(.portrait, app)
    oneColumnStands(app, showing: "listing.scope")
  }

  @MainActor
  func testTurningStandsTheSidebarBesideWhatIsOpenAndPutsItBackOverTheListing() {
    let app = launched()
    open("Tidy zsh startup time", in: app)
    oneColumnStands(app, showing: "session.title")
    for side in [UIDeviceOrientation.landscapeLeft, .landscapeRight] {
      turn(side, app)
      columnsStand(app)
      titled("Tidy zsh startup time", in: app)
      XCTAssertFalse(app.back.exists, "the root of the main view")
      turn(.portrait, app)
      oneColumnStands(app, showing: "session.title")
      titled("Tidy zsh startup time", in: app)
    }
    // One chosen beside it is what stands over the listing next.
    turn(.landscapeLeft, app)
    columnsStand(app)
    open("Draw the invite as a QR code in /serve", in: app)
    turn(.portrait, app)
    oneColumnStands(app, showing: "session.title")
    titled("Draw the invite as a QR code in /serve", in: app)
    app.back.tap()
    oneColumnStands(app, showing: "listing.scope")
    // Returned to, with nothing open beside it.
    turn(.landscapeRight, app)
    columnsStand(app, title: "landing.title")
    XCTAssertFalse(holdsTheKeyboard(element("landing.composer", in: app)))
    turn(.portrait, app)
    oneColumnStands(app, showing: "listing.scope")
  }
}
