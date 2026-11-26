import XCTest

/// The Duo partly folded like a book: run by `scripts/ios-test.sh posed book`.
///
/// What holds that it folded is where things stand about the fold: the split view divides the Sidebar and the main view
/// there by itself, as open flat in the same landscape the Sidebar is 320 points wide, and pairing keeps to the leading
/// half.
final class PosedBookTests: XCTestCase {
  /// The band kept for the fold, from head to foot of the window.
  private let band: ClosedRange<CGFloat> = 455.5...495.5

  @MainActor
  func testTheBarStandsDownTheSide() throws {
    try PosedWalk.walk(.book, self, barStandsVertically: true)
  }

  /// Waits until it stands so.
  @MainActor
  private func waits(_ said: String, file: StaticString = #filePath, line: UInt = #line, _ stands: () -> Bool) {
    for _ in 0..<40 {
      if stands() { return }
      RunLoop.current.run(until: Date().addingTimeInterval(0.5))
    }
    attach("Not as it should stand")
    XCTFail(said, file: file, line: line)
  }

  /// Pairing at the root of a phone holding no Remote, over the fake scanner, which holds up the workstation's Invite
  /// twenty seconds after it starts, so that it is still looking while the test looks at it: where pairing begins, the
  /// scanner and the confirmation stand in the leading half, the scanner whole in it, its words over it at its foot.
  /// What holds that it folded is each ending at the fold, as open flat in landscape they stand across the window.
  @MainActor
  func testPairingStandsInTheLeadingHalfItsScannerWholeInIt() throws {
    let app = try Pose.book.launched(
      self,
      [
        "--fake", "fresh", "--forget-scene", "--scan", "shaman-v1-fake-workstation", "--scan-every", "20",
      ])
    XCTAssertTrue(app.staticTexts["Pair with a server"].waitForExistence(timeout: 10), "pairing alone is offered")
    let window = app.window.frame
    let form = app.collectionViews.firstMatch
    waits("where pairing begins stands in the leading half") {
      form.exists && form.frame.maxX <= band.lowerBound + 1 && form.frame.width > 300
    }
    attach("Where pairing begins, partly folded like a book")

    app.buttons["pairing.scan"].tap()
    let picture = app.descendants(matching: .any)["scan.picture"]
    let looking = app.staticTexts["scan.looking"]
    waits("the scanner's picture ends at the fold") {
      picture.exists && abs(picture.frame.maxX - band.lowerBound) <= 1
    }
    XCTAssertEqual(picture.frame.minX, window.minX, accuracy: 1, "\(picture.frame)")
    XCTAssertEqual(picture.frame.maxY, window.maxY, accuracy: 1, "to the bottom edge")
    XCTAssertTrue(looking.exists)
    XCTAssertLessThan(looking.frame.maxX, band.lowerBound, "its words over its picture")
    XCTAssertEqual(looking.frame.midX, picture.frame.midX, accuracy: 1, "in its middle")
    XCTAssertGreaterThan(looking.frame.minY, window.midY, "at its foot")
    attach("The scanner, partly folded like a book")

    // Scanning as ever, it takes the Invite it is shown.
    XCTAssertTrue(app.navigationBars["Pair"].waitForExistence(timeout: 60), "the confirmation of the Invite read")
    XCTAssertTrue(app.descendants(matching: .any)["confirm.hostname"].waitForExistence(timeout: 10))
    waits("the confirmation stands in the leading half") {
      form.exists && form.frame.maxX <= band.lowerBound + 1 && form.frame.width > 300
    }
    attach("The confirmation, partly folded like a book")
  }

  /// Waits until the Sidebar and the main view divide at the fold, the listing ending in its band and the main view's
  /// `title` beginning past it, and nothing the app shows of words or buttons stands in that band.
  @MainActor
  private func dividedAtTheFold(
    _ app: XCUIApplication, title: String, file: StaticString = #filePath, line: UInt = #line
  ) {
    let list = app.collectionViews["listing.list"]
    let main = app.staticTexts[title]
    waits("the Sidebar and the main view divide at the fold", file: file, line: line) {
      list.exists && list.isHittable && band.contains(list.frame.maxX) && main.exists && main.isHittable
        && main.frame.minX > band.upperBound
    }
    let inner = band.lowerBound + 1...band.upperBound - 1
    let words = app.staticTexts.allElementsBoundByIndex + app.buttons.allElementsBoundByIndex
    let inTheFold = words.filter(\.isHittable).filter {
      $0.frame.maxX > inner.lowerBound && $0.frame.minX < inner.upperBound
    }
    XCTAssertTrue(
      inTheFold.isEmpty, "in the fold: \(inTheFold.map { $0.identifier.isEmpty ? $0.label : $0.identifier })",
      file: file, line: line)
  }

  @MainActor
  func testWithASessionOpenAndTheAsideNotShownTheSidebarAndTheMainViewDivideAtTheFold() throws {
    let app = try Pose.book.launched(self, ["--fake", "paired", "--open", "workstation-key/7", "--forget-scene"])
    dividedAtTheFold(app, title: "session.title")
    XCTAssertFalse(app.back.exists, "the main view's root")
    attach("The book pose: the listing and a session, divided at the fold")
  }

  @MainActor
  func testWithNothingOpenTheSidebarAndTheNewSessionScreenDivideAtTheFold() throws {
    let app = try Pose.book.launched(self, ["--fake", "paired", "--forget-scene"])
    dividedAtTheFold(app, title: "landing.title")
    attach("The book pose: the listing and the new-session screen, divided at the fold")
  }
}
