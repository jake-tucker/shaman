import CryptoKit
import Foundation
import Security
import ShamanCore

/// This phone's identity key: a Secure Enclave key that never leaves the phone, whose blob, which only the same chip
/// can use, is a Keychain item kept on this phone alone (ADR-0081). Every Remote pins it and every Relay Login is it,
/// so it is made on the app's own motion only where the Keychain says there is none.
public enum IdentityKey {
  /// The key, found or made, or why there is none to sign with.
  public enum Found: Sendable {
    /// The key, ready to sign.
    case ready(EnclaveSigner)
    /// The Keychain did not say whether it holds a key, or a key could not be made or kept. Nothing was made.
    case failed(String)
    /// The Keychain gave up a blob the chip will not take. Starting again under a new key is the user's to choose.
    case refused(String)
  }

  /// The key whose blob `store` keeps, made and kept there where the Keychain says there is none.
  public static func find(in store: some KeyStore = KeyItem.identity) -> Found {
    let blob: Data?
    do {
      blob = try store.read()
    } catch {
      return .failed("This phone's identity key could not be read from the Keychain: \(error.message)")
    }
    guard let blob else { return made(in: store) }
    do {
      return .ready(EnclaveSigner(try SecureEnclave.P256.Signing.PrivateKey(dataRepresentation: blob)))
    } catch {
      return .refused("The Secure Enclave will not take this phone's identity key: \(error.localizedDescription)")
    }
  }

  /// A new key in place of the one in `store`, as the user asked after the chip refused it.
  public static func startAgain(in store: some KeyStore = KeyItem.identity) -> Found {
    do {
      try store.remove()
    } catch {
      return .failed("This phone's old identity key could not be removed from the Keychain: \(error.message)")
    }
    return made(in: store)
  }

  /// A new key, kept in `store`.
  private static func made(in store: some KeyStore) -> Found {
    guard SecureEnclave.isAvailable else {
      return .failed("This phone has no Secure Enclave to keep an identity key in.")
    }
    let signer: EnclaveSigner
    do {
      signer = try EnclaveSigner.made()
    } catch {
      return .failed("The Secure Enclave could not make an identity key: \(error.localizedDescription)")
    }
    do {
      try store.add(signer.blob)
    } catch {
      return .failed("This phone's new identity key could not be kept in the Keychain: \(error.message)")
    }
    return .ready(signer)
  }
}

/// When the key may sign, fixed as it is made: at any time after the phone's first unlock since it started, on this
/// phone alone, and never on a condition of Face ID or a passcode, which every reconnection would prompt for.
enum KeyPolicy {
  static var accessibility: CFString { kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly }
  /// Signing alone: no user presence, biometry or passcode.
  static let flags: SecAccessControlCreateFlags = [.privateKeyUsage]

  static func accessControl() throws -> SecAccessControl {
    var error: Unmanaged<CFError>?
    guard let control = SecAccessControlCreateWithFlags(nil, accessibility, flags, &error) else {
      throw error!.takeRetainedValue() as Error
    }
    return control
  }
}

/// Signs for the core with a Secure Enclave key: the signer the core takes up, and the only way it reaches the key.
public final class EnclaveSigner: KeySigner {
  private let key: SecureEnclave.P256.Signing.PrivateKey

  init(_ key: SecureEnclave.P256.Signing.PrivateKey) {
    self.key = key
  }

  /// A new key, made in the Secure Enclave under `KeyPolicy`.
  static func made() throws -> EnclaveSigner {
    EnclaveSigner(try SecureEnclave.P256.Signing.PrivateKey(accessControl: try KeyPolicy.accessControl()))
  }

  /// What the Keychain keeps of the key: a blob only this phone's chip can use.
  var blob: Data {
    key.dataRepresentation
  }

  public func publicKey() -> Data {
    key.publicKey.derRepresentation
  }

  /// The chip's ECDSA signature over the SHA-256 digest of `message`, DER-encoded.
  public func sign(message: Data) throws(SignError) -> Data {
    do {
      return try key.signature(for: message).derRepresentation
    } catch {
      throw .Refused(message: "The Secure Enclave would not sign: \(error.localizedDescription)")
    }
  }
}

/// Where a key's blob is kept: the Keychain, or a stand-in for it in a test.
public protocol KeyStore: Sendable {
  /// The blob, or nothing where the item is absent.
  func read() throws(KeychainFailure) -> Data?
  /// Keeps `blob` where there is none.
  func add(_ blob: Data) throws(KeychainFailure)
  /// Removes the blob, where there is one.
  func remove() throws(KeychainFailure)
}

/// A generic-password Keychain item holding a key's blob, readable after the phone's first unlock since it started,
/// and kept on this phone alone, by no backup and no sync.
public struct KeyItem: KeyStore {
  /// The item the app keeps this phone's identity key in.
  public static let identity = KeyItem(account: "identity-key")
  /// The service every such item is kept under.
  public static let service = "com.jaketucker.shaman.identity"

  /// What names the item among the app's.
  public let account: String

  public init(account: String) {
    self.account = account
  }

  /// What finds the item.
  var query: [String: Any] {
    [
      kSecClass as String: kSecClassGenericPassword,
      kSecAttrService as String: Self.service,
      kSecAttrAccount as String: account,
    ]
  }

  public func read() throws(KeychainFailure) -> Data? {
    var query = self.query
    query[kSecReturnData as String] = true
    query[kSecMatchLimit as String] = kSecMatchLimitOne
    var found: CFTypeRef?
    let status = SecItemCopyMatching(query as CFDictionary, &found)
    switch status {
    case errSecSuccess:
      guard let blob = found as? Data else { throw KeychainFailure(status: errSecDecode) }
      return blob
    case errSecItemNotFound: return nil
    default: throw KeychainFailure(status: status)
    }
  }

  public func add(_ blob: Data) throws(KeychainFailure) {
    var item = query
    item[kSecValueData as String] = blob
    item[kSecAttrAccessible as String] = KeyPolicy.accessibility
    item[kSecAttrSynchronizable as String] = false
    let status = SecItemAdd(item as CFDictionary, nil)
    guard status == errSecSuccess else { throw KeychainFailure(status: status) }
  }

  public func remove() throws(KeychainFailure) {
    let status = SecItemDelete(query as CFDictionary)
    guard status == errSecSuccess || status == errSecItemNotFound else { throw KeychainFailure(status: status) }
  }
}

/// What the Keychain said where it did not do as asked.
public struct KeychainFailure: LocalizedError, Equatable {
  public let status: OSStatus

  public init(status: OSStatus) {
    self.status = status
  }

  /// The Keychain's own words for it.
  public var message: String {
    SecCopyErrorMessageString(status, nil) as String? ?? "error \(status)"
  }

  public var errorDescription: String? { message }
}
