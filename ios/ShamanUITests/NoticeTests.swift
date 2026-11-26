import XCTest

/// A notice said once in a window, on an iPhone 18 Pro Max, over the fake's `releases`, whose workstation is newer than
/// the app: in one column by whichever screen stands, and with columns standing by the main view, the Sidebar beside it
/// or put away. A screen's own banner stays beneath the bar of the screen that asked.
final class NoticeTests: TurningTestCase {
  /// Longer than any test takes, unless one is asked for, so the notice goes only when a test says.
  private let standing = ["--notice-stands", "3600"]

  /// What the core says of its own accord, wherever it stands.
  @MainActor
  private func notices(in app: XCUIApplication) -> XCUIElementQuery {
    app.descendants(matching: .any).matching(NSPredicate(format: "label CONTAINS %@", "runs a newer Shaman"))
  }

  /// Waits until the notice is said, and answers it, said once, beneath the bar of the screen whose title is
  /// `title`'s element, and in the main view, clear of the listing, where `besideTheListing`.
  @MainActor
  @discardableResult
  private func saidOnce(
    beneath title: String, besideTheListing: Bool = false, in app: XCUIApplication, file: StaticString = #filePath,
    line: UInt = #line
  ) -> XCUIElement {
    let notice = notices(in: app).firstMatch
    XCTAssertTrue(notice.waitForExistence(timeout: 10), "the notice is said", file: file, line: line)
    // Anything that would say it again has been drawn by now.
    RunLoop.current.run(until: Date().addingTimeInterval(1))
    XCTAssertEqual(notices(in: app).count, 1, "said once", file: file, line: line)
    XCTAssertTrue(notice.isHittable, "on screen", file: file, line: line)
    let bar = elements(title, in: app).firstMatch
    XCTAssertTrue(bar.exists, file: file, line: line)
    XCTAssertGreaterThanOrEqual(notice.frame.minY, bar.frame.maxY, "beneath \(title)", file: file, line: line)
    XCTAssertGreaterThan(notice.frame.maxX, bar.frame.minX, "beneath \(title)", file: file, line: line)
    XCTAssertLessThan(notice.frame.minX, bar.frame.maxX, "beneath \(title)", file: file, line: line)
    if besideTheListing {
      XCTAssertGreaterThanOrEqual(
        notice.frame.minX, listing(in: app).frame.maxX - 1, "in the main view", file: file, line: line)
    }
    return notice
  }

  @MainActor
  func testInOneColumnTheNoticeIsSaidOnceByWhicheverScreenStands() {
    let app = launched("releases", standing)
    oneColumnStands(app, showing: "listing.scope")
    saidOnce(beneath: "listing.scope", in: app)
    attach("Said by the listing alone")
    open("Tidy zsh startup time", in: app)
    oneColumnStands(app, showing: "session.title")
    saidOnce(beneath: "session.title", in: app)
    attach("Said by the Session over the listing")
    app.back.tap()
    oneColumnStands(app, showing: "listing.scope")
    saidOnce(beneath: "listing.scope", in: app).tap()
    XCTAssertTrue(notices(in: app).firstMatch.waitForNonExistence(timeout: 10), "tapped away")
  }

  @MainActor
  func testBesideTheSidebarTheMainViewSaysTheNoticeOnceAndAScreensOwnBannerStaysBeneathItsBar() {
    let app = launched("releases", ["--open", "workstation-key/7"] + standing, turned: true)
    columnsStand(app)
    saidOnce(beneath: "session.title", besideTheListing: true, in: app)
    attach("Said once, by the Session beside the Sidebar")

    // A send refused is said in the Session's own banner, beneath its own bar, and the notice still once.
    let composer = element("session.composer", in: app)
    composer.tap()
    composer.typeText("Please fail")
    element("session.send", in: app).tap()
    let refused = element("banner", holding: "refused the prompt", in: app)
    XCTAssertTrue(refused.waitForExistence(timeout: 10))
    let title = element("session.title", in: app)
    attach("The Session's own banner beneath its bar, with the notice, both in the main view")
    // Its frame, as XCTest reads it, spans the window's top whatever it covers, so where it stands is told by what
    // does not hold it.
    XCTAssertFalse(
      listing(in: app).descendants(matching: .any).matching(NSPredicate(format: "label CONTAINS %@", "refused"))
        .firstMatch.exists, "not in the listing")
    XCTAssertTrue(title.exists)
    saidOnce(beneath: "session.title", besideTheListing: true, in: app)

    // Another chosen from the Sidebar says it in its place.
    open("Draw the invite as a QR code in /serve", in: app)
    columnsStand(app)
    saidOnce(beneath: "session.title", besideTheListing: true, in: app)

    // Turned upright, one column alone, the Session over the listing says it, and the listing once returned to.
    turn(.portrait, app)
    oneColumnStands(app, showing: "session.title")
    saidOnce(beneath: "session.title", in: app)
    app.back.tap()
    oneColumnStands(app, showing: "listing.scope")
    saidOnce(beneath: "listing.scope", in: app)
    // Turned again, nothing open, the screen at rest beside the listing.
    turn(.landscapeLeft, app)
    columnsStand(app, title: "landing.title")
    saidOnce(beneath: "landing.title", besideTheListing: true, in: app)
    attach("Said once, by the new-session screen at rest beside the Sidebar")
  }

  @MainActor
  func testWithTheSidebarPutAwayTheMainViewSaysTheNoticeOnce() {
    let app = launched("releases", ["--open", "workstation-key/7"] + standing, turned: true)
    columnsStand(app)
    app.buttons["Hide Sidebar"].firstMatch.tap()
    XCTAssertTrue(app.buttons["listing.scope"].waitForNonExistence(timeout: 10), "the Sidebar put away")
    saidOnce(beneath: "session.title", in: app)
    attach("Said once, by the Session, the Sidebar put away")
    app.buttons["Show Sidebar"].firstMatch.tap()
    columnsStand(app)
    saidOnce(beneath: "session.title", besideTheListing: true, in: app)
  }

  /// Swipes the sheet on top away: by its grabber, or, at full height on its side where it has none, dragged down from
  /// `top`.
  @MainActor
  private func swipeAway(by top: XCUIElement, in app: XCUIApplication) {
    if app.buttons["Sheet Grabber"].exists {
      app.swipeSheetAway()
    } else {
      top.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5))
        .press(forDuration: 0.1, thenDragTo: app.point(across: 0.5, down: 0.99))
    }
  }

  @MainActor
  func testAListingsSheetOverTheColumnsSaysItsOwnAndTheSessionSaysItOnceItGoesWhereverItWent() {
    let app = launched("releases", ["--open", "workstation-key/7"] + standing, turned: true)
    columnsStand(app)
    saidOnce(beneath: "session.title", besideTheListing: true, in: app)
    // The sheet of Servers, Relays and This device says it itself, and nothing beneath it does.
    app.buttons["home.servers"].tap()
    let bar = app.navigationBars["Servers & device"]
    XCTAssertTrue(bar.waitForExistence(timeout: 10))
    saidOnce(beneath: "Servers & device", in: app)
    attach("Said once, by the listing's sheet over the columns")
    // Gone once the columns have collapsed beneath it, the Session over the listing says it.
    turn(.portrait, app)
    XCTAssertTrue(bar.exists, "the sheet stays up")
    swipeAway(by: bar, in: app)
    XCTAssertTrue(bar.waitForNonExistence(timeout: 10), "the sheet went")
    oneColumnStands(app, showing: "session.title")
    saidOnce(beneath: "session.title", in: app)
    attach("Said once, by the Session over the listing, once the listing's sheet went")
  }

  @MainActor
  func testANoticeBesideTheSidebarOutlastsASheetOverItAndGoesOnceItHasStoodOnScreen() {
    // Long enough that the tap that brings the sheet back and the drag that takes it away are a part of it alone.
    let stands: TimeInterval = 30
    // Opened on a Session whose Approval presents itself, so a sheet over the window that shows no notice of its own
    // soon covers this one, and one tap covers it again. It may show a moment before the sheet first comes, which is a
    // part of its while.
    let app = launched("releases", ["--open", "workstation-key/3", "--notice-stands", String(stands)], turned: true)
    let sheet = element("approval.sheet", in: app)
    XCTAssertTrue(sheet.waitForExistence(timeout: 10))
    let notice = notices(in: app).firstMatch
    XCTAssertFalse(notice.exists && notice.isHittable, "the sheet hides it")

    // What is left of its while runs from no sooner than the sheet is swiped away, and from no later than the notice is
    // next seen, so it would be over, whatever was left, its whole while after that.
    let uncovered = Date()
    swipeAway(by: element("approval.title", in: app), in: app)
    XCTAssertTrue(notice.waitForExistence(timeout: 10))
    let outstood = Date().addingTimeInterval(stands + 1)
    element("session.review", in: app).tap()
    XCTAssertTrue(sheet.waitForExistence(timeout: 10))
    XCTAssertLessThan(
      Date(), uncovered.addingTimeInterval(stands - 10), "the sheet hid it with time to spare in its while")
    // Until it would have outstood its while on screen, beneath the sheet.
    let outlasted = XCTNSPredicateExpectation(predicate: NSPredicate { _, _ in Date() >= outstood }, object: nil)
    XCTAssertEqual(XCTWaiter.wait(for: [outlasted], timeout: stands + 2), .completed)
    swipeAway(by: element("approval.title", in: app), in: app)
    XCTAssertTrue(sheet.waitForNonExistence(timeout: 10), "the sheet went")
    XCTAssertTrue(notice.waitForExistence(timeout: 10), "not seen while the sheet hid it")
    columnsStand(app)
    saidOnce(beneath: "session.title", besideTheListing: true, in: app)
    attach("The notice still standing once the sheet goes")
    XCTAssertTrue(notice.waitForNonExistence(timeout: stands + 5), "and gone once it has stood on screen")
  }
}
