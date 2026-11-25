import XCTest

/// The Duo open flat, its inner display in portrait: run by `scripts/ios-test.sh posed portrait`.
final class PosedPortraitTests: XCTestCase {
  @MainActor
  func testTheBarStandsAcrossTheHead() throws {
    try PosedWalk.walk(.portrait, self, barStandsVertically: false)
  }
}
