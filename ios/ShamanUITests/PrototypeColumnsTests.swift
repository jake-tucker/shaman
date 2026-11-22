// PROTOTYPE for #725. Throwaway: never merge. The tests that would hold each way, run over the lab's placeholder
// screens: `TEST_RUNNER_LAB_ARGS` chooses the container, as "--lab uikit" or "--lab swiftui --lab-back appear".

import XCTest

final class PrototypeColumnsTests: XCTestCase {
  private var app: XCUIApplication!

  override func setUp() {
    continueAfterFailure = false
    XCUIDevice.shared.orientation = .portrait
  }

  override func tearDown() {
    XCUIDevice.shared.orientation = .portrait
  }

  private func launch(_ more: [String] = []) {
    let chosen = (ProcessInfo.processInfo.environment["LAB_ARGS"] ?? "--lab uikit").split(separator: " ").map(String.init)
    app = XCUIApplication()
    app.launchArguments = ["--fake", "paired"] + chosen + more
    app.launch()
  }

  /// Waits for the lab's own line to hold these words, and says what it held where it never did.
  private func expect(_ words: String..., file: StaticString = #filePath, line: UInt = #line) {
    let state = app.staticTexts["lab.state"]
    XCTAssertTrue(state.waitForExistence(timeout: 10), "no state line", file: file, line: line)
    let held = NSPredicate { _, _ in words.allSatisfy { state.label.contains($0) } }
    let result = XCTWaiter.wait(for: [XCTNSPredicateExpectation(predicate: held, object: nil)], timeout: 6)
    XCTAssertEqual(result, .completed, "wanted \(words), held \(state.label)", file: file, line: line)
  }

  private func back() {
    let button = app.buttons["BackButton"].firstMatch
    XCTAssertTrue(button.waitForExistence(timeout: 5), "no Back")
    button.tap()
  }

  private func note(_ words: String) {
    let line = "------- test: \(words)\n"
    if let handle = FileHandle(forWritingAtPath: "/tmp/duo725/app.log") {
      handle.seekToEndOfFile()
      handle.write(Data(line.utf8))
      try? handle.close()
    }
  }

  /// Back from two screens deep steps one screen at a time, and only the last step is the reader returning.
  func testBackStepsOneScreenAtATime() {
    launch()
    app.buttons["lab.row.3"].tap()
    expect("path=[S3]", "returned=0")
    app.buttons["lab.push"].firstMatch.tap()
    expect("path=[S3>S3+]", "returned=0")
    note("back from two deep")
    back()
    expect("path=[S3]", "returned=0")
    XCTAssertTrue(app.navigationBars["S3"].waitForExistence(timeout: 3))
    note("back from the root")
    back()
    expect("path=[]", "returned=1")
    XCTAssertTrue(app.buttons["lab.row.3"].waitForExistence(timeout: 3))
    // And again, so that what closed the first leaves the second whole.
    app.buttons["lab.row.5"].tap()
    expect("path=[S5]", "returned=1")
    XCTAssertTrue(app.navigationBars["S5"].waitForExistence(timeout: 3))
  }

  /// A swipe from the edge given up leaves what is open; one carried through closes it once.
  func testASwipeGivenUpClosesNothing() {
    launch()
    app.buttons["lab.row.3"].tap()
    expect("path=[S3]", "returned=0")
    let window = app.windows.firstMatch
    let edge = window.coordinate(withNormalizedOffset: CGVector(dx: 0.0, dy: 0.5))
    note("a swipe given up")
    edge.press(
      forDuration: 0.1, thenDragTo: window.coordinate(withNormalizedOffset: CGVector(dx: 0.3, dy: 0.5)),
      withVelocity: .slow, thenHoldForDuration: 0.6)
    sleep(2)
    expect("path=[S3]", "returned=0")
    XCTAssertTrue(app.navigationBars["S3"].exists)
    note("a swipe carried through")
    edge.press(
      forDuration: 0.1, thenDragTo: window.coordinate(withNormalizedOffset: CGVector(dx: 0.9, dy: 0.5)),
      withVelocity: .slow, thenHoldForDuration: 0.1)
    expect("path=[]", "returned=1")
  }

  /// Turning changes the width class where the display is wide enough, both ways, and closes nothing.
  func testTurningKeepsWhatIsOpen() {
    launch()
    app.buttons["lab.row.3"].tap()
    app.buttons["lab.push"].firstMatch.tap()
    expect("path=[S3>S3+]", "returned=0")
    for turn in 1...2 {
      note("turned on its side, \(turn)")
      XCUIDevice.shared.orientation = .landscapeLeft
      sleep(2)
      expect("path=[S3>S3+]", "returned=0")
      XCTAssertTrue(app.navigationBars["S3+"].exists)
      note("turned upright, \(turn)")
      XCUIDevice.shared.orientation = .portrait
      sleep(2)
      expect("path=[S3>S3+]", "returned=0")
      XCTAssertTrue(app.navigationBars["S3+"].exists)
    }
    back()
    expect("path=[S3]", "returned=0")
    back()
    expect("path=[]", "returned=1")
  }

  /// The Aside waits with what it holds while there is no room, and stands again by itself as room returns.
  func testTheAsideWaitsThroughATurn() throws {
    launch()
    XCUIDevice.shared.orientation = .landscapeLeft
    sleep(2)
    app.buttons["lab.row.3"].tap()
    expect("path=[S3]")
    let wide = app.buttons["lab.row.5"].exists
    try XCTSkipUnless(wide, "this display does not tile columns on its side")
    note("the aside asked for")
    app.buttons["lab.aside"].firstMatch.tap()
    expect("wanted=1", "aside=1")
    app.buttons["lab.activity.2"].tap()
    expect("work=1")
    XCTAssertTrue(app.staticTexts["The whole of activity 2"].waitForExistence(timeout: 3))
    for turn in 1...2 {
      note("turned upright with the aside up, \(turn)")
      XCUIDevice.shared.orientation = .portrait
      sleep(2)
      expect("wanted=1", "aside=0", "work=1", "path=[S3]", "returned=0")
      XCTAssertFalse(app.staticTexts["The whole of activity 2"].isHittable, "the aside stands with no room")
      XCTAssertTrue(app.navigationBars["S3"].exists)
      note("turned on its side again, \(turn)")
      XCUIDevice.shared.orientation = .landscapeLeft
      sleep(2)
      expect("wanted=1", "aside=1", "work=1", "path=[S3]", "returned=0")
      XCTAssertTrue(app.staticTexts["The whole of activity 2"].waitForExistence(timeout: 3))
    }
  }
}
