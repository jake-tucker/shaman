// Device Hub's windows, worked through accessibility for scripts/ios-pose.sh, which compiles this and runs it.
//
//   permitted                        whether this shell may use accessibility
//   answering <simulator>            the pid of a Device Hub that answers, one showing the simulator before another
//   running                          the pid of each Device Hub running
//   front <pid>
//   open <pid> <simulator> <udid>    opens a window on the simulator, where none shows it, and prints what closes it
//   close <pid> <window>             closes a window `open` printed
//   press <pid> <simulator> <button> presses the button whose tooltip that is, in whichever window has it
//   says <pid> <simulator>           what stands in the simulator's windows in place of its screen
//
// A Device Hub is addressed by its pid, as more than one may run and one may be stuck: System Events, which AppleScript
// goes through, turns a process back into its name, and answers for the wrong one.

import AppKit
import ApplicationServices

func attribute(_ element: AXUIElement, _ name: String) -> CFTypeRef? {
  var value: CFTypeRef?
  return AXUIElementCopyAttributeValue(element, name as CFString, &value) == .success ? value : nil
}

func text(_ element: AXUIElement, _ name: String) -> String { attribute(element, name) as? String ?? "" }

func children(_ element: AXUIElement) -> [AXUIElement] { attribute(element, "AXChildren") as? [AXUIElement] ?? [] }

/// Every element within `depth` of `root` that matches, and not those within one that does.
func all(in root: AXUIElement, depth: Int, where matches: (AXUIElement) -> Bool) -> [AXUIElement] {
  if matches(root) { return [root] }
  if depth == 0 { return [] }
  return children(root).flatMap { all(in: $0, depth: depth - 1, where: matches) }
}

/// The columns of a window: the list first and the simulator last. What stands in each is reached from here and no
/// further than it has to be, as beside a Duo's buttons stands its simulated screen, and a walk into that hangs for
/// minutes.
func columns(_ window: AXUIElement) -> [AXUIElement] {
  let splits = { (element: AXUIElement) in children(element).filter { text($0, "AXRole") == "AXSplitGroup" } }
  let groups = { (element: AXUIElement) in children(element).filter { text($0, "AXRole") == "AXGroup" } }
  return groups(window).flatMap(splits).flatMap(groups).flatMap(splits).flatMap(groups)
}

func application(_ pid: pid_t) -> AXUIElement { AXUIElementCreateApplication(pid) }

func windows(_ pid: pid_t) -> [AXUIElement] { attribute(application(pid), "AXWindows") as? [AXUIElement] ?? [] }

/// The windows showing a simulator, found by their title, "<name> – iOS <version>", and never by their place, as the
/// one in front may be another checkout's. Those on another Space are not among them.
func windows(_ pid: pid_t, showing simulator: String) -> [AXUIElement] {
  windows(pid).filter { text($0, "AXTitle").hasPrefix("\(simulator) – ") }
}

func running() -> [pid_t] {
  NSRunningApplication.runningApplications(withBundleIdentifier: "com.apple.dt.Devices").map(\.processIdentifier)
}

/// A Device Hub that answers has a menu bar, whether or not it has a window.
func answers(_ pid: pid_t) -> Bool { attribute(application(pid), "AXMenuBar") != nil }

func press(_ element: AXUIElement) -> Bool {
  AXUIElementPerformAction(element, "AXPress" as CFString) == .success
}

func wait(_ seconds: Double, until done: () -> Bool) -> Bool {
  let end = Date().addingTimeInterval(seconds)
  while Date() < end {
    if done() { return true }
    Thread.sleep(forTimeInterval: 0.5)
  }
  return done()
}

/// Has a new window show the simulator, chosen in its list by UDID.
func open(_ pid: pid_t, _ simulator: String, _ udid: String) -> String {
  if !windows(pid, showing: simulator).isEmpty { return "shown" }
  let stood = Set(windows(pid).map { text($0, "AXIdentifier") })
  guard let bar = attribute(application(pid), "AXMenuBar"),
    let item = all(in: bar as! AXUIElement, depth: 4, where: { text($0, "AXTitle") == "New Window" }).first, press(item)
  else { return "failed: no New Window in Device Hub's menus" }
  var fresh: AXUIElement?
  guard
    wait(
      10,
      until: {
        fresh = windows(pid).first { !stood.contains(text($0, "AXIdentifier")) }
        return fresh != nil
      }), let fresh
  else { return "failed: no new window came" }
  let name = text(fresh, "AXIdentifier")
  var row: AXUIElement?
  let found = wait(10) {
    let lists = columns(fresh).prefix(1).flatMap { all(in: $0, depth: 2) { text($0, "AXRole") == "AXOutline" } }
    let rows = lists.flatMap { attribute($0, "AXRows") as? [AXUIElement] ?? [] }
    row = rows.first { row in
      !all(in: row, depth: 4, where: { text($0, "AXIdentifier") == "TableRow.Device.\(udid)" }).isEmpty
    }
    return row != nil
  }
  guard found, let row,
    AXUIElementSetAttributeValue(row, "AXSelected" as CFString, kCFBooleanTrue) == .success
  else {
    close(pid, name)
    return "failed: the simulator is not in Device Hub's list"
  }
  guard wait(10, until: { !windows(pid, showing: simulator).isEmpty }) else {
    close(pid, name)
    return "failed: the window did not turn to the simulator"
  }
  return name
}

func close(_ pid: pid_t, _ name: String) {
  for window in windows(pid) where text(window, "AXIdentifier") == name {
    if let button = attribute(window, "AXCloseButton") { _ = press(button as! AXUIElement) }
  }
}

/// Presses in the first window that has the button: of several showing one simulator, one newly made may have none yet.
func press(_ pid: pid_t, _ simulator: String, _ tooltip: String) -> String {
  let shown = windows(pid, showing: simulator)
  if shown.isEmpty { return "no window" }
  for window in shown {
    let buttons = columns(window).suffix(1).flatMap(children).filter { text($0, "AXRole") == "AXButton" }
    if let button = buttons.first(where: { text($0, "AXHelp") == tooltip }) {
      return press(button) ? "pressed" : "not pressed"
    }
  }
  return "no button"
}

// Every element's, and not the application's alone.
AXUIElementSetMessagingTimeout(AXUIElementCreateSystemWide(), 4)
let words = Array(CommandLine.arguments.dropFirst())
switch words.first {
case "permitted": print(AXIsProcessTrusted())
case "running": for pid in running() { print(pid) }
case "answering":
  let answering = running().filter(answers)
  let shows = answering.first { !windows($0, showing: words[1]).isEmpty }
  if let pid = shows ?? answering.first { print(pid) }
case "front":
  AXUIElementSetAttributeValue(application(pid_t(words[1])!), "AXFrontmost" as CFString, kCFBooleanTrue)
case "open": print(open(pid_t(words[1])!, words[2], words[3]))
case "close": close(pid_t(words[1])!, words[2])
case "press": print(press(pid_t(words[1])!, words[2], words[3]))
case "says":
  let shown = windows(pid_t(words[1])!, showing: words[2])
  let said = shown.flatMap { columns($0).suffix(1).flatMap(children) }.filter { text($0, "AXRole") == "AXStaticText" }
  print(said.map { text($0, "AXValue") }.joined(separator: ", "))
default: exit(2)
}
