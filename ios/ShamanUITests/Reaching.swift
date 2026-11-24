import XCTest

/// How a test reaches what it is after, on a display of any size and wherever the bar stands.
extension XCUIApplication {
  /// Back, which is among the navigation bar's buttons only where the bar is horizontal.
  var back: XCUIElement { buttons["BackButton"] }

  /// `element`, which stands in a list, once it stands still where a tap `across` it, as a part of its width, reaches it.
  ///
  /// The list is dragged to its foot and then back to its head until it does: the one in `part`, such as a sheet or a
  /// column, or the last the app shows. A list draws little more than what is on screen, so one further along need not
  /// exist before. What is drawn later, as a screen being pushed is, is waited for first, as only the list is here.
  @discardableResult
  func scrolled(
    to element: XCUIElement, at across: CGFloat = 0.5, over part: XCUIElement? = nil, file: StaticString = #filePath,
    line: UInt = #line
  ) -> XCUIElement {
    let scrolls = NSPredicate(format: "label BEGINSWITH 'Vertical scroll bar'")
    let lists = (part ?? self).descendants(matching: .any).containing(scrolls).matching(
      NSPredicate(format: "elementType IN %@", [XCUIElement.ElementType.collectionView, .scrollView].map(\.rawValue)))
    XCTAssertTrue(lists.firstMatch.waitForExistence(timeout: 10), "a list is shown", file: file, line: line)
    let list = lists.element(boundBy: lists.count - 1)
    // A bar's button, say, which no scrolling moves.
    if element.exists {
      let name = element.identifier.isEmpty ? element.label : element.identifier
      if !name.isEmpty, !list.descendants(matching: .any)[name].exists { return element }
    }
    // It stands between whatever covers the list's head and whatever covers its foot, and says how far down the list is.
    let bar = list.otherElements.matching(scrolls).firstMatch
    for (from, to) in [(0.85, 0.15), (0.15, 0.85)] {
      var stood: String?
      for _ in 0..<50 {
        if reaches(element, at: across, beside: bar) { return element }
        let stands = "\(bar.frame) \(bar.value ?? "")"
        // A drag may raise a sheet, which moves the bar, before it moves the list; one that moves neither was the last.
        // At its head the list is not dragged on down, as that would take a sheet away.
        if stands == stood || (from < to && stands.hasSuffix(" 0%")) { break }
        stood = stands
        // Beside the bar, over the list's rows.
        let point = { (down: Double) in bar.coordinate(withNormalizedOffset: CGVector(dx: -4, dy: down)) }
        point(from).press(forDuration: 0.05, thenDragTo: point(to), withVelocity: .slow, thenHoldForDuration: 0.2)
      }
    }
    XCTFail("not reached by scrolling: \(element)", file: file, line: line)
    return element
  }

  /// Whether `element` stands still where a tap `across` it reaches it: level with `bar`, and clear of the listing's
  /// capsule, which floats over its rows and takes a touch that falls near it, where no sheet covers it.
  private func reaches(_ element: XCUIElement, at across: CGFloat, beside bar: XCUIElement) -> Bool {
    guard element.exists else { return false }
    let frame = element.frame
    Thread.sleep(forTimeInterval: 0.1)
    guard element.exists, element.frame == frame else { return false }
    let point = CGPoint(x: frame.minX + frame.width * across, y: frame.midY)
    let capsule = otherElements["home.beginsIn"]
    if capsule.exists, capsule.frame.insetBy(dx: -24, dy: -24).contains(point), capsule.isHittable { return false }
    // The bar ends short of a display's corners, which a list at its foot runs on beside.
    let shown = bar.frame
    guard point.y >= shown.minY, point.y <= shown.maxY || bar.value as? String == "100%" else { return false }
    // Asked only of what is on screen, as asked of some of what is off it, XCUITest fails the test.
    return element.isHittable
  }

  /// Swipes the sheet on top away by its grabber, as a reader does.
  func swipeSheetAway(file: StaticString = #filePath, line: UInt = #line) {
    let grabber = buttons["Sheet Grabber"]
    XCTAssertTrue(grabber.waitForExistence(timeout: 5), file: file, line: line)
    grabber.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5))
      .press(forDuration: 0.1, thenDragTo: point(across: 0.5, down: 0.99))
  }

  /// Has the app leave the front, as switching to another does: the Home button does nothing on a Duo from a test.
  func leaveTheFront(file: StaticString = #filePath, line: UInt = #line) {
    XCUIApplication(bundleIdentifier: "com.apple.Preferences").activate()
    XCTAssertTrue(
      wait(for: .runningBackground, timeout: 10) || wait(for: .runningBackgroundSuspended, timeout: 5),
      "the app left the front", file: file, line: line)
  }
}
