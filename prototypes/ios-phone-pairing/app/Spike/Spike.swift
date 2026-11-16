import Foundation
import Observation
import UIKit

/// One thing a followed stream said, or how it ended.
enum StreamItem: Sendable {
    case event(StreamEvent)
    case ended(String)
}

/// The core's stream listener, turned into an `AsyncStream`: the listener yields into the continuation, and the
/// stream's termination, which a cancelled consumer causes, cancels the core's handle.
func events(of connection: Connection, path: String) -> AsyncStream<StreamItem> {
    AsyncStream { continuation in
        let handle = connection.follow(path: path, listener: Forwarder(continuation))
        continuation.onTermination = { _ in handle.cancel() }
    }
}

private final class Forwarder: StreamListener {
    private let continuation: AsyncStream<StreamItem>.Continuation

    init(_ continuation: AsyncStream<StreamItem>.Continuation) {
        self.continuation = continuation
    }

    func event(event: StreamEvent) {
        continuation.yield(.event(event))
    }

    func ended(reason: String) {
        continuation.yield(.ended(reason))
        continuation.finish()
    }
}

private final class LoginForwarder: LoginListener {
    private let onStart: @Sendable (String, String, UInt64) -> Void

    init(_ onStart: @escaping @Sendable (String, String, UInt64) -> Void) {
        self.onStart = onStart
    }

    func started(verificationUri: String, userCode: String, expiresInSeconds: UInt64) {
        onStart(verificationUri, userCode, expiresInSeconds)
    }
}

/// Drives the spike a step at a time, from buttons or from `SPIKE_RUN` in the environment the app is launched
/// with, and says everything it finds in the log.
@MainActor
@Observable
final class Spike {
    var lines: [String] = []
    var invite = ProcessInfo.processInfo.environment["SPIKE_INVITE"] ?? ""
    var loginCode: String?

    @ObservationIgnored private var log: SpikeLog!
    @ObservationIgnored private var signer: EnclaveSigner?
    @ObservationIgnored private var core: Core?
    @ObservationIgnored private var connections: [Connection] = []
    @ObservationIgnored private var streams: [Task<Void, Never>] = []
    @ObservationIgnored private let sheet = LoginSheet()
    @ObservationIgnored private var running = false
    private let name = "phone-spike"

    init() {
        log = SpikeLog { [weak self] line in
            Task { @MainActor in self?.lines.append(line) }
        }
        log.line("---- launched; pid \(ProcessInfo.processInfo.processIdentifier) ----")
        let center = NotificationCenter.default
        for (notification, label) in [
            (UIApplication.willResignActiveNotification, "will resign active"),
            (UIApplication.didEnterBackgroundNotification, "did enter background"),
            (UIApplication.willEnterForegroundNotification, "will enter foreground"),
            (UIApplication.didBecomeActiveNotification, "did become active"),
        ] {
            center.addObserver(forName: notification, object: nil, queue: .main) { [weak self] _ in
                MainActor.assumeIsolated {
                    guard let self else { return }
                    let left = UIApplication.shared.backgroundTimeRemaining
                    self.log.line("lifecycle: \(label); background time remaining "
                        + (left > 1e6 ? "unbounded" : String(format: "%.1fs", left)))
                    if notification == UIApplication.willEnterForegroundNotification, !self.connections.isEmpty {
                        self.run("probe")
                    }
                }
            }
        }
        if let steps = ProcessInfo.processInfo.environment["SPIKE_RUN"], !steps.isEmpty {
            run(steps)
        }
    }

    /// Runs steps separated by commas, one after another.
    func run(_ steps: String) {
        Task {
            while running { try? await Task.sleep(for: .milliseconds(200)) }
            running = true
            defer { running = false }
            for step in steps.split(separator: ",") {
                let step = step.trimmingCharacters(in: .whitespaces)
                log.line("==== \(step) ====")
                do {
                    try await perform(step)
                } catch {
                    log.line("FAILED \(step): \(error)")
                }
            }
        }
    }

    private func perform(_ step: String) async throws {
        let parts = step.split(separator: ":", maxSplits: 1).map(String.init)
        let argument = parts.count > 1 ? parts[1] : ""
        switch parts[0] {
        case "identity":
            _ = try identity()
        case "dial":
            let answer = try await identity().plainDial(address: argument)
            log.line("dial \(argument): \(answer)")
        case "preview":
            let preview = try identity().preview(invite: invite)
            log.line("invite: \(preview.hostname) \(preview.fingerprint.prefix(16))… by \(preview.ways)")
        case "redeem":
            let record = try await identity().redeem(invite: invite, name: name, only: argument)
            pairing = record
            log.line("paired with \(record.hostname) at protocol \(record.protocolVersion); "
                + "\(record.ways.count) ways kept")
        case "connect":
            try await connect(matching: argument)
        case "probe":
            await probe()
        case "standing":
            let relay = try relay(argument)
            log.line("relay \(relay): login standing: "
                + "\(try await identity().relayStanding(relay: relay) ?? "none")")
        case "login":
            try await login(at: relay(argument))
        case "forget-login":
            try await identity().relayForget(relay: relay(argument))
            log.line("relay: login given up")
        case "nap":
            await nap()
        case "stop-streams":
            log.line("cancelling \(streams.count) stream consumer(s)")
            streams.forEach { $0.cancel() }
            streams = []
        case "drop-connections":
            connections = []
            log.line("connections dropped by Swift")
        case "wait":
            try await Task.sleep(for: .seconds(Double(argument) ?? 1))
        case "reset":
            pairing = nil
            log.line("forgot the Pairing; forgetting the key gave status \(EnclaveSigner.forget())")
        default:
            log.line("no such step")
        }
    }

    /// The core, made on first use with the Secure Enclave key behind it.
    private func identity() throws -> Core {
        if let core { return core }
        log.line("device: UIDevice name \(UIDevice.current.name), model \(UIDevice.current.model), "
            + "system \(UIDevice.current.systemVersion)")
        let signer = try EnclaveSigner(log: log)
        log.line("key: \(signer.origin)")
        let core = try Core(signer: signer, log: log)
        log.line("key: the core's SubjectPublicKeyInfo "
            + (core.publicKey() == signer.subjectPublicKeyInfo ? "equals" : "DIFFERS FROM") + " CryptoKit's")
        log.line("gethostname: \(core.hostname())")
        log.line("pairing kept: " + (pairing.map { "\($0.hostname), \($0.ways.count) ways" } ?? "none"))
        self.signer = signer
        self.core = core
        return core
    }

    private func connect(matching wanted: String) async throws {
        guard let pairing else { throw SpikeError.Failed(message: "nothing is paired") }
        let core = try identity()
        let ways = pairing.ways.filter { $0.contains(wanted) }
        var connection: Connection?
        for way in ways {
            do {
                connection = try await core.connect(pairing: pairing, way: way)
                break
            } catch {
                log.line("\(way): not connected: \(error)")
            }
        }
        guard let connection else { throw SpikeError.Failed(message: "no way matching \(wanted) connected") }
        connections.append(connection)
        let rows = try await connection.listSessions()
        log.line("\(connection.way()): \(rows.count) sessions; first: \(rows.first?.title ?? "none")")
        let wantedSession = ProcessInfo.processInfo.environment["SPIKE_SESSION"]
        let session = rows.first { $0.id == wantedSession } ?? rows.first
        let path = session.map { "/v1/sessions/\($0.id)/events" } ?? "/v1/session-events"
        follow(connection, path: path)
    }

    private func follow(_ connection: Connection, path: String) {
        let label = "\(connection.way()) \(path.suffix(20))"
        let stream = events(of: connection, path: path)
        let log = log!
        streams.append(Task {
            var count = 0
            var last = ContinuousClock.now
            for await item in stream {
                count += 1
                let gap = ContinuousClock.now - last
                last = ContinuousClock.now
                switch item {
                case .event(let event):
                    if count <= 5 || gap > .seconds(3) || count % 100 == 0 {
                        log.line("stream \(label) #\(count) after \(gap): \(event.name), \(event.bytes) bytes; "
                            + event.summary)
                    }
                case .ended(let reason):
                    log.line("stream \(label) #\(count): ended: \(reason)")
                }
            }
            log.line("stream \(label): the AsyncStream finished after \(count) items; "
                + "consumer cancelled: \(Task.isCancelled)")
        })
    }

    /// Asks every connection for the Session listing, which says what each returns after a suspension.
    private func probe() async {
        log.line("probing \(connections.count) connection(s)")
        for connection in connections {
            let started = ContinuousClock.now
            do {
                let rows = try await connection.listSessions()
                log.line("probe \(connection.way()): \(rows.count) sessions in \(ContinuousClock.now - started)")
            } catch {
                log.line("probe \(connection.way()): FAILED in \(ContinuousClock.now - started): \(error)")
            }
        }
    }

    private func relay(_ argument: String) throws -> String {
        if !argument.isEmpty { return argument }
        let relays = (pairing?.ways ?? []).compactMap { way -> String? in
            guard let data = way.data(using: .utf8),
                let read = try? JSONSerialization.jsonObject(with: data) as? [String: String]
            else { return nil }
            return read["relay"]
        }
        guard let relay = relays.first else {
            throw SpikeError.Failed(message: "the Pairing offers no Relay; name one after the step")
        }
        return relay
    }

    private func login(at relay: String) async throws {
        let core = try identity()
        let log = log!
        let listener = LoginForwarder { [weak self] uri, code, expires in
            Task { @MainActor in
                guard let self else { return }
                self.loginCode = code
                UIPasteboard.general.string = code
                log.line("login: code \(code) copied; \(expires)s to enter it at \(uri)")
                if let url = URL(string: "\(uri)?user_code=\(code)") {
                    self.sheet.open(url, log: log)
                }
            }
        }
        defer {
            sheet.close()
            loginCode = nil
        }
        let account = try await core.relayLogin(relay: relay, name: name, listener: listener)
        log.line("login: done, as \(account)")
    }

    /// Cancels a Swift task that waits on the core, to see whether the core hears of it.
    private func nap() async {
        guard let core = try? identity() else { return }
        let log = log!
        let task = Task {
            do {
                try await core.nap(seconds: 20)
                log.line("nap: the call returned")
            } catch {
                log.line("nap: the call threw \(type(of: error)): \(error)")
            }
        }
        try? await Task.sleep(for: .seconds(2))
        log.line("nap: cancelling the Swift task")
        task.cancel()
        await task.value
    }

    /// The Pairing as the app keeps it between launches.
    private var pairing: PairingRecord? {
        get {
            guard let kept = UserDefaults.standard.dictionary(forKey: "pairing"),
                let hostname = kept["hostname"] as? String,
                let serverKey = kept["serverKey"] as? Data,
                let fingerprint = kept["fingerprint"] as? String,
                let ways = kept["ways"] as? [String],
                let protocolVersion = kept["protocolVersion"] as? Int
            else { return nil }
            return PairingRecord(
                hostname: hostname, serverKey: serverKey, fingerprint: fingerprint, ways: ways,
                protocolVersion: UInt32(protocolVersion))
        }
        set {
            guard let record = newValue else {
                UserDefaults.standard.removeObject(forKey: "pairing")
                return
            }
            UserDefaults.standard.set(
                [
                    "hostname": record.hostname, "serverKey": record.serverKey,
                    "fingerprint": record.fingerprint, "ways": record.ways,
                    "protocolVersion": Int(record.protocolVersion),
                ], forKey: "pairing")
        }
    }
}
