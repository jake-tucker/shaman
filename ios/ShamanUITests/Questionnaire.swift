import XCTest

extension XCTestCase {
  /// Checks that the field of a Questionnaire's page, on a page with choices, stands whole, its border too, between
  /// the title's bar and the foot of the page's scroll view, where the sheet's foot begins, or, where not `whole`, that
  /// its foot stands above that foot; and answers its height.
  @MainActor
  func fieldInView(
    in app: XCUIApplication, _ when: String, whole: Bool = true, file: StaticString = #filePath, line: UInt = #line
  ) -> CGFloat {
    let sheet = app.descendants(matching: .any)["questionnaire.sheet"]
    let page = sheet.scrollViews.containing(.any, identifier: "questionnaire.field").firstMatch.frame
    let bar = app.navigationBars.containing(.button, identifier: "questionnaire.decline").firstMatch.frame
    let words = app.descendants(matching: .any)["questionnaire.field"].frame
    let choices = app.descendants(matching: .any).matching(identifier: "questionnaire.choice")
    // The frame of a field of several lines is its words', inside a border that begins 12 points below the last choice.
    let border = words.minY - choices.element(boundBy: choices.count - 1).frame.maxY - 12
    XCTAssertGreaterThan(border, 0, "the words stand inside a border", file: file, line: line)
    let field = words.insetBy(dx: 0, dy: -border)
    // Where a frame rounds to a pixel.
    let rounding: CGFloat = 0.5
    if whole {
      XCTAssertGreaterThanOrEqual(
        field.minY, bar.maxY - rounding, "\(when), \(field) is clear of the bar at \(bar)", file: file, line: line)
    }
    XCTAssertLessThanOrEqual(
      field.maxY, page.maxY + rounding, "\(when), \(field) is clear of the foot of the page at \(page)", file: file,
      line: line)
    return field.height
  }
}
