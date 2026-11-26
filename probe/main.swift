// A window that runs the `shaman` nested in its own bundle, for issue 781.
//
//   open -n -W Probe.app --args <where> <report-file> <shaman arguments...>
//
// <where> is the directory under Contents the nested `shaman` is run from: `MacOS` or `Helpers`.
// The report file is given what `shaman` printed, how it exited and how long it took.
import AppKit

final class Delegate: NSObject, NSApplicationDelegate {
    var window: NSWindow!

    func applicationDidFinishLaunching(_ notification: Notification) {
        window = NSWindow(
            contentRect: NSRect(x: 200, y: 200, width: 420, height: 120),
            styleMask: [.titled, .closable], backing: .buffered, defer: false)
        window.title = "Shaman probe 781"
        let label = NSTextField(labelWithString: "Running the shaman nested in this bundle…")
        label.frame = NSRect(x: 20, y: 50, width: 380, height: 20)
        window.contentView?.addSubview(label)
        window.makeKeyAndOrderFront(nil)

        let arguments = Array(CommandLine.arguments.dropFirst())
        guard arguments.count >= 2 else { NSApp.terminate(nil); return }
        let shaman = Bundle.main.bundleURL
            .appendingPathComponent("Contents").appendingPathComponent(arguments[0])
            .appendingPathComponent("shaman")
        let report = URL(fileURLWithPath: arguments[1])
        DispatchQueue.global().async {
            let process = Process()
            process.executableURL = shaman
            process.arguments = Array(arguments.dropFirst(2))
            let output = Pipe()
            process.standardOutput = output
            process.standardError = output
            let started = Date()
            var text = "ran \(shaman.path) \(process.arguments!.joined(separator: " "))\n"
            do {
                try process.run()
                let data = output.fileHandleForReading.readDataToEndOfFile()
                process.waitUntilExit()
                text += String(decoding: data, as: UTF8.self)
                text += "exit \(process.terminationStatus) after \(Int(Date().timeIntervalSince(started) * 1000)) ms\n"
            } catch {
                text += "could not run: \(error)\n"
            }
            try? text.write(to: report, atomically: true, encoding: .utf8)
            DispatchQueue.main.async { NSApp.terminate(nil) }
        }
    }
}

let app = NSApplication.shared
let delegate = Delegate()
app.delegate = delegate
app.setActivationPolicy(.regular)
app.run()
