import XCTest

/// A conversation keeps its place as the columns beside it come and go, on a turned iPhone 18 Pro Max: each stands it
/// across another width.
final class PlaceBesideTests: TurningTestCase {
  @MainActor
  private func opened() -> XCUIApplication {
    let app = launched("longSession", ["--open", "workstation-key/30"], turned: true)
    columnsStand(app)
    XCTAssertTrue(element("session.agentMessage", in: app).waitForExistence(timeout: 10))
    Thread.sleep(forTimeInterval: 2)
    return app
  }

  /// Waits until the conversation is as wide as `column` says.
  @MainActor
  private func conversation(
    in app: XCUIApplication, file: StaticString = #filePath, line: UInt = #line, _ column: (CGFloat) -> Bool
  ) -> CGFloat {
    let conversation = element("session.conversation", in: app)
    for _ in 0..<40 {
      if column(conversation.frame.width) { return conversation.frame.width }
      RunLoop.current.run(until: Date().addingTimeInterval(0.25))
    }
    XCTFail("the conversation stands \(conversation.frame) wide", file: file, line: line)
    return 0
  }

  @MainActor
  func testTheSidebarPutAwayAndShownAgainReturnsExactlyThreeTimesOver() throws {
    let app = opened()
    let beside = element("session.conversation", in: app).frame.width
    for _ in 0..<5 { drag(150, in: app) }
    let began = try standing(in: app)
    let words = try lines(in: app)
    attach("Beside the Sidebar")
    var alone: [String]?
    for round in 0..<3 {
      app.buttons["Hide Sidebar"].firstMatch.tap()
      _ = conversation(in: app) { $0 > beside + 100 }
      Thread.sleep(forTimeInterval: 1)
      if let alone {
        try stands(alone, in: app, "the Sidebar put away, round \(round)")
      } else {
        attach("The Sidebar put away")
        alone = try standing(in: app)
        sameAtTheTop(words, try lines(in: app))
      }
      app.buttons["Show Sidebar"].firstMatch.tap()
      _ = conversation(in: app) { $0 == beside }
      try stands(began, in: app, "beside the Sidebar, round \(round)")
    }
  }

  @MainActor
  func testTheAsideShownAndHiddenReturnsExactlyThreeTimesOver() throws {
    let app = opened()
    let beside = element("session.conversation", in: app).frame.width
    for _ in 0..<5 { drag(150, in: app) }
    let began = try standing(in: app)
    let words = try lines(in: app)
    var shown: [String]?
    for round in 0..<3 {
      app.buttons["session.aside"].tap()
      waitForOne("session.aside", "label == 'Hide aside'", in: app)
      _ = conversation(in: app) { $0 != beside }
      Thread.sleep(forTimeInterval: 1)
      if let shown {
        try stands(shown, in: app, "the Aside shown, round \(round)")
      } else {
        attach("The Aside shown")
        shown = try standing(in: app)
        sameAtTheTop(words, try lines(in: app))
      }
      // Turned upright the Aside waits, and the conversation stands across a third width.
      if round == 1 {
        turn(.portrait, app)
        sameAtTheTop(words, try lines(in: app))
        turn(.landscapeLeft, app)
        try stands(XCTUnwrap(shown), in: app, "turned and turned back")
      }
      app.buttons["session.aside"].tap()
      columnsStand(app)
      _ = conversation(in: app) { $0 == beside }
      try stands(began, in: app, "the Aside hidden, round \(round)")
    }
  }

  @MainActor
  func testAtItsFootItStaysAtItsFootAsTheAsideIsShownAndHidden() throws {
    let app = opened()
    for round in 0..<3 {
      for label in ["Hide aside", "Show aside"] {
        app.buttons["session.aside"].tap()
        waitForOne("session.aside", "label == '\(label)'", in: app)
        try standsAtItsFoot(app, lastWords: "That's everything", "round \(round)")
      }
    }
  }

  /// A Turn begun beside the Sidebar is told of as stopped beside the Aside, which came in between.
  @MainActor
  func testACardChangesAfterTheAsideIsShown() {
    let app = launched("paired", ["--open", "workstation-key/7"], turned: true)
    titled("Tidy zsh startup time", in: app)
    let composer = element("session.composer", in: app)
    composer.tap()
    composer.typeText("Now the other shell")
    app.buttons["session.send"].tap()
    XCTAssertTrue(app.buttons["session.stop"].waitForExistence(timeout: 20), "a Turn began")
    waitForOne("session.turn", "label == 'Working'", in: app)
    app.buttons["session.aside"].tap()
    waitForOne("session.aside", "label == 'Hide aside'", in: app)
    XCTAssertTrue(app.navigationBars["Working"].waitForExistence(timeout: 10))
    Thread.sleep(forTimeInterval: 1)
    app.buttons["session.stop"].tap()
    XCTAssertTrue(app.navigationBars["Stopped after 4m 12s"].waitForExistence(timeout: 10))
    waitForOne("session.turn", "label BEGINSWITH 'Stopped after 4m 12s'", in: app)
    XCTAssertFalse(element("session.turn", holding: "Working", in: app).exists)
  }
}
