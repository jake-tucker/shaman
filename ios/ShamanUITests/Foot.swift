import XCTest

extension XCTestCase {
  /// Waits until the box the composer's field `identifier` is drawn in, a line tall, ends `points` above the display's
  /// bottom edge, and stays there.
  @MainActor
  func fieldStands(
    _ points: CGFloat, aboveTheEdgeIn app: XCUIApplication, _ identifier: String, _ said: String = "",
    file: StaticString = #filePath, line: UInt = #line
  ) {
    fieldStands(points, above: { app.window.frame.maxY }, in: app, identifier, said, file: file, line: line)
  }

  /// Waits until that box ends 8 points above the software keyboard, with its predictions, fully up.
  @MainActor
  func fieldStandsOnTheKeyboard(
    in app: XCUIApplication, _ identifier: String, _ said: String = "", file: StaticString = #filePath,
    line: UInt = #line
  ) {
    let keyboard = app.otherElements["inputView"].firstMatch
    XCTAssertTrue(keyboard.waitForExistence(timeout: 10), "the software keyboard is up", file: file, line: line)
    fieldStands(8, above: { keyboard.frame.minY }, in: app, identifier, said, file: file, line: line)
    XCTAssertLessThan(keyboard.frame.minY, app.window.frame.maxY - 100, "fully up", file: file, line: line)
  }

  @MainActor
  private func fieldStands(
    _ points: CGFloat, above edge: () -> CGFloat, in app: XCUIApplication, _ identifier: String, _ said: String,
    file: StaticString, line: UInt
  ) {
    let field = app.descendants(matching: .any).matching(identifier: identifier).firstMatch
    XCTAssertTrue(field.waitForExistence(timeout: 10), file: file, line: line)
    var stood: [CGFloat] = []
    for _ in 0..<40 {
      // What is read of it is its words' frame, in the middle of a box 40 points tall, as Messages' is.
      stood.append(edge() - (field.frame.midY + 20))
      if stood.count > 1, stood.suffix(2).allSatisfy({ abs($0 - points) < 0.5 }) { return }
      RunLoop.current.run(until: Date().addingTimeInterval(0.25))
    }
    attach("The field out of its place")
    XCTFail("\(said) the field stands \(stood.suffix(3)) and not \(points) above", file: file, line: line)
  }
}
