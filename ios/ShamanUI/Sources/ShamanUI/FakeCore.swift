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
      let renamed = device.withLock { device in
        device.name = name
        return device
      }
      for listener in listeners.withLock({ Array($0.values) }) {
        listener.yield(renamed)
      }
    }

    private static func device(in scenario: Scenario) -> ThisDevice {
      let note =
        "Servers paired from now on know this phone by this name. Those already paired keep the name they were given."
      return switch scenario {
      case .fresh: ThisDevice(name: "iPhone", nameNote: note, protocolVersion: 116)
      case .named: ThisDevice(name: "Pocket", nameNote: note, protocolVersion: 116)
      }
    }
  }
#endif
