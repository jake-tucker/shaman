import XCTest

/// How far from each edge of its column a conversation stands.
struct Sides: Equatable {
  var leading: CGFloat
  var trailing: CGFloat

  /// `frame`'s, across `column`, to half a point.
  init(_ frame: CGRect, across column: CGRect) {
    leading = ((frame.minX - column.minX) * 2).rounded() / 2
    trailing = ((column.maxX - frame.maxX) * 2).rounded() / 2
  }

  init(_ leading: CGFloat, _ trailing: CGFloat) {
    self.leading = leading
    self.trailing = trailing
  }

  /// Of what is 700 points wide in the middle of `column`, from a whole point.
  static func middle(of column: CGRect) -> Sides {
    let spare = column.width - 700
    return Sides((spare / 2).rounded(.down), spare - (spare / 2).rounded(.down))
  }
}

extension XCTestCase {
  /// Waits until the open Session's conversation stands `sides` from its column's edges: each row on screen, its foot's
  /// bar, and its composer, from its field's box to send. A user's bubble ends where they do.
  @MainActor
  func conversationReads(
    _ sides: (CGRect) -> Sides, in app: XCUIApplication, _ said: String = "", file: StaticString = #filePath,
    line: UInt = #line
  ) {
    let conversation = app.descendants(matching: .any)["session.conversation"]
    XCTAssertTrue(conversation.waitForExistence(timeout: 10), file: file, line: line)
    var stood: [String: Set<String>] = [:]
    var expected = Sides(0, 0)
    for _ in 0..<40 {
      let column = conversation.frame
      expected = sides(column)
      var all: [String: [Sides]] = [:]
      all["rows"] = conversation.cells.allElementsBoundByIndex.map { Sides($0.frame, across: column) }
      // A bubble's element is across its row but for what it keeps clear at its leading edge.
      all["bubbles"] = app.descendants(matching: .any).matching(identifier: "session.userMessage")
        .allElementsBoundByIndex.map { Sides(expected.leading, Sides($0.frame, across: column).trailing) }
      let bar = app.descendants(matching: .any)["session.bar"]
      all["bar"] = bar.exists ? [Sides(bar.frame, across: column)] : []
      all["composer"] = [composer("session", in: app, across: column)]
      stood = all.mapValues { Set($0.map { "\($0.leading), \($0.trailing)" }) }
      if all["rows"]?.isEmpty == false, all.values.allSatisfy({ $0.allSatisfy { $0 == expected } }) { return }
      RunLoop.current.run(until: Date().addingTimeInterval(0.25))
    }
    attach("The conversation out of its place")
    XCTFail("\(said) the conversation stands \(stood) and not \(expected) from its edges", file: file, line: line)
  }

  /// Where the composer of `screen`, "session" or "landing", stands across `column`: from its field's box, which is 14
  /// points wider at each side than the words read of it, to send.
  @MainActor
  func composer(_ screen: String, in app: XCUIApplication, across column: CGRect) -> Sides {
    let field = app.descendants(matching: .any).matching(identifier: "\(screen).composer").firstMatch.frame
    let send = app.buttons["\(screen).send"].frame
    return Sides(CGRect(x: field.minX - 14, y: 0, width: send.maxX - field.minX + 14, height: 1), across: column)
  }
}

extension XCUIApplication {
  /// The system's margins of `column`, which is across the window but for the vertical bar's side: 16 points a side on
  /// an ordinary iPhone upright, and beside a vertical bar 20, with none at the bar's own, where the least is 8.
  func systemMargins(of column: CGRect) -> Sides {
    let window = window.frame
    let margin: CGFloat = column.width < window.width ? 20 : 16
    return Sides(column.minX > window.minX ? 8 : margin, column.maxX < window.maxX ? 8 : margin)
  }
}
