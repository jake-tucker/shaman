import CryptoKit
import Foundation
import Security

/// The identity key: a P-256 key made in the Secure Enclave, which signs and never leaves (ADR-0081). What is kept
/// in the Keychain is the enclave's own wrapped blob, which only this device's enclave can turn back into the key.
final class EnclaveSigner: Signer {
    private let key: SecureEnclave.P256.Signing.PrivateKey
    private let log: SpikeLog
    /// How the key came to be on this launch.
    let origin: String

    private static let service = "shaman-phone-spike"
    private static let account = "identity-key"

    init(log: SpikeLog) throws {
        self.log = log
        if let blob = Self.storedBlob() {
            key = try SecureEnclave.P256.Signing.PrivateKey(dataRepresentation: blob)
            origin = "made again from the \(blob.count)-byte blob the Keychain held"
        } else {
            var failure: Unmanaged<CFError>?
            guard let access = SecAccessControlCreateWithFlags(
                nil, kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly, [.privateKeyUsage], &failure)
            else {
                throw SignerError.Failed(message: "no access control: \(String(describing: failure))")
            }
            key = try SecureEnclave.P256.Signing.PrivateKey(accessControl: access)
            let status = Self.store(key.dataRepresentation)
            origin = "new; storing its \(key.dataRepresentation.count)-byte blob gave status \(status)"
        }
    }

    func publicKey() -> Data {
        key.publicKey.x963Representation
    }

    func sign(message: Data) throws -> Data {
        let started = ContinuousClock.now
        do {
            let signature = try key.signature(for: message).derRepresentation
            log.line("signer: signed \(message.count) bytes in \(ContinuousClock.now - started) on "
                + (Thread.isMainThread ? "the main thread" : "a background thread"))
            return signature
        } catch {
            log.line("signer: REFUSED after \(ContinuousClock.now - started): \(error)")
            throw SignerError.Failed(message: "\(error)")
        }
    }

    /// The key as CryptoKit writes its SubjectPublicKeyInfo, to compare with what the core derives.
    var subjectPublicKeyInfo: Data {
        key.publicKey.derRepresentation
    }

    private static var query: [String: Any] {
        [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account,
        ]
    }

    private static func storedBlob() -> Data? {
        var query = query
        query[kSecReturnData as String] = true
        var found: CFTypeRef?
        guard SecItemCopyMatching(query as CFDictionary, &found) == errSecSuccess else { return nil }
        return found as? Data
    }

    private static func store(_ blob: Data) -> OSStatus {
        var item = query
        item[kSecValueData as String] = blob
        item[kSecAttrAccessible as String] = kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly
        return SecItemAdd(item as CFDictionary, nil)
    }

    /// Forgets the key. The enclave keeps nothing of its own, so this is the key's end.
    static func forget() -> OSStatus {
        SecItemDelete(query as CFDictionary)
    }
}
