import Foundation
import ShamanCore

/// The thin Client's core itself, kept in a directory of the app's container.
public final class LiveCore: Core {
  private let client: ThinClient

  /// The core kept in `directory`, which `CoreDirectory.prepare` makes, proving this phone by the key `signer` signs
  /// with. Whatever the directory holds of another key is discarded as it opens, all but the name.
  public init(directory: URL, signer: any KeySigner) throws(OpenError) {
    do {
      client = try ThinClient.open(directory: directory.path(percentEncoded: false), signer: signer)
    } catch let error as OpenError {
      throw error
    } catch {
      throw .Failed(message: "This phone's records could not be read: \(error)")
    }
  }

  public func thisDevice() -> AsyncStream<ThisDevice> {
    listened { send in client.watchDevice(listener: Forwarder(send)) }
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

  public func dismissPairAgain() async throws(ActError) {
    do {
      try await client.dismissPairAgain()
    } catch let error as ActError {
      throw error
    } catch {
      throw .Refused(message: "This phone could not do that: \(error)")
    }
  }

  public func outlook() -> AsyncStream<Outlook> {
    listened { send in client.watchOutlook(listener: Forwarder(send)) }
  }

  public func servers() -> AsyncStream<Servers> {
    listened { send in client.watchServers(listener: Forwarder(send)) }
  }

  public func readInvite(_ invite: String) -> InviteReading {
    client.readInvite(invite: invite)
  }

  public func confirmPairing(invite: String, remoteName: String, deviceName: String) async -> PairingOutcome {
    await client.confirmPairing(invite: invite, remoteName: remoteName, deviceName: deviceName)
  }
}

/// The live core over the key `IdentityKey` found, or why there is none.
public enum Opening {
  case core(LiveCore)
  /// The records could not be read or kept, or there is no key to open them under.
  case failed(String)
  /// The key cannot prove this phone: the chip will not take its blob, or it will not sign. Starting again under a
  /// new key is the user's to choose.
  case keyRefused(String)

  /// The core kept in `directory`, over the key `found`.
  public static func of(_ found: IdentityKey.Found, in directory: URL) -> Opening {
    switch found {
    case .ready(let signer): over(signer, in: directory)
    case .failed(let message): .failed(message)
    case .refused(let message): .keyRefused(message)
    }
  }

  /// The core kept in `directory`, over the key `signer` signs with.
  static func over(_ signer: any KeySigner, in directory: URL) -> Opening {
    do {
      return .core(try LiveCore(directory: directory, signer: signer))
    } catch .KeyRefused(let message) {
      return .keyRefused(message)
    } catch {
      return .failed(error.message)
    }
  }
}
