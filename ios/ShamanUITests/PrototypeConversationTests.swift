// PROTOTYPE for #717 and #718. Throwaway: launches the app as the Mac asks and then does what it is told, one command at a
// time, from the file /tmp/duo719/cmd, which it removes once the command is done:
//
//   drag <from> <to>     drags the conversation between two heights, as shares of its own height
//   flick <from> <to>    the same, let go moving
//   tap <identifier>     taps the first element so named
//   label <label>        taps the first button so labelled
//   focus                taps the composer
//   type <words>         types into whatever has focus
//   quit                 ends the test
//
// For #719: `grab <share>` drags a sheet's grabber to that share of the screen's height, `field <identifier> <words>`
// taps a field and types, `frames` says where a sheet's parts stand, in the screen's points, `anytap <label>` taps the
// first element of any kind so labelled, `at <x> <y>` taps a share of the screen, `swipe <x> <y> <x> <y>` drags between two, and `relaunch <arguments>` ends the
// app and launches it again as asked.
import XCTest

final class PrototypeConversationTests: XCTestCase {
  @MainActor
  func testDriven() {
    let environment = ProcessInfo.processInfo.environment
    let app = XCUIApplication()
    app.launchArguments = (environment["PROTO_ARGS"] ?? "--fake longSession --open workstation-key/30")
      .split(separator: " ").map(String.init) + ["--forget-scene"]
    app.launch()
    let command = "/tmp/duo719/cmd"
    try? FileManager.default.removeItem(atPath: command)
    FileManager.default.createFile(atPath: "/tmp/duo719/ready", contents: Data())
    let deadline = Date().addingTimeInterval(3000)
    while Date() < deadline {
      guard let told = try? String(contentsOfFile: command, encoding: .utf8) else {
        Thread.sleep(forTimeInterval: 0.2)
        continue
      }
      let words = told.trimmingCharacters(in: .whitespacesAndNewlines).split(separator: " ", maxSplits: 1)
        .map(String.init)
      let rest = words.count > 1 ? words[1] : ""
      var said = "done"
      switch words.first ?? "" {
      case "drag", "flick":
        let shares = rest.split(separator: " ").compactMap { Double($0) }
        let list = app.descendants(matching: .any).matching(identifier: "session.conversation").firstMatch
        if shares.count == 2, list.waitForExistence(timeout: 5) {
          let from = list.coordinate(withNormalizedOffset: CGVector(dx: 0.4, dy: shares[0]))
          let to = list.coordinate(withNormalizedOffset: CGVector(dx: 0.4, dy: shares[1]))
          if words[0] == "drag" {
            from.press(forDuration: 0.05, thenDragTo: to, withVelocity: .slow, thenHoldForDuration: 0.3)
          } else {
            from.press(forDuration: 0.05, thenDragTo: to, withVelocity: .fast, thenHoldForDuration: 0)
          }
        } else {
          said = "no conversation"
        }
      case "tap":
        let found = app.descendants(matching: .any).matching(identifier: rest).firstMatch
        if found.waitForExistence(timeout: 5) { found.tap() } else { said = "no \(rest)" }
      case "label":
        let found = app.buttons.matching(NSPredicate(format: "label == %@", rest)).firstMatch
        if found.waitForExistence(timeout: 5) { found.tap() } else { said = "no \(rest)" }
      case "focus":
        let found = app.descendants(matching: .any).matching(identifier: "session.composer").firstMatch
        if found.waitForExistence(timeout: 5) { found.tap() } else { said = "no composer" }
      case "type":
        app.typeText(rest)
      case "relaunch":
        app.terminate()
        app.launchArguments = rest.split(separator: " ").map(String.init) + ["--forget-scene"]
        app.launch()
      case "grab":
        let grabber = app.buttons["Sheet Grabber"].firstMatch
        if let share = Double(rest), grabber.waitForExistence(timeout: 5) {
          grabber.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5))
            .press(
              forDuration: 0.1, thenDragTo: app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: share)),
              withVelocity: .slow, thenHoldForDuration: 0.4)
        } else {
          said = "no grabber"
        }
      case "field":
        let parts = rest.split(separator: " ", maxSplits: 1).map(String.init)
        let found = app.descendants(matching: .any).matching(identifier: parts.first ?? "").firstMatch
        if found.waitForExistence(timeout: 5) {
          found.tap()
          if parts.count > 1 { app.typeText(parts[1]) }
        } else {
          said = "no \(rest)"
        }
      case "anytap":
        let found = app.descendants(matching: .any).matching(NSPredicate(format: "label == %@", rest)).firstMatch
        if found.waitForExistence(timeout: 5) { found.tap() } else { said = "no \(rest)" }
      case "swipe":
        let shares = rest.split(separator: " ").compactMap { Double($0) }
        if shares.count == 4 {
          app.coordinate(withNormalizedOffset: CGVector(dx: shares[0], dy: shares[1]))
            .press(
              forDuration: 0.05,
              thenDragTo: app.coordinate(withNormalizedOffset: CGVector(dx: shares[2], dy: shares[3])),
              withVelocity: .default, thenHoldForDuration: 0.2)
        }
      case "at":
        let shares = rest.split(separator: " ").compactMap { Double($0) }
        if shares.count == 2 {
          app.coordinate(withNormalizedOffset: CGVector(dx: shares[0], dy: shares[1])).tap()
        }
      case "frames":
        func line(_ name: String, _ element: XCUIElement) -> String? {
          guard element.exists else { return nil }
          let f = element.frame
          return String(
            format: "%@ x %.1f y %.1f w %.1f h %.1f (to x %.1f y %.1f)", name, f.minX, f.minY, f.width, f.height,
            f.maxX, f.maxY)
        }
        var lines: [String] = []
        lines.append(line("window", app.windows.firstMatch) ?? "no window")
        for identifier in [
          "approval.sheet", "approval.title", "questionnaire.sheet", "questionnaire.decline",
          "questionnaire.question", "questionnaire.next", "questionnaire.field", "tree.sheet", "tree.count",
          "login.cancel", "login.code", "session.composer", "session.conversation",
        ] {
          if let said = line(identifier, app.descendants(matching: .any).matching(identifier: identifier).firstMatch) {
            lines.append(said)
          }
        }
        if let said = line("grabber", app.buttons["Sheet Grabber"].firstMatch) { lines.append(said) }
        for bar in app.navigationBars.allElementsBoundByIndex {
          if let said = line("bar[\(bar.identifier)]", bar) { lines.append(said) }
          for text in bar.staticTexts.allElementsBoundByIndex {
            if let said = line("  bar text[\(text.label)]", text) { lines.append(said) }
          }
          for button in bar.buttons.allElementsBoundByIndex {
            if let said = line("  bar button[\(button.label)]", button) { lines.append(said) }
          }
        }
        for bar in app.toolbars.allElementsBoundByIndex {
          if let said = line("toolbar[\(bar.identifier)]", bar) { lines.append(said) }
          for button in bar.buttons.allElementsBoundByIndex {
            if let said = line("  toolbar button[\(button.label)]", button) { lines.append(said) }
          }
        }
        said = lines.joined(separator: "\n")
      case "texts":
        said = app.staticTexts.allElementsBoundByIndex.prefix(60).map { "\($0.label)|\($0.identifier)" }
          .joined(separator: "\n")
      case "buttons":
        said = app.buttons.allElementsBoundByIndex.map { "\($0.label)|\($0.identifier)" }.joined(separator: "\n")
      case "quit":
        try? FileManager.default.removeItem(atPath: command)
        return
      default:
        said = "unknown"
      }
      try? said.write(toFile: "/tmp/duo719/said", atomically: true, encoding: .utf8)
      try? FileManager.default.removeItem(atPath: command)
    }
  }
}
