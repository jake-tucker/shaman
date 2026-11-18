import CoreText
import CryptoKit
import Foundation
import Security
import ShamanCore
import SwiftUI
import Synchronization
import Testing
import UIKit

@testable import ShamanUI

/// A subscription that only counts its cancellations.
private final class CountedSubscription: SubscriptionProtocol {
  let cancelled = Atomic<Int>(0)

  func cancel() {
    cancelled.add(1, ordering: .relaxed)
  }
}

/// A directory of the test's own, gone afterwards.
private func scratch() throws -> URL {
  let directory = FileManager.default.temporaryDirectory.appending(path: UUID().uuidString, directoryHint: .isDirectory)
  try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
  return directory
}

@Suite struct ListenedTests {
  @Test func whatTheListenerHearsIsWhatTheStreamYields() async {
    let subscription = CountedSubscription()
    let stream: AsyncStream<Int> = listened { send in
      send(1)
      return subscription
    }
    var heard = stream.makeAsyncIterator()
    #expect(await heard.next() == 1)
  }

  @Test func onlyTheNewestUnreadValueIsKept() async {
    let subscription = CountedSubscription()
    let stream: AsyncStream<Int> = listened { send in
      for value in 1...3 { send(value) }
      return subscription
    }
    var heard = stream.makeAsyncIterator()
    #expect(await heard.next() == 3)
  }

  @Test func theStreamEndingCancelsTheSubscription() async {
    let subscription = CountedSubscription()
    let following = Task {
      let stream: AsyncStream<Int> = listened { _ in subscription }
      for await _ in stream {}
    }
    following.cancel()
    await following.value
    #expect(subscription.cancelled.load(ordering: .relaxed) == 1)
  }
}

@Suite struct CoreDirectoryTests {
  @Test func theDirectoryIsMadeAndKeptOutOfBackups() throws {
    let base = try scratch()
    defer { try? FileManager.default.removeItem(at: base) }
    let directory = try CoreDirectory.prepare(named: "Core", in: base)
    let values = try directory.resourceValues(forKeys: [.isDirectoryKey, .isExcludedFromBackupKey])
    #expect(values.isDirectory == true)
    #expect(values.isExcludedFromBackup == true)
    #expect(try CoreDirectory.prepare(named: "Core", in: base) == directory, "made once, found after")
  }

  @Test func aRemovedDirectoryIsGoneWithWhatItHeldAndRemovingNoneIsFine() throws {
    let base = try scratch()
    defer { try? FileManager.default.removeItem(at: base) }
    let directory = try CoreDirectory.prepare(named: "Scratch", in: base)
    try Data("{}".utf8).write(to: directory.appending(path: "thin-client.json"))
    try CoreDirectory.remove(named: "Scratch", in: base)
    #expect(!FileManager.default.fileExists(atPath: directory.path(percentEncoded: false)))
    try CoreDirectory.remove(named: "Scratch", in: base)
  }
}

/// A key made in the simulator's Secure Enclave, as the app makes one. The package's tests run without the Keychain,
/// which an unhosted test bundle has no entitlement to reach; `ShamanTests`, hosted in the app, covers the item itself.
private func madeKey() throws -> EnclaveSigner {
  try EnclaveSigner.made()
}

/// A Keychain held in memory, which may be told not to say whether it holds the item, or not to keep one.
private final class MemoryStore: KeyStore {
  let reads: OSStatus?
  let adds: OSStatus?
  let blob = Mutex<Data?>(nil)

  init(holding blob: Data? = nil, reads: OSStatus? = nil, adds: OSStatus? = nil) {
    self.blob.withLock { $0 = blob }
    self.reads = reads
    self.adds = adds
  }

  func read() throws(KeychainFailure) -> Data? {
    if let reads { throw KeychainFailure(status: reads) }
    return blob.withLock { $0 }
  }

  func add(_ blob: Data) throws(KeychainFailure) {
    if let adds { throw KeychainFailure(status: adds) }
    try self.blob.withLock { (held: inout Data?) throws(KeychainFailure) in
      guard held == nil else { throw KeychainFailure(status: errSecDuplicateItem) }
      held = blob
    }
  }

  func remove() throws(KeychainFailure) {
    blob.withLock { $0 = nil }
  }
}

@Suite struct EnclaveSignerTests {
  @Test func itSignsWithTheKeyOverSHA256AsDERAndNamesTheKeyAsDER() throws {
    let signer = try madeKey()
    let key = try P256.Signing.PublicKey(derRepresentation: signer.publicKey())
    let message = Data("a message".utf8)
    let signature = try P256.Signing.ECDSASignature(derRepresentation: try signer.sign(message: message))
    #expect(key.isValidSignature(signature, for: message), "the key's, over SHA-256, in ASN.1 DER")
    #expect(!key.isValidSignature(signature, for: Data("another".utf8)))
  }

  /// The access control a key is made under, compared whole with others by the Security framework's own equality.
  @Test func theKeyIsMadeForSigningAloneAfterTheFirstUnlockOnThisPhone() throws {
    func control(_ accessibility: CFString, _ flags: SecAccessControlCreateFlags) throws -> SecAccessControl {
      try #require(SecAccessControlCreateWithFlags(nil, accessibility, flags, nil))
    }
    let madeUnder = try KeyPolicy.accessControl()
    let afterFirstUnlock = kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly
    #expect(CFEqual(madeUnder, try control(afterFirstUnlock, [.privateKeyUsage])))
    for other in [
      try control(afterFirstUnlock, [.privateKeyUsage, .userPresence]),
      try control(afterFirstUnlock, [.privateKeyUsage, .biometryAny]),
      try control(afterFirstUnlock, [.privateKeyUsage, .devicePasscode]),
      try control(kSecAttrAccessibleWhenUnlockedThisDeviceOnly, [.privateKeyUsage]),
      try control(kSecAttrAccessibleAfterFirstUnlock, [.privateKeyUsage]),
    ] {
      #expect(!CFEqual(madeUnder, other), "\(other)")
    }
  }

  @Test func aKeyIsMadeAgainFromItsBlob() throws {
    let made = try madeKey()
    let again = EnclaveSigner(try SecureEnclave.P256.Signing.PrivateKey(dataRepresentation: made.blob))
    #expect(again.publicKey() == made.publicKey())
  }

  @Test func theCoreSignsThroughItAndShowsTheFingerprintAServerWould() async throws {
    let base = try scratch()
    defer { try? FileManager.default.removeItem(at: base) }
    let core = try LiveCore(directory: try CoreDirectory.prepare(in: base), signer: try madeKey())
    var heard = core.thisDevice().makeAsyncIterator()
    let fingerprint = try #require(await heard.next()?.fingerprint)
    #expect(fingerprint.count == 64)
    #expect(fingerprint.allSatisfy { $0.isHexDigit })
  }
}

/// A key that will not sign, as a chip may refuse.
private final class RefusingSigner: KeySigner {
  private let key: Data

  init() throws {
    key = try madeKey().publicKey()
  }

  func publicKey() -> Data { key }

  func sign(message: Data) throws(SignError) -> Data {
    throw .Refused(message: "the chip will not take the key")
  }
}

@Suite struct OpeningTests {
  @Test func aKeyThatWillNotSignOffersToStartAgainInTheCoresWords() throws {
    let base = try scratch()
    defer { try? FileManager.default.removeItem(at: base) }
    let opening = Opening.over(try RefusingSigner(), in: try CoreDirectory.prepare(in: base))
    guard case .keyRefused(let message) = opening else {
      Issue.record("a key that will not sign was not refused: \(opening)")
      return
    }
    #expect(message == "This phone's identity key would not sign: the chip will not take the key")
  }

  @Test func aKeyTheChipRefusesOffersToStartAgain() throws {
    let base = try scratch()
    defer { try? FileManager.default.removeItem(at: base) }
    let opening = Opening.of(.refused("refused"), in: try CoreDirectory.prepare(in: base))
    guard case .keyRefused("refused") = opening else {
      Issue.record("\(opening)")
      return
    }
  }

  @Test func aKeyThatSignsOpensTheCore() throws {
    let base = try scratch()
    defer { try? FileManager.default.removeItem(at: base) }
    guard case .core = Opening.of(.ready(try madeKey()), in: try CoreDirectory.prepare(in: base)) else {
      Issue.record("the core did not open")
      return
    }
  }
}

@Suite struct IdentityKeyTests {
  @Test func aKeyIsMadeWhereTheItemIsAbsentAndReusedAfter() throws {
    let store = MemoryStore()
    guard case .ready(let made) = IdentityKey.find(in: store) else {
      Issue.record("no key was made")
      return
    }
    #expect(try store.read() == made.blob, "its blob is kept")
    guard case .ready(let found) = IdentityKey.find(in: store) else {
      Issue.record("the key was not found again")
      return
    }
    #expect(found.publicKey() == made.publicKey(), "the one key, reused")
  }

  @Test func aBlobTheChipRefusesIsSaidSoAndKeptUntilTheUserStartsAgain() throws {
    let spoiled = Data("not a key".utf8)
    let store = MemoryStore(holding: spoiled)
    guard case .refused(let message) = IdentityKey.find(in: store) else {
      Issue.record("a spoiled blob was taken")
      return
    }
    #expect(message.hasPrefix("The Secure Enclave will not take this phone's identity key"), "\(message)")
    #expect(try store.read() == spoiled, "nothing is made on the app's own motion")

    guard case .ready(let started) = IdentityKey.startAgain(in: store) else {
      Issue.record("no key was made starting again")
      return
    }
    guard case .ready(let found) = IdentityKey.find(in: store) else {
      Issue.record("the new key was not kept")
      return
    }
    #expect(found.publicKey() == started.publicKey())
  }

  @Test func aKeychainThatDoesNotSayTheItemIsAbsentCostsNoKey() {
    let store = MemoryStore(reads: errSecInteractionNotAllowed)
    guard case .failed(let message) = IdentityKey.find(in: store) else {
      Issue.record("a key was made, or one found, where the Keychain said neither")
      return
    }
    #expect(message.hasPrefix("This phone's identity key could not be read from the Keychain"), "\(message)")
    #expect(store.blob.withLock { $0 } == nil, "nothing is made")
  }

  @Test func aKeyTheKeychainWillNotKeepIsSaidSo() {
    let store = MemoryStore(adds: errSecNotAvailable)
    guard case .failed(let message) = IdentityKey.find(in: store) else {
      Issue.record("a key the Keychain did not keep was used")
      return
    }
    #expect(message.hasPrefix("This phone's new identity key could not be kept"), "\(message)")
  }
}

@Suite struct LiveCoreTests {
  @Test func aNewPhoneIsFirstOfferedAsIPhoneAndARefusalIsInTheCoresWords() async throws {
    let base = try scratch()
    defer { try? FileManager.default.removeItem(at: base) }
    let core = try LiveCore(directory: try CoreDirectory.prepare(in: base), signer: try madeKey())
    var heard = core.thisDevice().makeAsyncIterator()
    #expect(await heard.next()?.name == "iPhone")

    await #expect(throws: ActError.Invalid(message: "A name needs something other than spaces")) {
      try await core.renameDevice("   ")
    }
    try await core.renameDevice("Pocket")
    #expect(await heard.next()?.name == "Pocket")
  }

  @Test func recordsOfALostKeyAreDiscardedUnderANewOneAndTheNameStays() async throws {
    let base = try scratch()
    defer { try? FileManager.default.removeItem(at: base) }
    let directory = try CoreDirectory.prepare(in: base)
    let lost = try LiveCore(directory: directory, signer: try madeKey())
    var heard = lost.thisDevice().makeAsyncIterator()
    let before = try #require(await heard.next())
    try await lost.renameDevice("Pocket")
    _ = await heard.next()
    try Data(#"[{"name":"laptop"}]"#.utf8).write(to: directory.appending(path: "remotes.json"))

    let core = try LiveCore(directory: directory, signer: try madeKey())
    var device = core.thisDevice().makeAsyncIterator()
    let after = try #require(await device.next())
    #expect(after.name == "Pocket")
    #expect(after.fingerprint != before.fingerprint)
    #expect(after.pairAgain != nil)
    #expect(
      !FileManager.default.fileExists(atPath: directory.appending(path: "remotes.json").path(percentEncoded: false)))
    try await core.dismissPairAgain()
    #expect(await device.next()?.pairAgain == nil)
  }
}

@Suite struct RunningTimeTests {
  private let now = Date(timeIntervalSince1970: 1_000_000)

  @Test func aDurationReadsSecondsThenMinutesThenHoursAndTheMinutesPastThem() {
    #expect(RunningTime.duration(since: now.addingTimeInterval(-42), now: now) == "42s")
    #expect(RunningTime.duration(since: now.addingTimeInterval(-252), now: now) == "4m")
    #expect(RunningTime.duration(since: now.addingTimeInterval(-(2 * 3_600 + 5 * 60)), now: now) == "2h 5m")
  }

  @Test func theCompactTimeReadsNowThenMinutesHoursOrDays() {
    #expect(RunningTime.compact(since: now.addingTimeInterval(-59), now: now) == "now")
    #expect(RunningTime.compact(since: now.addingTimeInterval(-60), now: now) == "1m")
    #expect(RunningTime.compact(since: now.addingTimeInterval(-3 * 3_600), now: now) == "3h")
    #expect(RunningTime.compact(since: now.addingTimeInterval(-9 * 86_400), now: now) == "9d")
  }

  @Test func aStartInTheFutureReadsAsNoTimeAtAll() {
    let later = now.addingTimeInterval(90)
    #expect(RunningTime.duration(since: later, now: now) == "0s")
    #expect(RunningTime.compact(since: later, now: now) == "now")
  }
}

@Suite struct IconFontTests {
  @Test @MainActor func drawingAGlyphRegistersTheBundledFontWhichDrawsTheCoresGlyphs() throws {
    let renderer = ImageRenderer(content: Glyph(glyph: "\u{f06a9}"))
    #expect(renderer.uiImage != nil, "a glyph is drawn")
    let name = try #require(IconFont.name, "the bundled font registered as the glyph was drawn")
    // CoreText answers registering it again with its being registered already.
    let file = try #require(IconFont.file)
    var error: Unmanaged<CFError>?
    #expect(!CTFontManagerRegisterFontsForURL(file as CFURL, .process, &error))
    #expect(
      error.map { CFErrorGetCode($0.takeRetainedValue()) } == CTFontManagerError.alreadyRegistered.rawValue,
      "registered with CoreText for this process")
    let font = try #require(UIFont(name: name, size: 17))
    // The Sidekick Icon, the plain folder, and a Checkout State's branch.
    for glyph in ["\u{f06a9}", "\u{ea83}", "\u{ec6f}"] {
      let units = Array(glyph.utf16)
      var glyphs = [CGGlyph](repeating: 0, count: units.count)
      #expect(CTFontGetGlyphsForCharacters(font as CTFont, units, &glyphs, units.count), "draws \(glyph)")
    }
  }

  @Test func theFontIsRegisteredOnceHoweverOftenItIsAskedFor() {
    #expect(IconFont.name == IconFont.name)
    #expect(IconFont.name != nil)
  }
}
