import XCTest

/// The Duo partly folded like a book: run by `scripts/ios-test.sh posed book`.
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
}
