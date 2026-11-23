// PROTOTYPE for #729. Throwaway: never merge. Opens a conversation in Apple's Messages and leaves it open, so that
// the Mac can bring Messages forward in each pose and measure where its field stands; and says where the elements
// near the bottom of its window are, as accessibility has them.

import XCTest

final class PrototypeFootTests: XCTestCase {
  private func say(_ words: String) {
    let name = ProcessInfo.processInfo.environment["FOOT_TAG"] ?? "messages"
    let url = URL(fileURLWithPath: "/tmp/duo729/\(name).txt")
    let data = Data((words + "\n").utf8)
    if let handle = try? FileHandle(forWritingTo: url) {
      handle.seekToEndOfFile()
      handle.write(data)
      try? handle.close()
    } else {
      try? data.write(to: url)
    }
  }

  /// Takes the keyboard down by dragging the conversation down over it, as a reader does.
  private func lower(_ messages: XCUIApplication) {
    guard messages.keyboards.firstMatch.exists else { return }
    let window = messages.windows.firstMatch
    window.coordinate(withNormalizedOffset: CGVector(dx: 0.4, dy: 0.3)).press(
      forDuration: 0.1, thenDragTo: window.coordinate(withNormalizedOffset: CGVector(dx: 0.4, dy: 0.98)),
      withVelocity: .default, thenHoldForDuration: 0.1)
    sleep(2)
    say("keyboard up after the drag: \(messages.keyboards.firstMatch.exists)")
  }

  private func measure(_ messages: XCUIApplication) {
    lower(messages)
    let window = messages.windows.firstMatch.frame
    say("window \(window)")
    for element in messages.descendants(matching: .any).allElementsBoundByIndex {
      let frame = element.frame
      guard frame.maxY > window.maxY - 140, frame.height < 200, frame.width > 0 else { continue }
      say("\(element.elementType.rawValue) '\(element.identifier)' '\(element.label.prefix(30))' \(frame) fromBottom=\(window.maxY - frame.maxY)")
    }
  }

  func testOpenMessages() {
    let messages = XCUIApplication(bundleIdentifier: "com.apple.MobileSMS")
    messages.activate()
    sleep(3)
    for name in ["Continue", "Close", "Cancel"] {
      let button = messages.buttons[name].firstMatch
      if button.exists {
        button.tap()
        sleep(1)
      }
    }
    let row = messages.staticTexts.matching(NSPredicate(format: "label CONTAINS '555'")).firstMatch
    if row.waitForExistence(timeout: 8), row.isHittable {
      row.tap()
      sleep(3)
    } else {
      say("no conversation row to tap")
    }
    measure(messages)
  }

  /// Opens the app switcher over the app as it runs and returns to it by its card, for the app's own log to say
  /// what it was told meanwhile.
  func testAppSwitcher() {
    let app = XCUIApplication()
    app.activate()
    sleep(2)
    let from = app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.999))
    let to = app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.55))
    from.press(forDuration: 0.05, thenDragTo: to, withVelocity: .slow, thenHoldForDuration: 1.2)
    sleep(2)
    say("in the switcher: app state \(app.state.rawValue)")
    let springboard = XCUIApplication(bundleIdentifier: "com.apple.springboard")
    let shot = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
    shot.lifetime = .keepAlways
    add(shot)
    springboard.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)).tap()
    sleep(4)
    say("after the card was tapped: app state \(app.state.rawValue)")
  }

  func testMeasureMessages() {
    let messages = XCUIApplication(bundleIdentifier: "com.apple.MobileSMS")
    messages.activate()
    sleep(2)
    measure(messages)
  }
}
