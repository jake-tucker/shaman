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

  @MainActor
  func testWithTheSidebarPutAwayAConversationReads700PointsWideInTheMiddle() {
    for session in ["workstation-key/1", "workstation-key/7"] {
      let app = launched("paired", ["--open", session], turned: true)
      columnsStand(app)
      let conversation = element("session.conversation", in: app)
      attach("\(session) beside the Sidebar")
      XCTAssertLessThan(conversation.frame.width, 700)
      app.buttons["Hide Sidebar"].firstMatch.tap()
      XCTAssertTrue(app.buttons["listing.scope"].waitForNonExistence(timeout: 10), "the Sidebar put away")
      conversationReads(Sides.middle, in: app, session)
      XCTAssertGreaterThan(conversation.frame.width, 750)
      attach("\(session), the Sidebar put away")
      app.terminate()
    }
  }

  @MainActor
  func testTheFieldStandsLowerBesideTheSidebarThanAlone() {
    let keyboardStays = ["-AutomaticMinimizationEnabled", "NO"]
    for (scenario, arguments, field, title) in [
      ("paired", ["--open", "workstation-key/7"], "session.composer", "session.title"),
      ("composing", ["--compose"], "landing.composer", "landing.title"),
    ] {
      let app = launched(scenario, arguments + keyboardStays, turned: true)
      columnsStand(app, title: title)
      fieldStands(22, aboveTheEdgeIn: app, field, "beside the Sidebar")
      attach("\(field) beside the Sidebar")
      if field == "session.composer" {
        app.buttons["session.aside"].tap()
        XCTAssertTrue(element("work.list", in: app).waitForExistence(timeout: 10))
        fieldStands(22, aboveTheEdgeIn: app, field, "beside the Aside")
        attach("\(field) beside the Aside")
        app.buttons["session.aside"].tap()
        columnsStand(app, title: title)
      }
      turn(.portrait, app)
      fieldStands(28, aboveTheEdgeIn: app, field, "alone")
      attach("\(field) alone")
      turn(.landscapeRight, app)
      columnsStand(app, title: title)
      fieldStands(22, aboveTheEdgeIn: app, field, "beside the Sidebar again")
      element(field, in: app).tap()
      fieldStandsOnTheKeyboard(in: app, field)
      attach("\(field) beside the Sidebar, the keyboard up")
      app.terminate()
    }
    // And at rest.
    let app = launched("composing", turned: true)
    columnsStand(app, title: "landing.title")
    fieldStands(22, aboveTheEdgeIn: app, "landing.composer", "at rest")
  }
}
