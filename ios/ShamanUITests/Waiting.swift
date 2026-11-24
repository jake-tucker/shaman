import XCTest

extension XCUIElementQuery {
  /// Whether an element this finds comes to stand as `predicate` says, looked for twice a second, `looks` times.
  ///
  /// Counted in looks and not on the clock, as a Mac booting clones can take half a minute over one, in which the app
  /// does as little as the test. Nor is it an expectation of the element, which describes the whole screen for the
  /// report each time it looks and finds otherwise, so that on a loaded Mac one look can be all a deadline holds; and
  /// one that reads an element's label fails the test where the element goes meanwhile, as a sheet's does where another
  /// takes its place.
  @MainActor
  func waitForOne(_ predicate: NSPredicate, looks: Int = 40) -> Bool {
    let one = matching(predicate).firstMatch
    for _ in 0..<looks {
      if one.exists { return true }
      RunLoop.current.run(until: Date().addingTimeInterval(0.5))
    }
    return one.exists
  }
}
