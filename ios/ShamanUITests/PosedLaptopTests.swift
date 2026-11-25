import XCTest

/// The Duo sat like a laptop, partly folded and turned: run by `scripts/ios-test.sh posed laptop`.
final class PosedLaptopTests: XCTestCase {
  @MainActor
  func testTheBarStandsAcrossTheHead() throws {
    try PosedWalk.walk(.laptop, self, barStandsVertically: false)
  }
}
