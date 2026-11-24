import XCTest

/// A long Session over the fake, `longSession`'s, of hundreds of items of very uneven height: it opens at the true
/// foot of its conversation, and scrolls through it steadily both ways.
///
/// What the screen shows is read from screenshots, and the conversation is dragged by where it stands on the screen
/// rather than as an element, so that what moves it is the drag alone. Launched by `simctl`, a conversation in a
/// SwiftUI lazy stack opened blank past its end; under XCUITest, whose accessibility client lays it out again, it did
/// not, so the opening is guarded here but that blank was seen only by `simctl`. Scrolling up through it jumped either
/// way, which the drags catch.
final class LongSessionTests: XCTestCase {
  override func setUp() {
    continueAfterFailure = false
  }

  @MainActor
  private func launched(opening session: String) -> XCUIApplication {
    let app = XCUIApplication()
    app.launchArguments = ["--fake", "longSession", "--open", session]
    app.launch()
    return app
  }

  @MainActor
  private func attach(text: String, _ name: String) {
    let attachment = XCTAttachment(string: text)
    attachment.name = name
    attachment.lifetime = .keepAlways
    add(attachment)
  }

  /// A slow drag of `points` down the screen's middle, held at its end so nothing flings.
  @MainActor
  private func drag(_ app: XCUIApplication, by points: CGFloat) {
    let start = app.point(across: 0.5, down: points > 0 ? 0.3 : 0.7)
    start.press(
      forDuration: 0.05, thenDragTo: start.withOffset(CGVector(dx: 0, dy: points)), withVelocity: .slow,
      thenHoldForDuration: 0.3)
  }

  /// Gives the list a moment to settle after a drag.
  private func settle() {
    Thread.sleep(forTimeInterval: 0.4)
  }

  @MainActor
  func testItOpensAtTheFootOfItsConversation() {
    let app = launched(opening: "workstation-key/30")
    // Long enough for the conversation to land and every row on screen to settle, untouched.
    Thread.sleep(forTimeInterval: 4)
    let opened = Screen(Display.picture())
    attach("Opened, untouched", showing: opened.picture)
    XCTAssertGreaterThan(opened.inked, 0.05, "the conversation stands on screen, not blank")

    let closing = app.descendants(matching: .any).matching(identifier: "session.agentMessage")
      .matching(NSPredicate(format: "label CONTAINS %@", "The last item of this Session")).firstMatch
    XCTAssertTrue(closing.waitForExistence(timeout: 5))
    XCTAssertTrue(closing.isHittable, "the last item is on screen")
    let composer = app.descendants(matching: .any)["session.composer"]
    XCTAssertLessThanOrEqual(closing.frame.maxY, composer.frame.minY, "above the composer")
    XCTAssertGreaterThan(closing.frame.maxY, composer.frame.minY - 60, "at the foot, not above it")
  }

  /// The conversation's last row, the answer holding `lastWords`, ends just above the composer, once it stands
  /// there and the rows have had a moment to settle.
  @MainActor
  private func assertAtTheFoot(_ app: XCUIApplication, lastWords: String, _ said: String) {
    let last = app.descendants(matching: .any).matching(identifier: "session.agentMessage")
      .matching(NSPredicate(format: "label CONTAINS %@", lastWords)).firstMatch
    XCTAssertTrue(last.waitForExistence(timeout: 15), "\(said): the last row comes on screen")
    Thread.sleep(forTimeInterval: 1.5)
    let composer = app.descendants(matching: .any)["session.composer"]
    let gap = composer.frame.minY - last.frame.maxY
    if !(0...40).contains(gap) {
      attach(said)
    }
    XCTAssertTrue((0...40).contains(gap), "\(said): the last row ends \(gap) points above the composer")
  }

  /// `/33` ends in eight one-line answers, each of which Textual draws empty first, so the conversation shrinks under
  /// its foot as it opens: the system scrolls toward the new foot over a few frames, which the list must neither ride
  /// up nor take for the reader leaving.
  @MainActor
  func testAConversationShrinkingUnderItsFootAsItOpensStandsAtItsFoot() {
    let app = launched(opening: "workstation-key/33")
    assertAtTheFoot(app, lastWords: "Tail line #7", "opened")
  }

  /// Begun while the rows just above the foot are still being measured, and on up through rows never drawn, past
  /// tall answers, tables and code blocks, then back down over them.
  @MainActor
  func testItScrollsUpAndDownSteadily() {
    let app = launched(opening: "workstation-key/30")
    Thread.sleep(forTimeInterval: 2)
    var log = ""
    var before = Screen(Display.picture())
    var moves: [CGFloat] = []
    for (direction, steps) in [(CGFloat(1), 12), (CGFloat(-1), 6)] {
      for _ in 0..<steps {
        drag(app, by: 200 * direction)
        settle()
        let after = Screen(Display.picture())
        let moved = after.shift(from: before).map { -$0 }
        log += "dragged \(Int(200 * direction)): moved \(moved.map { "\(Int($0))" } ?? "beyond a screen")\n"
        moves.append(moved.map { $0 * direction } ?? .infinity)
        if moved.map({ abs($0 - 190 * direction) > 60 }) ?? true {
          attach("Before a jump", showing: before.picture)
          attach("After a jump", showing: after.picture)
        }
        before = after
      }
    }
    attach(text: log, "Moves")
    // A drag of 200 points moves what is on screen by what the drag moved past the touch's slop, each time; the foot
    // and the head stop it short.
    let jumps = moves.filter { $0 > 230 || $0 < -10 }
    XCTAssertTrue(jumps.isEmpty, "every drag moved the conversation steadily:\n\(log)")
  }

  @MainActor
  func testItFollowsAnAnswerStreamingInButLeavesAReaderWhoScrolledUp() throws {
    let app = launched(opening: "workstation-key/31")
    let composer = app.descendants(matching: .any)["session.composer"]
    XCTAssertTrue(composer.waitForExistence(timeout: 10))
    Thread.sleep(forTimeInterval: 4)
    let grown = try XCTUnwrap(blocks(of: app).compactMap(\.step).max(), "the answer is on screen")
    Thread.sleep(forTimeInterval: 3)
    let blocks = try blocks(of: app)
    let streamed = try XCTUnwrap(blocks.compactMap(\.step).max())
    XCTAssertGreaterThan(streamed, grown, "the answer is still streaming in, its last block on screen")
    // The bar saying the Session works stands over the composer.
    let foot = app.descendants(matching: .any)["session.bar"].frame.minY
    let last = try XCTUnwrap(blocks.map(\.frame.maxY).max())
    XCTAssertEqual(last, foot - 12, accuracy: 12, "it grew at the foot, which the screen followed")

    for _ in 0..<3 {
      drag(app, by: 200)
    }
    // Long enough for the last drag to have come to rest, however busy the machine.
    Thread.sleep(forTimeInterval: 1.5)
    let scrolled = Screen(Display.picture())
    Thread.sleep(forTimeInterval: 3)
    let later = Screen(Display.picture())
    attach("Scrolled up while it streams", showing: scrolled.picture)
    attach("A while later", showing: later.picture)
    // The block the reader is in may be set again as it grows, which moves its words by a pixel here and there.
    let moved = later.shift(from: scrolled, closely: 0.2)
    XCTAssertNotNil(moved, "what the reader scrolled to is still on screen")
    // Pulled down after the answer, the reader would move by what streamed in, a line or more at a time.
    XCTAssertEqual(moved ?? .infinity, 0, accuracy: 3, "what the reader scrolled to stays where it stands")

    // What streamed in stands below, where the reader finds it.
    var reached = streamed
    for _ in 0..<10 where reached < streamed + 6 {
      app.window.swipeUp(velocity: .fast)
      reached = try self.blocks(of: app).compactMap(\.step).max() ?? 0
    }
    XCTAssertGreaterThanOrEqual(reached, streamed + 6, "it streamed on all the while")
  }

  /// A block of an agent Message on screen.
  private struct Block {
    let label: String
    let frame: CGRect

    /// Which step of the streaming answer it is, where it is one: "Step 12: …".
    var step: Int? {
      label.hasPrefix("Step ") ? Int(label.dropFirst(5).prefix { $0.isNumber }) : nil
    }
  }

  /// The blocks of agent Messages on screen, all read at one moment, as blocks come and go while an answer streams in.
  @MainActor
  private func blocks(of app: XCUIApplication) throws -> [Block] {
    func blocks(in element: any XCUIElementSnapshot) -> [Block] {
      (element.identifier == "session.agentMessage" ? [Block(label: element.label, frame: element.frame)] : [])
        + element.children.flatMap(blocks)
    }
    return blocks(in: try app.snapshot())
  }
}

/// A picture of the display the app is on, read for what it shows between the navigation bar and the composer.
@MainActor private struct Screen {
  let picture: UIImage
  /// Every other pixel row's ink, in bands across the width, from a fifth of the way down to four fifths.
  private let rows: [[Double]]
  private let scale: Double

  init(_ picture: UIImage) {
    self.picture = picture
    let image = picture.cgImage!
    // Every other pixel row is read, so a row read stands for two.
    scale = Double(image.width) / Double(picture.size.width) / 2
    let width = image.width
    let height = image.height
    let context = CGContext(
      data: nil, width: width, height: height, bitsPerComponent: 8, bytesPerRow: width * 4,
      space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)!
    context.draw(image, in: CGRect(x: 0, y: 0, width: width, height: height))
    let pixels = context.data!.bindMemory(to: UInt8.self, capacity: width * height * 4)
    let bands = 8
    rows = stride(from: height / 5, to: height * 4 / 5, by: 2).map { y in
      // CoreGraphics draws from the bottom up.
      let row = height - 1 - y
      return (0..<bands).map { band in
        var ink = 0.0
        for x in stride(from: band * width / bands, to: (band + 1) * width / bands, by: 2) {
          let offset = (row * width + x) * 4
          ink += 765 - Double(Int(pixels[offset]) + Int(pixels[offset + 1]) + Int(pixels[offset + 2]))
        }
        return ink / Double(width / bands / 2) / 765
      }
    }
  }

  /// The share of the middle that is not the plain ground.
  var inked: Double {
    let all = rows.flatMap { $0 }
    return Double(all.filter { $0 > 0.05 }.count) / Double(all.count)
  }

  /// How many points down what `before` showed has moved to stand where it stands here; nothing where no shift of
  /// less than a screen lines the two up as closely as `closely`, the mean difference of a row's ink. What moved
  /// by scrolling lines up within 0.05; text set again, as a Message streaming in is, differs by a pixel here and there.
  func shift(from before: Screen, closely: Double = 0.05) -> CGFloat? {
    let count = rows.count
    var best: (shift: Int, error: Double)?
    for shift in stride(from: -count * 2 / 3, through: count * 2 / 3, by: 1) {
      var error = 0.0
      var compared = 0
      var structure = 0.0
      for y in max(0, -shift)..<min(count, count - shift) {
        for band in 0..<rows[y].count {
          error += abs(before.rows[y][band] - rows[y + shift][band])
          structure += before.rows[y][band]
        }
        compared += 1
      }
      // Too little overlap, or too little in it to line up, says nothing.
      guard compared > count / 6, structure > 1 else { continue }
      let mean = error / Double(compared)
      if best.map({ mean < $0.error }) ?? true {
        best = (shift, mean)
      }
    }
    guard let best, best.error < closely else { return nil }
    return CGFloat(Double(best.shift) / scale)
  }
}
