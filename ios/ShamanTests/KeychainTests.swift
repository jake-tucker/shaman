import Foundation
import Security
import ShamanUI
import Testing

/// The Keychain item the identity key's blob is kept in, as the app reaches it. These run hosted in the app, whose
/// signature carries the entitlement the Keychain asks of whatever reaches it; the package's own tests have none.
@Suite struct KeychainTests {
  /// An item of the test's own, removed afterwards.
  private let item = KeyItem(account: "test-\(UUID().uuidString)")

  @Test func anAbsentItemIsSaidToBeAbsentAndAKeptBlobIsReadBack() throws {
    defer { try? item.remove() }
    #expect(try item.read() == nil)
    let blob = Data("a blob".utf8)
    try item.add(blob)
    #expect(try item.read() == blob)
    #expect(throws: KeychainFailure(status: errSecDuplicateItem)) { try item.add(blob) }
    try item.remove()
    #expect(try item.read() == nil)
    try item.remove()
  }

  @Test func aKeyIsMadeWhereTheItemIsAbsentAndTheSameKeyFoundAfter() throws {
    defer { try? item.remove() }
    guard case .ready(let made) = IdentityKey.find(in: item) else {
      Issue.record("no key was made")
      return
    }
    guard case .ready(let found) = IdentityKey.find(in: item) else {
      Issue.record("the key was not found again")
      return
    }
    #expect(found.publicKey() == made.publicKey())
  }

  @Test func theItemIsKeptOnThisPhoneAloneAfterTheFirstUnlockAndNeverSynced() throws {
    defer { try? item.remove() }
    guard case .ready = IdentityKey.find(in: item) else {
      Issue.record("no key was made")
      return
    }
    let query: [String: Any] = [
      kSecClass as String: kSecClassGenericPassword,
      kSecAttrService as String: KeyItem.service,
      kSecAttrAccount as String: item.account,
      kSecReturnAttributes as String: true,
    ]
    var found: CFTypeRef?
    #expect(SecItemCopyMatching(query as CFDictionary, &found) == errSecSuccess)
    let attributes = try #require(found as? [String: Any])
    #expect(
      attributes[kSecAttrAccessible as String] as? String == kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly as String)
    #expect((attributes[kSecAttrSynchronizable as String] as? Bool ?? false) == false)
  }

  @Test func aBlobTheChipRefusesIsKeptUntilTheUserStartsAgain() throws {
    defer { try? item.remove() }
    let spoiled = Data("not a key".utf8)
    try item.add(spoiled)
    guard case .refused = IdentityKey.find(in: item) else {
      Issue.record("a spoiled blob was taken")
      return
    }
    #expect(try item.read() == spoiled)
    guard case .ready(let started) = IdentityKey.startAgain(in: item) else {
      Issue.record("no key was made starting again")
      return
    }
    guard case .ready(let found) = IdentityKey.find(in: item) else {
      Issue.record("the new key was not kept")
      return
    }
    #expect(found.publicKey() == started.publicKey())
  }
}
