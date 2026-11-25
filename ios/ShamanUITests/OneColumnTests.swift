import XCTest

/// One column standing alone on a display that is wide enough for two, as an iPhone 18 Pro's turned is, at compact
/// width: what stands beside the main view elsewhere stands there as it does upright.
final class OneColumnTests: TurningTestCase {
  @MainActor
  func testTurnedThereIsNoAsideSoATurnsWorkIsPushedAndTheTreeIsASheet() {
    let app = launched("paired", ["--open", "workstation-key/12", "--aside", "tree"], turned: true)
    oneColumnStands(app, showing: "session.title")
    XCTAssertTrue(app.buttons["session.tree"].waitForExistence(timeout: 10))
    XCTAssertFalse(app.buttons["session.aside"].exists, "wide enough, but for one column")
    XCTAssertFalse(element("tree.row", in: app).exists)
    attach("A session turned, in one column")

    app.buttons["session.tree"].tap()
    let sheet = app.otherElements["tree.sheet"]
    XCTAssertTrue(sheet.waitForExistence(timeout: 10))
    // The open Session's entry puts it away.
    app.scrolled(to: element("tree.row", holding: "Find what holds the session lock", in: app), over: sheet).tap()
    XCTAssertTrue(sheet.waitForNonExistence(timeout: 10))

    element("session.turn", in: app).tap()
    XCTAssertTrue(element("work.list", in: app).waitForExistence(timeout: 10))
    XCTAssertTrue(app.staticTexts["session.title"].waitForNonExistence(timeout: 10), "pushed over it")
    XCTAssertTrue(app.back.isHittable)
  }

  @MainActor
  func testTurnedTheFieldStandsAsItDoesUprightAndOnTheKeyboard() {
    let keyboardStays = ["-AutomaticMinimizationEnabled", "NO"]
    for (scenario, arguments, field) in [
      ("paired", ["--open", "workstation-key/7"], "session.composer"), ("composing", ["--compose"], "landing.composer"),
    ] {
      let app = launched(scenario, arguments + keyboardStays, turned: true)
      fieldStands(28, aboveTheEdgeIn: app, field)
      // The one column is wider than a conversation reads.
      if field == "session.composer" { conversationReads(Sides.middle, in: app) }
      attach("\(field) turned, in one column")
      element(field, in: app).tap()
      fieldStandsOnTheKeyboard(in: app, field)
      attach("\(field) turned, the keyboard up")
      app.terminate()
    }
  }
}
