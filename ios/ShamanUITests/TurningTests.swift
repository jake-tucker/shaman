import XCTest

/// Turning, which is a change of size alone and changes nothing the reader did: on an iPhone 18 Pro, whose one column
/// is as wide or as short as the display, and on an iPhone 18 Pro Max, where the width class changes each turn and the
/// columns expand and collapse. And where the height is compact, the new-session screen opening with its keyboard down.
final class TurningTests: TurningTestCase {
  /// On its side and upright again, twice.
  private let twiceEachWay: [UIDeviceOrientation] = [.landscapeLeft, .portrait, .landscapeRight, .portrait]

  @MainActor
  func testASubagentsSessionStaysOverItsParentsWork() {
    let app = launched()
    open("Find what holds the session lock", in: app)
    element("session.turn", in: app).tap()
    let explorer = element("work.row", holding: "Explorer", in: app)
    XCTAssertTrue(explorer.waitForExistence(timeout: 10))
    explorer.tap()
    titled("Find what held the lock", in: app)
    for (index, held) in twiceEachWay.enumerated() {
      turn(held, app)
      titled("Find what held the lock", in: app)
      XCTAssertTrue(element("session.turn", in: app).waitForExistence(timeout: 10))
      XCTAssertTrue(app.back.isHittable)
      attach("Turned, \(index + 1)")
    }
    // What it is over is as it was, one screen at a time.
    app.back.tap()
    XCTAssertTrue(explorer.waitForExistence(timeout: 10), "back at the work")
    turn(.landscapeLeft, app)
    XCTAssertTrue(explorer.waitForExistence(timeout: 10))
    app.back.tap()
    titled("Find what holds the session lock", in: app)
    turn(.portrait, app)
    titled("Find what holds the session lock", in: app)
    app.back.tap()
    oneColumnStands(app, showing: "listing.scope")
  }

  @MainActor
  func testAnApprovalsSheetStaysUpAndGoesOnTakingTaps() {
    let moment = 20.0
    let app = launched("paired", ["--open", "workstation-key/3", "--arming-moment", String(moment)])
    let accept = element("approval.acceptOnce", in: app)
    XCTAssertTrue(accept.waitForExistence(timeout: 10))
    waitForOne("approval.acceptOnce", "isEnabled == true", in: app)
    var told = 0
    for held in twiceEachWay {
      let began = Date()
      turn(held, app)
      XCTAssertTrue(accept.isHittable, "the sheet stays up")
      let takesTaps = accept.isEnabled
      // A moment begun afresh by a turn that outlasted it, as one whose animations were miscounted does, is over by now
      // and tells nothing.
      if Date().timeIntervalSince(began) < moment {
        XCTAssertTrue(takesTaps, "its moment is not begun afresh")
        told += 1
      }
    }
    XCTAssertGreaterThan(told, 0, "no turn was shorter than the moment")
    attach("An approval turned and turned back")
    // Swiped away, it is not presented again by a turn.
    app.swipeSheetAway()
    let sheet = element("approval.sheet", in: app)
    XCTAssertTrue(sheet.waitForNonExistence(timeout: 20))
    for held in twiceEachWay {
      turn(held, app)
      XCTAssertFalse(sheet.waitForExistence(timeout: 1), "swiped away, it stays away")
      XCTAssertTrue(element("session.review", in: app).isHittable)
    }
  }

  @MainActor
  func testAQuestionnaireKeepsWhatWasHalfGiven() {
    let app = launched("paired", ["--open", "workstation-key/13"])
    let choice = { (label: String) in
      self.elements("questionnaire.choice", in: app).matching(NSPredicate(format: "label BEGINSWITH %@", label))
        .firstMatch
    }
    let next = element("questionnaire.next", in: app)
    XCTAssertTrue(next.waitForExistence(timeout: 10))
    waitForOne("questionnaire.decline", "isEnabled == true", in: app)
    choice("Staging").tap()
    next.tap()
    XCTAssertTrue(app.staticTexts["Question 2 of 3"].waitForExistence(timeout: 10))
    app.scrolled(to: choice("Soak"), over: element("questionnaire.sheet", in: app)).tap()
    for held in twiceEachWay {
      turn(held, app)
      XCTAssertTrue(app.staticTexts["Question 2 of 3"].waitForExistence(timeout: 10), "on the page it was")
      XCTAssertTrue(next.isHittable && next.isEnabled)
    }
    XCTAssertTrue(choice("Soak").isSelected)
    // And a field's writing, with its keyboard.
    let field = element("questionnaire.field", in: app)
    app.scrolled(to: field, over: element("questionnaire.sheet", in: app)).tap()
    field.typeText("the relay suites alone")
    for held in twiceEachWay {
      turn(held, app)
      XCTAssertEqual(field.value as? String, "the relay suites alone")
      waitForKeyboard(true, on: field, "the field holds the keyboard")
    }
    attach("A written answer turned and turned back")
  }

  @MainActor
  func testTheTreeStaysASheetAndAServersNameHalfWrittenStaysSo() {
    let app = launched("paired", ["--open", "workstation-key/12"])
    XCTAssertTrue(app.buttons["session.tree"].waitForExistence(timeout: 10))
    app.buttons["session.tree"].tap()
    let root = element("tree.row", holding: "Find what holds the session lock", in: app)
    XCTAssertTrue(root.waitForExistence(timeout: 10))
    for held in twiceEachWay {
      turn(held, app)
      XCTAssertTrue(root.waitForExistence(timeout: 10))
      XCTAssertTrue(root.isHittable, "the tree's sheet stays up")
    }
    app.terminate()

    let again = launched()
    again.buttons["home.servers"].tap()
    let field = again.textFields["server.workstation"]
    XCTAssertTrue(field.waitForExistence(timeout: 10))
    // At its trailing end, where the name ends.
    field.coordinate(withNormalizedOffset: CGVector(dx: 0.98, dy: 0.5)).tap()
    field.typeText(" in the attic")
    for held in twiceEachWay {
      turn(held, again)
      XCTAssertEqual(field.value as? String, "workstation in the attic")
      waitForKeyboard(true, on: field, "the name is still being written")
    }
    attach("A server's name half written, turned and turned back")
  }

  @MainActor
  func testAComposerKeepsItsTextAndItsKeyboardStaysAsItWas() {
    let app = launched("paired", ["--open", "workstation-key/7"])
    let composer = element("session.composer", in: app)
    XCTAssertTrue(composer.waitForExistence(timeout: 10))
    for held in twiceEachWay {
      turn(held, app)
      XCTAssertTrue(composer.waitForExistence(timeout: 10))
      XCTAssertFalse(holdsTheKeyboard(composer), "down, it stays down")
    }
    composer.tap()
    composer.typeText("Only the prompt")
    for held in twiceEachWay {
      turn(held, app)
      XCTAssertEqual(composer.value as? String, "Only the prompt")
      waitForKeyboard(true, on: composer, "up, it stays up")
    }
    attach("A composer turned and turned back")
  }

  @MainActor
  func testTheNewSessionScreenOpensWithItsKeyboardDownWhereTheHeightIsCompact() {
    let app = launched("composing", turned: true)
    let composer = element("landing.composer", in: app)
    // By the compose button, and once a touch has put the field's keyboard up, it stays so as it is turned.
    app.buttons["home.compose"].tap()
    XCTAssertTrue(app.staticTexts["landing.title"].waitForExistence(timeout: 10))
    XCTAssertTrue(composer.waitForExistence(timeout: 10))
    XCTAssertTrue(app.buttons["landing.elsewhere"].waitForExistence(timeout: 10))
    // Its push has ended, and with it whatever would raise the keyboard.
    XCTAssertFalse(app.keyboards.firstMatch.waitForExistence(timeout: 2))
    XCTAssertFalse(holdsTheKeyboard(composer), "it opens with its keyboard down")
    attach("The new-session screen opened on its side")
    for held in [.portrait, .landscapeRight] as [UIDeviceOrientation] {
      turn(held, app)
      XCTAssertFalse(app.keyboards.firstMatch.waitForExistence(timeout: 2))
      XCTAssertFalse(holdsTheKeyboard(composer), "and no turn raises it")
    }
    composer.tap()
    waitForKeyboard(true, on: composer, "touching its field raises it")
    for held in [.portrait, .landscapeLeft] as [UIDeviceOrientation] {
      turn(held, app)
      waitForKeyboard(true, on: composer, "and no turn lowers it")
    }

    // Upright it opens typing, at launch too, and turned it is not.
    app.terminate()
    for turned in [false, true] {
      let app = launched("composing", ["--compose"], turned: turned)
      XCTAssertTrue(composer.waitForExistence(timeout: 10))
      if turned {
        XCTAssertFalse(app.keyboards.firstMatch.waitForExistence(timeout: 2))
        XCTAssertFalse(holdsTheKeyboard(composer))
      } else {
        waitForKeyboard(true, on: composer, "elsewhere it opens typing")
        turn(.landscapeLeft, app)
        waitForKeyboard(true, on: composer, "and goes on as it is turned")
      }
      app.terminate()
    }
  }

  @MainActor
  func testTheWorkspacePickerStaysPushedOverTheNewSessionScreen() {
    let app = launched("composing", ["--compose"])
    let composer = element("landing.composer", in: app)
    XCTAssertTrue(composer.waitForExistence(timeout: 10))
    app.scrolled(to: app.buttons["landing.elsewhere"]).tap()
    let picker = element("picker.screen", in: app)
    XCTAssertTrue(picker.waitForExistence(timeout: 10))
    for held in twiceEachWay {
      turn(held, app)
      XCTAssertTrue(picker.waitForExistence(timeout: 10), "the picker stays pushed")
      XCTAssertTrue(app.back.isHittable)
    }
    app.back.tap()
    XCTAssertTrue(picker.waitForNonExistence(timeout: 10))
    XCTAssertTrue(app.staticTexts["landing.title"].waitForExistence(timeout: 10))
  }
}
