import Foundation

/// Where every line goes: standard output, a file in the app's Documents that a Mac can copy off the phone, and the
/// screen. Lines come from any thread, the core's among them.
final class SpikeLog: LogSink, @unchecked Sendable {
    private let lock = NSLock()
    private let file: FileHandle?
    private let stamp: DateFormatter
    private let show: @Sendable (String) -> Void

    static var fileURL: URL {
        FileManager.default.urls(for: .documentDirectory, in: .userDomainMask)[0].appendingPathComponent("spike.log")
    }

    init(show: @escaping @Sendable (String) -> Void) {
        self.show = show
        stamp = DateFormatter()
        stamp.dateFormat = "HH:mm:ss.SSS"
        if !FileManager.default.fileExists(atPath: Self.fileURL.path) {
            FileManager.default.createFile(atPath: Self.fileURL.path, contents: nil)
        }
        file = try? FileHandle(forWritingTo: Self.fileURL)
        _ = try? file?.seekToEnd()
    }

    func line(text: String) {
        line(text)
    }

    func line(_ text: String) {
        lock.lock()
        let stamped = "\(stamp.string(from: Date())) \(text)"
        try? file?.write(contentsOf: Data((stamped + "\n").utf8))
        lock.unlock()
        print(stamped)
        show(stamped)
    }
}
