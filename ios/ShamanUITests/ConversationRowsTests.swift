import XCTest

/// The rows of an open Session's conversation stand one below another, each as tall as what it draws: however often
/// a Session is opened, an Agent's Message, which Textual draws empty at first and grows into place, never stands at
/// what it first measured with its words running over the rows beside it.
final class ConversationRowsTests: XCTestCase {
  override func setUp() {
    continueAfterFailure = false
  }

  /// Every kind of row the conversation draws, by the identifier it carries.
  private static let rows = [
    "session.userMessage", "session.sendingMessage", "session.agentMessage", "session.turn", "session.compaction",
    "session.error", "session.delegation", "session.queued", "session.refused", "session.mayHaveBegun",
  ]

  /// The `paired` phone's short Sessions by their titles: settled, working, stopped and failed.
  private static let titles = [
    "workstation-key/7": "Tidy zsh startup time",
    "workstation-key/1": "Draw the invite as a QR code in /serve",
    "workstation-key/8": "Stop /serve offering link-local addresses",
    "workstation-key/9": "Find the workspace that holds the deploy scripts",
  ]

  /// The rows on screen, top first, once what they draw has had a moment to settle.
  @MainActor
  private func rows(in app: XCUIApplication) -> [(label: String, frame: CGRect)] {
    let conversation = app.descendants(matching: .any)["session.conversation"]
    XCTAssertTrue(conversation.waitForExistence(timeout: 10))
    XCTAssertTrue(
      app.descendants(matching: .any)["session.turn"].waitForExistence(timeout: 10), "the conversation landed")
    Thread.sleep(forTimeInterval: 1.5)
    let visible = conversation.frame
    return Self.rows.flatMap { identifier in
      app.descendants(matching: .any).matching(identifier: identifier).allElementsBoundByIndex.map {
        (label: "\(identifier) \($0.label.prefix(30))", frame: $0.frame)
      }
    }
    .filter { $0.frame.intersects(visible) }
    .sorted { $0.frame.minY < $1.frame.minY }
  }

  @MainActor
  private func assertNoRowOverlaps(_ session: String, in app: XCUIApplication, opening: Int) {
    let rows = rows(in: app)
    XCTAssertGreaterThan(rows.count, 2, "\(session) shows its conversation")
    for (above, below) in zip(rows, rows.dropFirst()) where below.frame.minY < above.frame.maxY - 1 {
      attach("\(session), opening \(opening)")
      XCTFail(
        "\(session), opening \(opening): \(below.label) at \(below.frame) overlaps \(above.label) at \(above.frame)")
    }
  }

  /// In `longSession`, `/32`'s answer is written again and again: long under the Model's name, then short without it.
  /// Each time the row of each of its blocks comes to stand as tall as its words, neither running over what is below nor
  /// leaving anything blank beneath them.
  @MainActor
  func testAnAnswerWrittenShorterOrLongerStandsAsTallAsItsWords() throws {
    let app = XCUIApplication()
    app.launchArguments = ["--fake", "longSession", "--open", "workstation-key/32"]
    app.launch()
    let answer = app.descendants(matching: .any).matching(identifier: "session.agentMessage").firstMatch
    XCTAssertTrue(answer.waitForExistence(timeout: 10))
    for round in 0..<2 {
      for (words, long) in [("short notes end here", false), ("long notes end here", true)] {
        let said = "round \(round), \(long ? "long" : "short")"
        // Measured while it is so written, which it stays a few seconds at a time: what was read while it was
        // written again is read again.
        var measured: [Block]?
        let deadline = Date.now.addingTimeInterval(15)
        while measured == nil, Date.now < deadline {
          guard try blocks(of: app).last?.label.contains(words) == true else {
            Thread.sleep(forTimeInterval: 0.05)
            continue
          }
          // Past the moment a shorter row waits to be sure of what it measures.
          Thread.sleep(forTimeInterval: 0.6)
          let blocks = try blocks(of: app)
          if blocks.last?.label.contains(words) == true {
            measured = blocks
          }
        }
        let blocks = try XCTUnwrap(measured, "\(said): so written a while")
        XCTAssertEqual(blocks.count, long ? 8 : 1, "\(said): a row for each of its blocks")
        for block in blocks {
          if abs(block.row.height - block.words.height) > 2 {
            attach(said)
          }
          XCTAssertEqual(
            block.row.height, block.words.height, accuracy: 2,
            "\(said): its row stands as tall as its words, at \(block.row) for \(block.words): \(block.label)")
        }
        XCTAssertFalse(
          app.descendants(matching: .any)["session.model"].exists, "the Model's name went with the first writing again")
      }
    }
  }

  /// A block of an agent Message on screen: its words, and the row that holds them.
  private struct Block {
    let label: String
    let row: CGRect
    let words: CGRect
  }

  /// The blocks of agent Messages on screen, top first, all read at one moment, as blocks come and go while an answer is
  /// written.
  @MainActor
  private func blocks(of app: XCUIApplication) throws -> [Block] {
    func words(in element: any XCUIElementSnapshot) -> [any XCUIElementSnapshot] {
      element.identifier == "session.agentMessage" ? [element] : element.children.flatMap(words)
    }
    func blocks(in element: any XCUIElementSnapshot) -> [Block] {
      guard element.elementType == .cell else { return element.children.flatMap(blocks) }
      return words(in: element).map { Block(label: $0.label, row: element.frame, words: $0.frame) }
    }
    return blocks(in: try app.snapshot()).sorted { $0.row.minY < $1.row.minY }
  }

  @MainActor
  func testShortSessionsOpenedAgainAndAgainStandEachRowBelowTheLast() {
    let app = XCUIApplication()
    app.launchArguments = ["--fake", "paired", "--open", "workstation-key/7"]
    app.launch()
    assertNoRowOverlaps("workstation-key/7", in: app, opening: 0)
    let order = [
      "workstation-key/1", "workstation-key/7", "workstation-key/8", "workstation-key/7",
      "workstation-key/9", "workstation-key/7", "workstation-key/7",
    ]
    for (opening, session) in order.enumerated() {
      app.back.tap()
      let row = app.descendants(matching: .any).matching(identifier: "listing.active")
        .matching(NSPredicate(format: "label CONTAINS %@", Self.titles[session]!)).firstMatch
      XCTAssertTrue(row.waitForExistence(timeout: 10), session)
      row.tap()
      assertNoRowOverlaps(session, in: app, opening: opening + 1)
    }
  }
}
