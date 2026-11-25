import XCTest

/// A pose a posed run holds an iPhone Duo in from start to finish, which a test written for it is run in alone.
///
/// Nothing in Xcode 27.2 lets a test pose a Duo, so `scripts/ios-test.sh posed` does, and names the pose to the test.
enum Pose: String {
  case open, book, laptop, portrait

  /// The size of the app's window: the inner display's, in landscape or turned to portrait. It is all a test can tell
  /// of a pose, so open flat and the book pose, and portrait and the laptop pose, look alike to it.
  private var size: CGSize {
    switch self {
    case .open, .book: CGSize(width: 951, height: 669)
    case .laptop, .portrait: CGSize(width: 669, height: 951)
    }
  }

  /// The app launched in this pose, which is where a test of it begins. Skips where no run, or a run of another pose,
  /// is under way, and fails where the simulator is not in it, as Device Hub does not always press.
  @MainActor
  func launched(
    _ test: XCTestCase, _ arguments: [String], file: StaticString = #filePath, line: UInt = #line
  ) throws -> XCUIApplication {
    guard let named = ProcessInfo.processInfo.environment["SHAMAN_POSE"], named == rawValue else {
      throw XCTSkip("Run by scripts/ios-test.sh posed \(rawValue)")
    }
    test.continueAfterFailure = false
    let app = XCUIApplication()
    app.launchArguments = arguments
    app.launch()
    // A first launch on the inner display takes longer to be drawn than on the outer.
    let window = app.window
    var shown = CGSize.zero
    for _ in 0..<40 {
      if window.exists { shown = window.frame.size }
      if abs(shown.width - size.width) <= 1, abs(shown.height - size.height) <= 1 { return app }
      RunLoop.current.run(until: Date().addingTimeInterval(0.5))
    }
    XCTFail(
      "the simulator is not in the pose \"\(rawValue)\": the window is \(Int(shown.width)) x \(Int(shown.height)) and "
        + "not \(Int(size.width)) x \(Int(size.height))", file: file, line: line)
    return app
  }
}

extension XCUIApplication {
  /// Back stands down the window's side, as it does where the bar stands vertically, and nowhere else: a navigation
  /// bar's buttons are those of a horizontal one.
  var barStandsVertically: Bool {
    let back = back.frame
    return back.midX > window.frame.maxX - 84 && !navigationBars.buttons["BackButton"].exists
  }

  /// Back stands at the head of the window's leading edge, in a navigation bar.
  var barStandsHorizontally: Bool {
    let back = back.frame
    return back.midY < 134 && back.midX < window.frame.midX && navigationBars.buttons["BackButton"].exists
  }
}

/// The least a pose shows, over today's build: how the bar stands, the composer's field where Messages stands its own, a
/// screenshot of the inner display, and that a drag made there by the helper moves what is under it. Each pose's tests begin with it.
@MainActor
enum PosedWalk {
  static func walk(_ pose: Pose, _ test: XCTestCase, barStandsVertically: Bool) throws {
    let app = try pose.launched(test, ["--fake", "longSession", "--open", "workstation-key/30"])
    XCTAssertTrue(app.back.waitForExistence(timeout: 10))
    XCTAssertEqual(app.barStandsVertically, barStandsVertically, "the bar down the side")
    XCTAssertEqual(app.barStandsHorizontally, !barStandsVertically, "the bar across the head")

    let last = app.descendants(matching: .any).matching(identifier: "session.agentMessage")
      .matching(NSPredicate(format: "label CONTAINS %@", "The last item of this Session")).firstMatch
    XCTAssertTrue(last.waitForExistence(timeout: 15))
    test.attach("The conversation, \(pose.rawValue)")
    test.fieldStands(22, aboveTheEdgeIn: app, "session.composer")
    let stood = last.frame.minY
    let start = app.point(across: 0.5, down: 0.3)
    start.press(
      forDuration: 0.05, thenDragTo: start.withOffset(CGVector(dx: 0, dy: 150)), withVelocity: .slow,
      thenHoldForDuration: 0.3)
    XCTAssertTrue(!last.exists || last.frame.minY > stood + 50, "the drag moved the conversation down")
  }
}
