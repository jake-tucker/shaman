import Vision
import XCTest

/// Where an open Session's conversation stands, as what is on screen of it says.
extension XCTestCase {
  /// The room the conversation is read in: across its column, below the bar and above what its foot holds.
  @MainActor
  func conversationsRoom(in app: XCUIApplication) -> CGRect {
    let column = app.descendants(matching: .any)["session.conversation"].frame
    let bar = app.navigationBars.containing(.staticText, identifier: "session.title").firstMatch.frame
    let work = app.descendants(matching: .any)["session.bar"]
    let foot = work.exists ? work.frame.minY : app.descendants(matching: .any)["session.composer"].frame.minY
    return CGRect(x: column.minX, y: bar.maxY, width: column.width, height: foot - 8 - bar.maxY)
  }

  /// Drags what the conversation shows down by `points`, or up, slowly and held at its end so that nothing flings, by
  /// where it stands on the screen.
  @MainActor
  func drag(_ points: CGFloat, in app: XCUIApplication) {
    let room = conversationsRoom(in: app)
    let window = app.window.frame
    let from = app.point(
      across: (room.minX + 60 - window.minX) / window.width,
      down: ((points > 0 ? room.minY + 10 : room.maxY - 10) - window.minY) / window.height)
    from.press(
      forDuration: 0.05, thenDragTo: from.withOffset(CGVector(dx: 0, dy: points)), withVelocity: .slow,
      thenHoldForDuration: 0.3)
    // Long enough for the rows that came into sight to settle.
    Thread.sleep(forTimeInterval: 1)
  }

  /// The lines of the conversation that stand whole on screen, the top one first, as a reader reads them.
  @MainActor
  func lines(in app: XCUIApplication) throws -> [String] {
    let room = conversationsRoom(in: app)
    let picture = Display.picture()
    let request = VNRecognizeTextRequest()
    request.usesLanguageCorrection = false
    try VNImageRequestHandler(cgImage: XCTUnwrap(picture.cgImage)).perform([request])
    var read = (request.results ?? []).compactMap { words -> (frame: CGRect, text: String)? in
      // Told from the foot of the picture up.
      let part = words.boundingBox
      let frame = CGRect(
        x: part.minX * picture.size.width, y: (1 - part.maxY) * picture.size.height,
        width: part.width * picture.size.width, height: part.height * picture.size.height)
      guard room.contains(frame), let text = words.topCandidates(1).first?.string else { return nil }
      return (frame, text)
    }
    read.sort { $0.frame.midY < $1.frame.midY }
    var lines: [[(frame: CGRect, text: String)]] = []
    for part in read {
      if let last = lines.last?.last, part.frame.midY < last.frame.maxY {
        lines[lines.count - 1].append(part)
      } else {
        lines.append([part])
      }
    }
    return lines.map { line in line.sorted { $0.frame.minX < $1.frame.minX }.map(\.text).joined(separator: " ") }
  }

  /// The top line reads on from where it did, to the nearest line however the words wrap: the first words of the one are
  /// in the top lines of the other.
  @MainActor
  func sameAtTheTop(
    _ before: [String], _ after: [String], file: StaticString = #filePath, line: UInt = #line
  ) {
    let begins = { (lines: [String]) in lines.first?.split(separator: " ").prefix(4).joined(separator: " ") ?? "none" }
    let top = { (lines: [String]) in lines.prefix(2).joined(separator: " ") }
    XCTAssertTrue(
      top(after).contains(begins(before)) || top(before).contains(begins(after)),
      "\(before.prefix(3)) and then \(after.prefix(3))", file: file, line: line)
  }

  /// What stands in the conversation's room, the top first: each element of a row's, by what it says, with where it is.
  @MainActor
  func standing(in app: XCUIApplication) throws -> [String] {
    let room = conversationsRoom(in: app)
    func elements(in element: any XCUIElementSnapshot) -> [any XCUIElementSnapshot] {
      let row = element.identifier.hasPrefix("session.") && element.identifier != "session.conversation"
      return (row ? [element] : []) + element.children.flatMap(elements)
    }
    let conversation = try app.descendants(matching: .any)["session.conversation"].snapshot()
    return elements(in: conversation).filter { $0.frame.intersects(room) }.sorted { $0.frame.minY < $1.frame.minY }
      .map { "\($0.identifier) at \($0.frame), \($0.label.prefix(40))" }
  }

  /// Waits until the conversation stands as `expected`, to the point and the pixel, and stays so.
  @MainActor
  func stands(
    _ expected: [String], in app: XCUIApplication, _ said: String, file: StaticString = #filePath, line: UInt = #line
  ) throws {
    XCTAssertFalse(expected.isEmpty, file: file, line: line)
    var now: [String] = []
    var stayed = 0
    for _ in 0..<20 where stayed < 2 {
      now = try standing(in: app)
      stayed = now == expected ? stayed + 1 : 0
      Thread.sleep(forTimeInterval: 0.5)
    }
    if stayed < 2 { attach("Out of its place, \(said)") }
    XCTAssertEqual(now, expected, "\(said): exactly where it stood", file: file, line: line)
  }

  /// Waits until the conversation's last row, a block of an agent Message that holds `lastWords`, ends at the foot of its
  /// room.
  @MainActor
  func standsAtItsFoot(
    _ app: XCUIApplication, lastWords: String = "", _ said: String, file: StaticString = #filePath, line: UInt = #line
  ) throws {
    func blocks(in element: any XCUIElementSnapshot) -> [any XCUIElementSnapshot] {
      (element.identifier == "session.agentMessage" ? [element] : []) + element.children.flatMap(blocks)
    }
    var gap = CGFloat.infinity
    for _ in 0..<20 {
      let foot = conversationsRoom(in: app).maxY
      let conversation = try app.descendants(matching: .any)["session.conversation"].snapshot()
      if let last = blocks(in: conversation).max(by: { $0.frame.maxY < $1.frame.maxY }),
        lastWords.isEmpty || last.label.contains(lastWords)
      {
        gap = foot - last.frame.maxY
        if (0...32).contains(gap) { return }
      }
      Thread.sleep(forTimeInterval: 0.5)
    }
    attach("Not at its foot, \(said)")
    XCTFail("\(said): the last row ends \(gap) points above the foot", file: file, line: line)
  }
}
