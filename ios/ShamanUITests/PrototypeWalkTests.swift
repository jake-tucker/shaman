// PROTOTYPE for #714. Throwaway: taps through a variant and asks a watcher on the Mac for a screenshot of each
// step, by a file under /tmp/duo714/req the watcher removes once it has taken it.
import XCTest

final class PrototypeWalkTests: XCTestCase {
  override func setUp() {
    continueAfterFailure = true
  }

  private var variant: String { ProcessInfo.processInfo.environment["PROTO_VARIANT"] ?? "two" }
  private var extra: [String] {
    (ProcessInfo.processInfo.environment["PROTO_ARGS"] ?? "").split(separator: " ").map(String.init)
  }

  @MainActor
  private func launched(_ arguments: [String]) -> XCUIApplication {
    let app = XCUIApplication()
    app.launchArguments = arguments + ["--inner", variant, "--inner-pill", "off"] + extra
    app.launch()
    return app
  }

  @MainActor
  private func capture(_ name: String, settle: TimeInterval = 1.5) {
    Thread.sleep(forTimeInterval: settle)
    let request = "/tmp/duo714/req/\(variant)-\(name)"
    FileManager.default.createFile(atPath: request, contents: Data())
    let deadline = Date().addingTimeInterval(30)
    while FileManager.default.fileExists(atPath: request), Date() < deadline {
      Thread.sleep(forTimeInterval: 0.2)
    }
  }

  @MainActor
  private func element(_ identifier: String, in app: XCUIApplication) -> XCUIElement {
    app.descendants(matching: .any).matching(identifier: identifier).firstMatch
  }

  @MainActor
  @discardableResult
  private func tap(_ identifier: String, in app: XCUIApplication, timeout: TimeInterval = 8) -> Bool {
    let found = element(identifier, in: app)
    guard found.waitForExistence(timeout: timeout), found.isHittable else {
      note("no \(identifier)")
      return false
    }
    found.tap()
    return true
  }

  @MainActor
  @discardableResult
  private func back(in app: XCUIApplication) -> Bool {
    let back = app.buttons.matching(NSPredicate(format: "label == 'Back' OR identifier == 'BackButton'")).firstMatch
    guard back.waitForExistence(timeout: 3), back.isHittable else {
      note("no Back")
      return false
    }
    back.tap()
    return true
  }

  private var noted: [String] = []
  @MainActor
  private func note(_ text: String) {
    noted.append(text)
    try? noted.joined(separator: "\n").write(
      toFile: "/tmp/duo714/notes/\(variant)-\(name.filter(\.isLetter)).txt", atomically: true, encoding: .utf8)
  }

  /// The listing, a Session, its Turn's work, an Activity's whole, and back again.
  @MainActor
  func testWalk() {
    let app = launched(["--fake", "paired", "--forget-scene"])
    XCTAssertTrue(element("listing.active", in: app).waitForExistence(timeout: 10))
    capture("1-listing")
    tap("listing.active", in: app)
    capture("2-session", settle: 3)
    note("buttons after open: \(app.buttons.allElementsBoundByIndex.map { "\($0.label)|\($0.identifier)" })")
    tap("session.turn", in: app)
    capture("3-work", settle: 2.5)
    tap("work.row", in: app)
    capture("4-detail", settle: 2.5)
    note("buttons at detail: \(app.buttons.allElementsBoundByIndex.map { "\($0.label)|\($0.identifier)" })")
    back(in: app)
    capture("5-back-once")
    back(in: app)
    capture("6-back-twice")
    back(in: app)
    capture("7-back-thrice")
    note("listing after backs: \(element("listing.active", in: app).isHittable)")
  }

  /// A Session opened and left by Back, twice over, and by the edge swipe.
  @MainActor
  func testOpenBack() {
    let app = launched(["--fake", "paired", "--forget-scene"])
    XCTAssertTrue(element("listing.active", in: app).waitForExistence(timeout: 10))
    tap("listing.active", in: app)
    Thread.sleep(forTimeInterval: 2)
    note("back 1: \(back(in: app))")
    Thread.sleep(forTimeInterval: 1.5)
    note("listing after back 1: \(element("listing.active", in: app).isHittable)")
    capture("b1")
    tap("listing.active", in: app)
    Thread.sleep(forTimeInterval: 2)
    tap("session.turn", in: app)
    Thread.sleep(forTimeInterval: 2)
    note("back 2a: \(back(in: app))")
    Thread.sleep(forTimeInterval: 1.5)
    capture("b2a")
    note("back 2b: \(back(in: app))")
    Thread.sleep(forTimeInterval: 1.5)
    note("listing after back 2: \(element("listing.active", in: app).isHittable)")
    capture("b2b")
    if tap("listing.active", in: app) {
      Thread.sleep(forTimeInterval: 2)
      let window = app.windows.firstMatch
      window.coordinate(withNormalizedOffset: CGVector(dx: 0.0, dy: 0.5))
        .press(forDuration: 0.05, thenDragTo: window.coordinate(withNormalizedOffset: CGVector(dx: 0.8, dy: 0.5)))
      Thread.sleep(forTimeInterval: 1.5)
      note("listing after swipe: \(element("listing.active", in: app).isHittable)")
      capture("b3")
    }
  }

  /// The new-session screen from the compose button, the Workspace Picker over it, and a Session begun.
  @MainActor
  func testCompose() {
    let app = launched(["--fake", "composing", "--forget-scene"])
    XCTAssertTrue(element("listing.active", in: app).waitForExistence(timeout: 10))
    tap("home.compose", in: app)
    capture("c1-compose", settle: 2.5)
    tap("landing.elsewhere", in: app)
    capture("c2-picker", settle: 2.5)
    back(in: app)
    capture("c3-back")
    let field = element("landing.composer", in: app)
    if field.waitForExistence(timeout: 3) {
      field.tap()
      field.typeText("Begin from the prototype")
      tap("landing.send", in: app)
      capture("c4-begun", settle: 4)
    } else {
      note("no composer field")
    }
    back(in: app)
    capture("c5-back")
  }

  /// A Session with Subagents: its tree, and a Subagent's Session chosen there.
  @MainActor
  func testTree() {
    let app = launched(["--fake", "paired", "--forget-scene", "--open", "workstation-key/12"])
    capture("t1-session", settle: 4)
    tap("session.tree", in: app)
    capture("t2-tree", settle: 2.5)
    let rows = app.descendants(matching: .any).matching(identifier: "tree.row")
    if rows.count > 2 {
      rows.element(boundBy: 2).tap()
      capture("t3-subagent", settle: 3)
      back(in: app)
      capture("t4-back")
    } else {
      note("tree rows: \(rows.count)")
    }
  }

  /// Opens a Session and holds, for the device to be posed around it.
  @MainActor
  func testHold() {
    let session = ProcessInfo.processInfo.environment["PROTO_SESSION"] ?? "workstation-key/1"
    let scenario = ProcessInfo.processInfo.environment["PROTO_SCENARIO"] ?? "paired"
    let app = launched(["--fake", scenario, "--forget-scene"])
    XCTAssertTrue(element("listing.active", in: app).waitForExistence(timeout: 10))
    if session != "none" {
      let rows = app.descendants(matching: .any).matching(identifier: "listing.active")
      rows.element(boundBy: Int(ProcessInfo.processInfo.environment["PROTO_ROW"] ?? "0") ?? 0).tap()
    }
    Thread.sleep(forTimeInterval: 2)
    let hold = "/tmp/duo714/req/hold"
    FileManager.default.createFile(atPath: hold, contents: Data())
    let deadline = Date().addingTimeInterval(600)
    while FileManager.default.fileExists(atPath: hold), Date() < deadline {
      Thread.sleep(forTimeInterval: 0.3)
    }
  }
}
