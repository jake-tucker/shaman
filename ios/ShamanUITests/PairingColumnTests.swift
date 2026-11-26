import XCTest

/// Pairing on its side at the root of a phone holding no Remote, over the fake: where it begins and the confirmation
/// read in a column as a conversation does, at most 700 points across in the middle of the window, which bites on a
/// turned iPhone 18 Pro Max, and the scanner's picture takes the whole width, its words at the bottom in the middle.
final class PairingColumnTests: TurningTestCase {
  /// The Invite the fake reads as the workstation's.
  private let invite = "shaman-v1-fake-workstation"

  /// Waits until the form on screen reads in a column at most 700 points across, in the middle of the window.
  @MainActor
  private func readsInAColumn(_ app: XCUIApplication, file: StaticString = #filePath, line: UInt = #line) {
    let form = app.collectionViews.firstMatch
    XCTAssertTrue(form.waitForExistence(timeout: 10), file: file, line: line)
    let window = app.window.frame
    XCTAssertGreaterThan(window.width, 750, "wider than the column", file: file, line: line)
    XCTAssertLessThanOrEqual(form.frame.width, 700.5, "\(form.frame)", file: file, line: line)
    XCTAssertGreaterThan(form.frame.width, 600, "\(form.frame)", file: file, line: line)
    XCTAssertEqual(form.frame.midX, window.midX, accuracy: 1, "in the middle", file: file, line: line)
  }

  @MainActor
  func testOnItsSidePairingReadsInAColumnAndTheScannersPictureTakesTheWholeWidth() {
    let app = launched("fresh", ["--scan", "https://example.com"], turned: true)
    XCTAssertTrue(app.staticTexts["Pair with a server"].waitForExistence(timeout: 10), "pairing alone is offered")
    readsInAColumn(app)
    attach("Pairing in a column, on its side")

    app.buttons["pairing.scan"].tap()
    let picture = element("scan.picture", in: app)
    XCTAssertTrue(picture.waitForExistence(timeout: 10), "the scanner")
    let looking = app.staticTexts["scan.looking"]
    XCTAssertTrue(looking.waitForExistence(timeout: 10))
    let window = app.window.frame
    XCTAssertGreaterThan(picture.frame.width, 750, "past the column: \(picture.frame)")
    XCTAssertEqual(picture.frame.midX, window.midX, accuracy: 1)
    XCTAssertEqual(picture.frame.maxY, window.maxY, accuracy: 1, "to the bottom edge")
    XCTAssertEqual(looking.frame.midX, window.midX, accuracy: 1, "its words in the middle")
    XCTAssertGreaterThan(looking.frame.minY, window.midY, "and at the bottom")
    attach("The scanner, on its side")
  }

  @MainActor
  func testOnItsSideTheConfirmationOfAnInviteByTheLaunchEnvironmentReadsInAColumn() {
    let app = launched("fresh", turned: true, environment: ["SHAMAN_INVITE": invite])
    XCTAssertTrue(app.navigationBars["Pair"].waitForExistence(timeout: 10), "the confirmation screen")
    XCTAssertTrue(element("confirm.hostname", in: app).waitForExistence(timeout: 10))
    readsInAColumn(app)
    attach("The confirmation in a column, on its side")
  }
}
