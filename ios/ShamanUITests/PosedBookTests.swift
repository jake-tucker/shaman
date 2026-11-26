import XCTest

/// The Duo partly folded like a book: run by `scripts/ios-test.sh posed book`.
///
/// What holds that it folded is where things stand about the fold: the split view divides the Sidebar and the main view
/// there by itself, as open flat in the same landscape the Sidebar is 320 points wide, and the main view and the Aside,
/// and pairing keeps to the leading half.
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
    nothingStandsInTheFold(app, file: file, line: line)
  }

  /// Fails where the app draws anything in the band but its grounds: any pixel darker than 240 of 255, where the band
  /// reads 246 or lighter, so that it catches words, a status word in its colour (an orange one reads 163), a chevron
  /// (178 to 191), a symbol, a tint or a hairline. It reads the picture, as a row's frame is its cell's, which runs on
  /// into the band to the middle of the fold where the system divides the columns, while what the row draws keeps 40
  /// points from there.
  @MainActor
  private func nothingStandsInTheFold(_ app: XCUIApplication, file: StaticString = #filePath, line: UInt = #line) {
    let picture = Display.picture()
    let pixels = picture.cgImage!
    let scale = CGFloat(pixels.width) / app.window.frame.width
    let width = pixels.width
    let height = pixels.height
    let context = CGContext(
      data: nil, width: width, height: height, bitsPerComponent: 8, bytesPerRow: width,
      space: CGColorSpaceCreateDeviceGray(), bitmapInfo: CGImageAlphaInfo.none.rawValue)!
    context.draw(pixels, in: CGRect(x: 0, y: 0, width: width, height: height))
    let gray = UnsafeRawBufferPointer(start: context.data!, count: width * height)
    let columns =
      Int(((band.lowerBound + 1) * scale).rounded(.up))..<Int(((band.upperBound - 1) * scale).rounded(.down))
    let drawn = (0..<height).flatMap { row in columns.map { (row, $0) } }.filter { gray[$0.0 * width + $0.1] < 240 }
    guard let first = drawn.first else { return }
    attach("Drawn in the fold", showing: picture)
    XCTFail(
      "drawn in the fold: \(drawn.count) pixels, the first at \(Int(CGFloat(first.1) / scale)) across and "
        + "\(Int(CGFloat(first.0) / scale)) down", file: file, line: line)
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

  /// With the Aside shown the system puts the Sidebar behind its button and divides the main view and the Aside at the
  /// fold by itself: the conversation ends before the band or in it and the Aside's list begins in it, where open flat in the
  /// same landscape the Aside stands at its own width, beginning far past the band.
  @MainActor
  func testWithTheAsideShownTheMainViewAndTheAsideDivideAtTheFold() throws {
    let app = try Pose.book.launched(
      self, ["--fake", "paired", "--open", "workstation-key/7", "--aside", "work", "--forget-scene"])
    let main = app.descendants(matching: .any)["session.conversation"]
    let aside = app.descendants(matching: .any)["work.list"]
    // Both lists read as not hittable here though they stand in sight, as the pictures show, so a row of each stands
    // for its list.
    let card = app.buttons["session.turn"].firstMatch
    let row = app.buttons["work.row"].firstMatch
    let scope = app.buttons["listing.scope"]
    waits("the main view and the Aside divide at the fold") {
      main.exists && card.exists && card.isHittable && main.frame.minX < band.lowerBound
        && main.frame.maxX <= band.upperBound && aside.exists && row.exists && row.isHittable
        && band.contains(aside.frame.minX)
        && aside.frame.maxX > band.upperBound + 200 && !(scope.exists && scope.isHittable)
    }
    XCTAssertEqual(app.buttons["session.aside"].firstMatch.label, "Hide aside")
    nothingStandsInTheFold(app)
    attach("The book pose: a session and the Aside, divided at the fold")
  }
}
