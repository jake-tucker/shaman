#if DEBUG
  import ShamanCore
  import Synchronization

  /// A stand-in for the core, holding canned values of the types it hands Swift, for previews, UI tests and looking
  /// at a screen without a Server. Debug builds alone carry it; a launch argument chooses it and its scenario.
  public final class FakeCore: Core {
    /// What the fake begins holding.
    public enum Scenario: String, CaseIterable, Sendable {
      /// A phone just installed, which has never been named.
      case fresh
      /// A phone that has been given a name of its own.
      case named
      /// A named phone whose records were of a key it no longer has, so it must pair again.
      case pairAgain
    }

    private let device: Mutex<ThisDevice>
    private let listeners = Mutex<[Int: AsyncStream<ThisDevice>.Continuation]>([:])
    private let nextListener = Atomic<Int>(0)

    public init(_ scenario: Scenario) {
      device = Mutex(Self.device(in: scenario))
    }

    public func thisDevice() -> AsyncStream<ThisDevice> {
      let id = nextListener.add(1, ordering: .relaxed).oldValue
      return AsyncStream(bufferingPolicy: .bufferingNewest(1)) { continuation in
        continuation.yield(device.withLock { $0 })
        listeners.withLock { $0[id] = continuation }
        continuation.onTermination = { [weak self] _ in
          _ = self?.listeners.withLock { $0.removeValue(forKey: id) }
        }
      }
    }

    public func renameDevice(_ name: String) async throws(ActError) {
      change { $0.name = name }
    }

    public func dismissPairAgain() async throws(ActError) {
      change { $0.pairAgain = nil }
    }

    private func change(_ change: (inout ThisDevice) -> Void) {
      let changed = device.withLock { device in
        change(&device)
        return device
      }
      for listener in listeners.withLock({ Array($0.values) }) {
        listener.yield(changed)
      }
    }

    private static func device(in scenario: Scenario) -> ThisDevice {
      let note =
        "Servers paired from now on know this phone by this name. Those already paired keep the name they were given."
      let fingerprint = "3f9a1c07d24e8b56a0c3e19f7d5b2a48c6e0f1d39b7a25c84e6d0f2a1b3c5d7e"
      let pairAgain =
        "This phone has a new identity key, so no server it was paired with knows it any more. Pair with each of them again."
      let name = scenario == .fresh ? "iPhone" : "Pocket"
      return ThisDevice(
        name: name, nameNote: note, fingerprint: fingerprint, pairAgain: scenario == .pairAgain ? pairAgain : nil,
        protocolVersion: 116)
    }
  }
#endif
