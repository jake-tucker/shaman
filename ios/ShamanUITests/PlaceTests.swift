import XCTest

/// A conversation keeps its place as the device is turned, which changes how wide it stands: on an iPhone 18 Pro, whose
/// one column is as wide as it reads once turned, and on an iPhone 18 Pro Max, where the Sidebar comes and goes beside
/// it. Over the fake's long Sessions, whose rows are of very uneven height.
///
/// What is at the top of the screen is read from screenshots, as a reader reads it, since no element is a line.
final class PlaceTests: TurningTestCase {
  private let settled = "workstation-key/30"
  private let streaming = "workstation-key/31"

  @MainActor
  private func opened(_ session: String, turned: Bool = false) -> XCUIApplication {
    let app = launched("longSession", ["--open", session], turned: turned)
    XCTAssertTrue(element("session.agentMessage", in: app).waitForExistence(timeout: 10))
    Thread.sleep(forTimeInterval: 2)
    return app
  }

  /// Where a reader of the long answers' prose is: the next paragraph to begin, and how many words of the one before it
  /// are still to come.
  private struct InProse: Equatable {
    let next: Int
    let words: Int

    init?(_ lines: [String]) {
      let lead = /Paragraph (\d) of the/
      guard let index = lines.firstIndex(where: { $0.contains(lead) }), let match = lines[index].firstMatch(of: lead),
        let next = Int(match.1)
      else { return nil }
      self.next = next
      words = lines[..<index].map(InProse.words).reduce(0, +)
    }

    static func words(in line: String) -> Int {
      line.split(separator: " ").count
    }
  }

  @MainActor
  func testInProseTheSameLineIsAtTheTop() throws {
    let app = opened(settled)
    var placed = 0
    for _ in 0..<30 where placed < 3 {
      drag(150, in: app)
      let lines = try lines(in: app)
      // Part-way through a paragraph, the next one beginning on screen either way up.
      guard let before = InProse(lines), (8...50).contains(before.words) else { continue }
      placed += 1
      attach("In prose, \(placed)")
      for side in [UIDeviceOrientation.landscapeLeft, .landscapeRight] {
        turn(side, app)
        Thread.sleep(forTimeInterval: 1.5)
        attach("In prose, \(placed), turned")
        let turned = try self.lines(in: app)
        let after = try XCTUnwrap(InProse(turned), "prose is at the top: \(turned)")
        XCTAssertEqual(after.next, before.next, "\(turned)")
        let line = turned.map(InProse.words).max() ?? 0
        XCTAssertLessThanOrEqual(abs(after.words - before.words), line, "to the nearest line: \(lines), \(turned)")
        turn(.portrait, app)
        Thread.sleep(forTimeInterval: 1.5)
        XCTAssertEqual(InProse(try self.lines(in: app)), before, "and back")
      }
    }
    XCTAssertEqual(placed, 3, "prose was come to")
  }

  /// The number of the line of code at the top, as the fake numbers them, where one is and several follow it.
  private func codeAtTheTop(_ lines: [String]) -> Int? {
    let numbered = /row ?_ ?(\d+)/
    let code = lines.prefix(5).map { $0.firstMatch(of: numbered).flatMap { Int($0.1) } }
    // A comment's number may be cut off where the line is, and every fourth line is one.
    guard code.count == 5, code.compactMap({ $0 }).count >= 3, let index = code.firstIndex(where: { $0 != nil }),
      index < 2, let number = code[index]
    else { return nil }
    return number - index
  }

  @MainActor
  func testInAMessageHeavyWithCodeTheSameLineIsAtTheTop() throws {
    let app = opened(settled)
    var placed = 0
    for _ in 0..<30 where placed < 3 {
      drag(230, in: app)
      let lines = try lines(in: app)
      guard let before = codeAtTheTop(lines), before >= 2 else { continue }
      placed += 1
      attach("In code, \(placed)")
      for side in [UIDeviceOrientation.landscapeLeft, .landscapeRight] {
        turn(side, app)
        Thread.sleep(forTimeInterval: 1.5)
        attach("In code, \(placed), turned")
        let turned = try self.lines(in: app)
        let after = try XCTUnwrap(codeAtTheTop(turned), "code is at the top: \(turned)")
        XCTAssertLessThanOrEqual(
          abs(after - before), 1, "the same line, or one cut off by the bar: \(lines), \(turned)")
        turn(.portrait, app)
        Thread.sleep(forTimeInterval: 1.5)
        XCTAssertEqual(codeAtTheTop(try self.lines(in: app)), before, "and back")
      }
    }
    XCTAssertEqual(placed, 3, "code was come to")
  }

  @MainActor
  func testARoundTripLandsExactlyThreeTimesOver() throws {
    let app = opened(settled)
    for stop in 0..<2 {
      for _ in 0..<(stop + 2) { drag(230, in: app) }
      let began = try standing(in: app)
      attach("Place \(stop)")
      for round in 0..<3 {
        turn(round % 2 == 0 ? .landscapeLeft : .landscapeRight, app)
        turn(.portrait, app)
        try stands(began, in: app, "place \(stop), round \(round)")
      }
      attach("Place \(stop), three round trips on")
    }
    // The reverse, from the place a scroll on its side comes to.
    turn(.landscapeLeft, app)
    for _ in 0..<2 { drag(150, in: app) }
    let began = try standing(in: app)
    for round in 0..<3 {
      turn(.portrait, app)
      turn(.landscapeLeft, app)
      try stands(began, in: app, "from its side, round \(round)")
    }
  }

  /// Whether `row` is of the answer that streams in, whose first block is headed so.
  private func streams(_ row: String) -> Bool {
    row.contains("Summary") || row.contains("The transcript now draws")
  }

  @MainActor
  func testAReaderAwayFromTheFootKeepsTheirPlaceAsAMessageStreamsIn() throws {
    let app = opened(streaming)
    // Past the head of the answer that streams in, into the one before it.
    var began: [String] = []
    for _ in 0..<12 where !began.contains(where: { $0.contains("Paragraph") }) || began.contains(where: streams) {
      drag(230, in: app)
      began = try standing(in: app)
    }
    XCTAssertFalse(began.contains(where: streams), "\(began)")
    let words = try lines(in: app)
    for round in 0..<3 {
      turn(.landscapeLeft, app)
      // While it grows by several blocks.
      Thread.sleep(forTimeInterval: 4)
      attach("Streaming, round \(round)")
      sameAtTheTop(words, try lines(in: app))
      turn(.portrait, app)
      try stands(began, in: app, "round \(round)")
    }
  }

  @MainActor
  func testAtItsFootItStaysAtItsFoot() throws {
    for (session, lastWords) in [(settled, "That's everything"), (streaming, "")] {
      let app = opened(session)
      for held in [UIDeviceOrientation.landscapeLeft, .portrait, .landscapeRight, .portrait] {
        turn(held, app)
        try standsAtItsFoot(app, lastWords: lastWords, "\(session) \(held.isLandscape ? "turned" : "upright")")
      }
      app.terminate()
    }
  }
}
