import XCTest

/// The Duo partly folded like a book: run by `scripts/ios-test.sh posed book`.
final class PosedBookTests: XCTestCase {
  @MainActor
  func testTheBarStandsDownTheSide() throws {
    try PosedWalk.walk(.book, self, barStandsVertically: true)
  }
}
