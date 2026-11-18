import Foundation
import ShamanCore

/// The thin Client's core itself, kept in a directory of the app's container.
public final class LiveCore: Core {
  private let client: ThinClient

  /// The core kept in `directory`, which `CoreDirectory.prepare` makes.
  public init(directory: URL) throws(OpenError) {
    do {
      client = try ThinClient.open(directory: directory.path(percentEncoded: false))
    } catch let error as OpenError {
      throw error
    } catch {
      throw .Failed(message: "This phone's records could not be read: \(error)")
    }
  }

  public func thisDevice() -> AsyncStream<ThisDevice> {
    listened { send in client.watchDevice(listener: DeviceForwarder(send)) }
  }

  public func renameDevice(_ name: String) async throws(ActError) {
    do {
      try await client.renameDevice(name: name)
    } catch let error as ActError {
      throw error
    } catch {
      throw .Refused(message: "This phone could not do that: \(error)")
    }
  }
}
