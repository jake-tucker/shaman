// PROTOTYPE for #717 and #718. Throwaway: launches the app as the Mac asks and then does what it is told, one command at a
// time, from the file /tmp/duo718/cmd, which it removes once the command is done:
//
//   drag <from> <to>     drags the conversation between two heights, as shares of its own height
//   flick <from> <to>    the same, let go moving
//   tap <identifier>     taps the first element so named
//   label <label>        taps the first button so labelled
//   focus                taps the composer
//   type <words>         types into whatever has focus
//   quit                 ends the test
import XCTest

final class PrototypeConversationTests: XCTestCase {
  @MainActor
  func testDriven() {
    let environment = ProcessInfo.processInfo.environment
    let app = XCUIApplication()
    app.launchArguments = (environment["PROTO_ARGS"] ?? "--fake longSession --open workstation-key/30")
      .split(separator: " ").map(String.init) + ["--forget-scene"]
    app.launch()
    let command = "/tmp/duo718/cmd"
    try? FileManager.default.removeItem(atPath: command)
    FileManager.default.createFile(atPath: "/tmp/duo718/ready", contents: Data())
    let deadline = Date().addingTimeInterval(1500)
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
      case "buttons":
        said = app.buttons.allElementsBoundByIndex.map { "\($0.label)|\($0.identifier)" }.joined(separator: "\n")
      case "quit":
        try? FileManager.default.removeItem(atPath: command)
        return
      default:
        said = "unknown"
      }
      try? said.write(toFile: "/tmp/duo718/said", atomically: true, encoding: .utf8)
      try? FileManager.default.removeItem(atPath: command)
    }
  }
}
