import CoreImage
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

  @Test func aTurnsWorkAndAnActivitysDetailCrossFromTheCoreThroughTheirStreams() async throws {
    let base = try scratch()
    defer { try? FileManager.default.removeItem(at: base) }
    let core = try LiveCore(directory: try CoreDirectory.prepare(in: base), signer: try madeKey())
    // A Session on a Remote the phone does not hold is never read, so what is attached to it says it is gone at once.
    let session = core.openSession("nobody/\(UUID().uuidString.lowercased())")
    defer { session.close() }
    var work = session.work(UUID().uuidString.lowercased()).makeAsyncIterator()
    #expect(await work.next()?.empty == "This turn is no longer in the session")
    var detail = session.detail(UUID().uuidString.lowercased()).makeAsyncIterator()
    #expect(await detail.next()?.sections.first?.body == "This is no longer in the session")
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

  @Test func theBarsTimeReadsSecondsThenPaddedMinutesThenHours() {
    #expect(RunningTime.working(since: now.addingTimeInterval(-42), now: now) == "42s")
    #expect(RunningTime.working(since: now.addingTimeInterval(-74), now: now) == "1m 14s")
    #expect(RunningTime.working(since: now.addingTimeInterval(-(3_600 + 2 * 60 + 3)), now: now) == "1h 02m 03s")
  }

  @Test func aStartInTheFutureReadsAsNoTimeAtAll() {
    let later = now.addingTimeInterval(90)
    #expect(RunningTime.working(since: later, now: now) == "0s")
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

@Suite @MainActor struct NoticeModelTests {
  /// Waits until the model's notice stands as `shown` looks for.
  private func notice(of model: NoticeModel, until shown: (Notice?) -> Bool) async throws -> Notice? {
    for _ in 0..<200 {
      if shown(model.notice) { return model.notice }
      try await Task.sleep(for: .milliseconds(10))
    }
    Issue.record("the notice never stood as looked for")
    return model.notice
  }

  @Test func aNoticeStandsUntilTappedAwayAndIsNeverSaidAgain() async throws {
    let core = FakeCore(.releases)
    let model = NoticeModel(core: core, stands: .seconds(3600))
    let following = Task { await model.follow() }
    defer { following.cancel() }
    let said = try await notice(of: model) { $0 != nil }
    #expect(said?.words == "workstation runs a newer Shaman; some things may not show")
    model.dismiss()
    #expect(model.notice == nil)
    // Followed afresh, the core says it no more.
    let again = NoticeModel(core: core, stands: .seconds(3600))
    let refollowing = Task { await again.follow() }
    defer { refollowing.cancel() }
    try await Task.sleep(for: .milliseconds(100))
    #expect(again.notice == nil)
  }

  @Test func aNoticeGoesByItselfOnceItHasStoodAWhileOnScreen() async throws {
    let model = NoticeModel(core: FakeCore(.releases), stands: .milliseconds(50))
    let following = Task { await model.follow() }
    defer { following.cancel() }
    _ = try await notice(of: model) { $0 != nil }
    model.appeared()
    _ = try await notice(of: model) { $0 == nil }
  }

  @Test func aNoticeNoBannerShowsIsNeverSeenForItAndItsWhileRunsOnlyOnScreen() async throws {
    let core = FakeCore(.releases)
    let model = NoticeModel(core: core, stands: .milliseconds(300))
    let following = Task { await model.follow() }
    defer { following.cancel() }
    _ = try await notice(of: model) { $0 != nil }
    // Beneath a sheet, no banner shows it, however long it waits.
    try await Task.sleep(for: .milliseconds(500))
    #expect(model.notice != nil)
    // Shown a while, then covered: the while it stood is kept, and runs on only once it shows again.
    model.appeared()
    try await Task.sleep(for: .milliseconds(150))
    model.disappeared()
    try await Task.sleep(for: .milliseconds(400))
    #expect(model.notice != nil, "not seen while covered")
    model.appeared()
    _ = try await notice(of: model) { $0 == nil }
    var heard = core.notices().makeAsyncIterator()
    #expect(await heard.next() == .some(nil), "and seen once it had stood its while on screen")
  }

  @Test func theLiveCoreSaysNothingOfItsOwnAccordAsItOpens() async throws {
    let base = try scratch()
    defer { try? FileManager.default.removeItem(at: base) }
    let core = try LiveCore(directory: try CoreDirectory.prepare(in: base), signer: try madeKey())
    var heard = core.notices().makeAsyncIterator()
    let first = await heard.next()
    #expect(first == .some(nil))
  }
}

@Suite @MainActor struct ListingModelTests {
  @Test func anActsFailureStandsInTheBannerUntilTappedAway() async {
    let model = ListingModel(core: FakeCore(.paired), failureStands: .seconds(3600))
    await model.settle("a session no longer listed")
    #expect(model.failure == "That session is no longer listed")
    model.dismissFailure()
    #expect(model.failure == nil)
  }

  @Test func anActsFailureGoesByItselfOnceItHasStoodAWhile() async throws {
    let model = ListingModel(core: FakeCore(.paired), failureStands: .milliseconds(50))
    await model.settle("a session no longer listed")
    #expect(model.failure != nil)
    try await Task.sleep(for: .milliseconds(400))
    #expect(model.failure == nil)
  }

  @Test func anActThatSucceedsClearsAFailureStanding() async {
    let model = ListingModel(core: FakeCore(.paired), failureStands: .seconds(3600))
    await model.settle("a session no longer listed")
    await model.settle("workstation-key/1")
    #expect(model.failure == nil)
  }

  /// Waits until the model's listing shows what `shown` looks for.
  private func listing(of model: ListingModel, until shown: (Listing) -> Bool) async throws -> Listing {
    for _ in 0..<200 {
      if let listing = model.listing, shown(listing) { return listing }
      try await Task.sleep(for: .milliseconds(10))
    }
    Issue.record("the listing never showed what was looked for")
    return try #require(model.listing)
  }

  @Test func anUnreachableRemotesRowIsTheOneTheListingHoldsNowAndGoesWithIt() async throws {
    let core = FakeCore(.unreachable)
    let model = ListingModel(core: core, failureStands: .seconds(3600))
    let following = Task { await model.follow() }
    defer { following.cancel() }
    let listing = try await listing(of: model) { !$0.unreachable.isEmpty }
    let remote = try #require(listing.unreachable.first).remote
    #expect(model.unreachable(remote)?.words == "homelab unreachable")

    try await core.tryAgain(remote)
    _ = try await self.listing(of: model) { $0.unreachable.isEmpty }
    #expect(model.unreachable(remote) == nil, "the sheet it opened goes with it")
  }

  @Test func aSearchIsHandedToTheCoreWhichAnswersWithWhatItFound() async throws {
    let model = ListingModel(core: FakeCore(.paired), failureStands: .seconds(3600))
    let following = Task { await model.follow() }
    defer { following.cancel() }
    model.search("zsh")
    let found = try await listing(of: model) { $0.found != nil }.found
    #expect(found?.map(\.session) == ["workstation-key/7"])
    model.search("")
    _ = try await listing(of: model) { $0.found == nil }
  }

  @Test func aScopeChosenIsHandedToTheCoreWhichNarrowsTheListing() async throws {
    let model = ListingModel(core: FakeCore(.several), failureStands: .seconds(3600))
    let following = Task { await model.follow() }
    defer { following.cancel() }
    let menu = try await listing(of: model) { _ in true }
    let laptop = try #require(menu.scopes.flatMap(\.choices).first { $0.words == "All of laptop" })
    await model.narrow(laptop.scope)
    let narrowed = try await listing(of: model) { $0.title == "laptop" }
    #expect(narrowed.active.allSatisfy { $0.place.hasPrefix("laptop · ") })
    #expect(model.failure == nil)
  }
}

@Suite struct ConversationTests {
  private func item(_ id: String, _ text: String) -> ConversationItem {
    ConversationItem(id: id, kind: .error(text: text))
  }

  @Test func theWholeIsTakenAndThenEachChangeReplacesItsItemsByIdInTheOrderSaid() {
    var conversation = Conversation()
    conversation.take(.whole(items: [item("loading", "…")]))
    #expect(conversation.shown == [item("loading", "…")])

    conversation.take(.changed(items: [item("a", "one"), item("b", "two")], order: ["a", "b"]))
    #expect(conversation.shown == [item("a", "one"), item("b", "two")], "the loading mark gave way to the whole")

    conversation.take(.changed(items: [item("b", "two, longer")], order: ["a", "b"]))
    #expect(conversation.shown == [item("a", "one"), item("b", "two, longer")], "only the item that changed moved")

    conversation.take(.changed(items: [], order: ["b"]))
    #expect(conversation.shown == [item("b", "two, longer")], "an item left out of the order is gone")
    conversation.take(.changed(items: [], order: ["b", "a"]))
    #expect(conversation.shown == [item("b", "two, longer")], "and stays gone")
  }

  @Test func theLoadingMarkAndWhyItCannotBeReadStandOverTheConversationNotInIt() {
    var conversation = Conversation()
    let loading = ConversationItem(id: "loading", kind: .loading(words: "Loading"))
    conversation.take(.whole(items: [loading]))
    #expect(conversation.rows.isEmpty)
    #expect(conversation.marks == [loading])
    #expect(!conversation.read)

    conversation.take(.whole(items: [item("a", "one"), item("b", "two")]))
    #expect(conversation.rows == [item("a", "one"), item("b", "two")])
    #expect(conversation.marks.isEmpty)
    #expect(conversation.read)
  }
}

@Suite struct ConversationGeometryTests {
  private let spacing = ConversationGeometry.spacing
  private let margin = ConversationGeometry.margin
  private let guess = ConversationGeometry.guess

  private func geometry(_ heights: [CGFloat]) -> ConversationGeometry {
    var geometry = ConversationGeometry()
    geometry.take(heights.indices.map { "row\($0)" })
    for (index, height) in heights.enumerated() {
      geometry.measured("row\(index)", at: height, foot: 0)
    }
    return geometry
  }

  @Test func rowsStandOneBelowAnotherAsTheyMeasuredAndAsGuessedWhereTheyHaveNot() {
    var geometry = geometry([40, 300])
    geometry.take(["row0", "row1", "row2"])
    #expect(geometry.frame(of: 0, width: 100) == CGRect(x: 0, y: margin, width: 100, height: 40))
    #expect(geometry.frame(of: 1, width: 100).minY == margin + 40 + spacing)
    #expect(geometry.frame(of: 2, width: 100).height == guess, "not yet drawn, so guessed")
    #expect(geometry.height == margin + 40 + spacing + 300 + spacing + guess + margin)
    #expect(geometry.measured("row1") && !geometry.measured("row2"))
  }

  @Test func theRowsBetweenTwoDepthsAreThoseStandingAtLeastPartlyBetweenThem() {
    let geometry = geometry([100, 100, 100, 100])
    // Rows stand at 8, 120, 232 and 344, each 100 tall.
    #expect(geometry.rows(between: 0, and: 50) == 0..<1)
    #expect(geometry.rows(between: 150, and: 250) == 1..<3)
    #expect(geometry.rows(between: 1000, and: 2000).isEmpty)
  }

  @Test func aRowEndingAboveWhatTheReaderSeesMovesItByAsMuchAsItChanged() {
    var geometry = geometry([100, 100, 100])
    // The reader sees from 300 down to 600: the first row ends at 108, above it.
    #expect(geometry.measured("row0", at: 350, foot: 600) == 250)
    #expect(geometry.frame(of: 2, width: 1).minY == margin + 350 + spacing + 100 + spacing)
    #expect(geometry.measured("row0", at: 50, foot: 600) == -300, "shrinking moves it back as far")
  }

  @Test func aRowFirstDrawnAtTheTopOfWhatTheReaderSeesKeepsWhatStandsBelowItStill() {
    var geometry = ConversationGeometry()
    geometry.take(["new", "seen"])
    geometry.measured("seen", at: 400, foot: 0)
    // Scrolling up, the row not yet drawn comes into sight at the top, guessed, its foot at 128.
    #expect(geometry.measured("new", at: 900, foot: 500) == 900 - guess)
  }

  @Test func aRowRunningOnPastWhatTheReaderSeesGrowsBeneathThemAndMovesNothing() {
    var geometry = geometry([100, 2000])
    // The reader is within the long last row, which streams on below the foot of what they see.
    #expect(geometry.measured("row1", at: 2400, foot: 1200) == 0)
    #expect(geometry.height == margin + 100 + spacing + 2400 + margin)
  }

  @Test func aRowBelowWhatTheReaderSeesMovesNothing() {
    var geometry = geometry([100, 100])
    #expect(geometry.measured("row1", at: 500, foot: 150) == 0)
  }

  @Test func whatWasMeasuredIsKeptForTheRowsStillHeldAndForgottenForTheRest() {
    var geometry = geometry([100, 200])
    geometry.take(["row1", "row2"])
    #expect(geometry.frame(of: 0, width: 1).height == 200)
    #expect(geometry.frame(of: 1, width: 1).height == guess)
    geometry.take(["row0", "row1"])
    #expect(!geometry.measured("row0"), "it left, so what it measured went with it")
    geometry.forget()
    #expect(!geometry.measured("row1"))
  }
}

@Suite struct ListenedInOrderTests {
  @Test func everyChangeIsKeptInOrder() async {
    let subscription = CountedSubscription()
    let stream: AsyncStream<Int> = listened(everyChange: true) { send in
      for value in 1...3 { send(value) }
      return subscription
    }
    var heard: [Int] = []
    for await value in stream {
      heard.append(value)
      if heard.count == 3 { break }
    }
    #expect(heard == [1, 2, 3])
  }
}

@Suite @MainActor struct SessionStackTests {
  @Test func aSessionIsClosedInTheCoreOnceItsScreenLeavesThePathAndNotBefore() {
    let core = FakeCore(.paired)
    let working = SessionRoute(session: "workstation-key/1")
    let settled = SessionRoute(session: "workstation-key/7")
    let stack = SessionStack(core: core)
    stack.path = [.session(working)]
    let model = stack.model(for: working)
    #expect(stack.model(for: working) === model, "opened once for its screen")
    stack.path.append(.session(settled))
    _ = stack.model(for: settled)
    let sessions = core.opened.withLock { $0 }
    #expect(sessions.count == 2)
    #expect(!sessions[0].isClosed && !sessions[1].isClosed, "both still pushed")

    stack.path.removeLast()
    #expect(!sessions[0].isClosed, "still pushed")
    #expect(sessions[1].isClosed, "popped, so closed")
    stack.path = []
    #expect(sessions[0].isClosed, "popped to the listing, so closed")
  }

  @Test func aSessionStaysOpenWhileItsTurnsWorkOrAnActivityIsPushedOverIt() {
    let core = FakeCore(.paired)
    let settled = SessionRoute(session: "workstation-key/7")
    let stack = SessionStack(core: core)
    stack.path = [.session(settled)]
    let model = stack.model(for: settled)
    stack.path.append(.work(settled, turn: "work"))
    stack.path.append(.detail(settled, activity: "check"))
    #expect(stack.model(for: settled) === model, "the work and the detail read through the Session's own")
    let session = core.opened.withLock { $0[0] }
    stack.path.removeLast(2)
    #expect(!session.isClosed, "its own screen is still pushed")
    stack.path = []
    #expect(session.isClosed)
  }

  @Test func theSameSessionPushedTwiceIsTwoScreens() {
    #expect(SessionRoute(session: "a") != SessionRoute(session: "a"))
  }

  @Test func anEntryChosenInTheTreeIsPushedPoppedBackToOrLeftAsTheOneOpen() {
    let core = FakeCore(.paired)
    let parent = SessionRoute(session: "workstation-key/12")
    let stack = SessionStack(core: core)
    stack.path = [.session(parent), .work(parent, turn: "work")]
    let explorer = SessionRoute(session: "workstation-key/12.1")
    stack.path.append(.session(explorer))

    // Not on the stack, it is pushed over whatever is open.
    stack.show("workstation-key/12.4", over: explorer)
    #expect(stack.path.count == 4)
    guard case .session(let grepper) = stack.path.last else {
      Issue.record("a Session pushed")
      return
    }
    #expect(grepper.session == "workstation-key/12.4")

    // The one open: nothing more.
    stack.show("workstation-key/12.4", over: grepper)
    #expect(stack.path.count == 4)

    // On the stack, it is popped back to, with what was pushed over it.
    stack.show("workstation-key/12.1", over: grepper)
    #expect(stack.path == [.session(parent), .work(parent, turn: "work"), .session(explorer)])
    stack.show("workstation-key/12", over: explorer)
    #expect(stack.path == [.session(parent)])
  }
}

@Suite @MainActor struct SidekickTreeTests {
  @Test func theSidekickChosenFromASessionOnAnotherServerOpenedOverItIsPoppedBackTo() {
    let stack = SessionStack(core: FakeCore(.sidekick))
    let sidekick = SessionRoute(session: "workstation-key/20")
    let parser = SessionRoute(session: "laptop-key/22")
    stack.path = [.session(sidekick), .session(parser)]
    // Each Session is known by its own Server's key, so one of the laptop's names no Session of the workstation's.
    stack.show("workstation-key/22", over: parser)
    #expect(stack.path.count == 3, "a Session of the same identity on another Server is another Session")
    stack.show("workstation-key/20", over: parser)
    #expect(stack.path == [.session(sidekick)])
  }

  @Test func anEntryOnAnotherServerChosenInTheTreeIsPushedAsItsOwnSession() {
    let stack = SessionStack(core: FakeCore(.sidekick))
    let sidekick = SessionRoute(session: "workstation-key/20")
    stack.path = [.session(sidekick)]
    stack.show("laptop-key/22", over: sidekick)
    guard case .session(let pushed) = stack.path.last else {
      Issue.record("a Session pushed")
      return
    }
    #expect(pushed.session == "laptop-key/22")
    #expect(stack.path.count == 2)
  }
}

@Suite @MainActor struct ReviewTests {
  @Test func aBarsReviewPushesTheOneSubagentWaitingOpensTheTreeOrPresentsTheSessionsOwn() {
    let core = FakeCore(.subagentsWait)
    let parent = SessionRoute(session: "workstation-key/12")
    let stack = SessionStack(core: core)
    stack.path = [.session(parent)]

    // One Subagent waiting: its Session is pushed over the one whose bar it was.
    #expect(!stack.review(.session(session: "workstation-key/12.4"), over: parent), "no tree")
    guard case .session(let grepper) = stack.path.last else {
      Issue.record("a Session pushed")
      return
    }
    #expect(grepper.session == "workstation-key/12.4")
    #expect(stack.path.count == 2)

    // Several: the tree is opened, and nothing pushed.
    #expect(stack.review(.tree, over: grepper))
    #expect(stack.path.count == 2)

    // Its own, presented again by the core where it stands.
    #expect(!stack.review(.interventions, over: grepper))
    #expect(stack.path == [.session(parent), .session(grepper)])
  }
}

@Suite @MainActor struct SessionModelTests {
  private let working = "workstation-key/1"

  @Test func aStopTheServerRefusesStandsInTheBannerInItsWordsAndStopsNothing() async {
    let core = FakeCore(.paired)
    let model = SessionModel(core: core, session: "workstation-key/12")
    // A settled Subagent: its Server has nothing to stop.
    await model.stopSubagent("workstation-key/12.2")
    #expect(model.banner == "That subagent is not working")
    #expect(!model.popped)
    var tree = model.tree().makeAsyncIterator()
    #expect(await tree.next()?.loading == "Loading", "the tree is read a while after its first session opens")
  }

  @Test func theComposerOpensOnItsDraftAndHandsTheCoreEachChange() {
    let core = FakeCore(.paired)
    core.setDraft("half a thought", version: 0, for: working)
    let model = SessionModel(core: core, session: working)
    #expect(model.text == "half a thought")
    model.text = "half a thought, finished"
    #expect(core.draft(for: working).text == "half a thought, finished")
  }

  @Test func aSendEmptiesTheFieldAtOnce() async {
    let core = FakeCore(.paired)
    let model = SessionModel(core: core, session: working)
    model.text = "Give it a quiet zone"
    let sending = Task { await model.send() }
    await Task.yield()
    #expect(model.text == "")
    #expect(core.draft(for: working).text == "")
    await sending.value
    #expect(model.banner == nil)
    #expect(model.text == "")
  }

  @Test func eachSendOrQueueIsCountedSoThatTheConversationGoesBackToItsFoot() async {
    let model = SessionModel(core: FakeCore(.paired), session: working, failureStands: .seconds(3600))
    #expect(model.sent == 0)
    model.text = "   "
    await model.send()
    #expect(model.sent == 0, "nothing to send, so nothing sent")
    model.text = "Give it a quiet zone"
    await model.send()
    #expect(model.sent == 1)
    model.text = "And after that, the docs"
    await model.queue()
    #expect(model.sent == 2)
  }

  @Test func aRefusedSendStandsInTheBannerAndItsTextIsBackInTheField() async {
    let core = FakeCore(.paired)
    let model = SessionModel(core: core, session: working, failureStands: .seconds(3600))
    model.text = "Please fail"
    await model.send()
    #expect(model.banner == "The server refused the prompt: its agent is not signed in")
    #expect(model.text == "Please fail")
    #expect(!model.popped)
    model.dismissBanner()
    #expect(model.banner == nil)
  }

  @Test func aSessionGonePopsTheScreenAndShowsNoBanner() async {
    let model = SessionModel(core: FakeCore(.paired), session: working)
    model.text = "Is it gone?"
    await model.send()
    #expect(model.popped)
    #expect(model.banner == nil)
  }

  @Test func aQueuedPromptNoLongerThereSaysSoAndLeavesTheScreenUp() async {
    let model = SessionModel(core: FakeCore(.paired), session: working, failureStands: .seconds(3600))
    await model.steerNow("no such prompt")
    #expect(model.banner == "That prompt is no longer queued")
    #expect(!model.popped)
    #expect(model.acting.isEmpty)
  }

  @Test func onAnUnreachableOriginASendFailsInTheBannerAndKeepsItsText() async {
    let model = SessionModel(core: FakeCore(.unreachable), session: "homelab-key/2", failureStands: .seconds(3600))
    model.text = "Upgrade it"
    await model.send()
    #expect(model.banner == "The server did not answer in time")
    #expect(model.invalid == nil)
  }

  @Test func aSendNeverAnsweredKeepsItsTextAndTheLineSaysItMayHaveBeenSent() async throws {
    let model = SessionModel(core: FakeCore(.paired), session: working, failureStands: .seconds(3600))
    let following = Task { await model.follow() }
    defer { following.cancel() }
    model.text = "Unanswered, this one"
    await model.send()
    #expect(model.banner == "The server did not answer")
    #expect(model.text == "Unanswered, this one")
    for _ in 0..<100 where model.frame?.composer?.note == nil {
      try await Task.sleep(for: .milliseconds(10))
    }
    #expect(model.frame?.composer?.note == "The server did not answer, so this prompt may have been sent")
  }

  @Test func textGivenBackWinsOverWhatIsTypedBeforeTheFieldHearsOfIt() async throws {
    let core = FakeCore(.paired)
    let model = SessionModel(core: core, session: working)
    let following = Task { await model.follow() }
    defer { following.cancel() }
    model.text = "and then"
    // The core gives a Prompt back, and a keystroke reaches it before the field has heard.
    core.setDraft("and then", version: 0, for: working)
    FakeCore.giveBack("Tidy the build", in: core, to: working)
    core.setDraft("and then!", version: 0, for: working)
    #expect(core.draft(for: working).text == "Tidy the build\nand then!", "the core keeps what it gave back ahead")
    for _ in 0..<100 where !model.text.hasPrefix("Tidy the build") {
      try await Task.sleep(for: .milliseconds(10))
    }
    #expect(model.text.hasPrefix("Tidy the build\n"), "the field took it in")
    model.text += " and the docs"
    #expect(core.draft(for: working).text == model.text, "once heard, the field's text is taken as it stands")
  }

  @Test func aStopThatWithdrawsAPromptPutsItBackInTheFieldAheadOfWhatIsWritten() async {
    let model = SessionModel(core: FakeCore(.paired), session: "workstation-key/11")
    model.text = "and the changelog"
    await model.stop()
    #expect(model.text == "Note the floor in the pairing docs\nand the changelog")
    #expect(!model.stopping)
  }
}

@Suite @MainActor struct ApprovalModelTests {
  private let waiting = "workstation-key/3"

  /// Follows `model` until `shown` holds of it.
  private func until(_ model: SessionModel, _ shown: @MainActor (SessionModel) -> Bool) async throws {
    for _ in 0..<300 where !shown(model) {
      try await Task.sleep(for: .milliseconds(10))
    }
    #expect(shown(model))
  }

  @Test func theApprovalTheFramePresentsIsTheSheetsItemAndADecisionDimsItUntilTheServerAnswers() async throws {
    let model = SessionModel(core: FakeCore(.paired), session: waiting)
    let following = Task { await model.follow() }
    defer { following.cancel() }
    try await until(model) { $0.presented != nil }
    guard case .approval(let approval) = try #require(model.presented).intervention else {
      Issue.record("an Approval presented")
      return
    }
    #expect(model.presented?.id == "approval:\(approval.reference)")

    let deciding = Task { await model.decide(approval.reference, .acceptOnce) }
    try await until(model) { $0.deciding == [approval.reference] }
    await deciding.value
    #expect(model.deciding.isEmpty)
    #expect(model.banner == nil)
    try await until(model) { $0.presented == nil }
    #expect(model.frame?.bar?.kind == .working)
  }

  @Test func swipingTheSheetAwayLeavesTheOrangeBarAndReviewBringsItBack() async throws {
    let model = SessionModel(core: FakeCore(.paired), session: waiting)
    let following = Task { await model.follow() }
    defer { following.cancel() }
    try await until(model) { $0.presented != nil }
    model.dismissInterventions()
    try await until(model) { $0.presented == nil }
    #expect(model.frame?.bar?.kind == .waiting)
    #expect(model.frame?.bar?.words == "Approval waiting")
    #expect(model.frame?.bar?.review == Review(label: "Review", opens: .interventions))
    #expect(model.frame?.bar?.since == nil)
    model.review()
    try await until(model) { $0.presented != nil }
  }

  @Test func eachDecisionUnderWayDimsItsOwnSheetUntilItsOwnAnswerComes() async throws {
    let model = SessionModel(core: FakeCore(.several), session: "laptop-key/4")
    let following = Task { await model.follow() }
    defer { following.cancel() }
    try await until(model) { $0.presented != nil }
    let first = Task { await model.decide("approval-file-change", .decline) }
    try await until(model) { $0.deciding == ["approval-file-change"] }
    // Asked a second later, so its answer comes a second after the first's.
    try await Task.sleep(for: .seconds(1))
    let second = Task { await model.decide("approval-tool", .acceptOnce) }
    try await until(model) { $0.deciding == ["approval-file-change", "approval-tool"] }
    await first.value
    #expect(model.deciding == ["approval-tool"], "the first answered leaves the second dimmed")
    await second.value
    #expect(model.deciding.isEmpty)
  }

  @Test func aDecisionOnAnApprovalNoLongerWaitingStandsInTheBanner() async {
    let model = SessionModel(core: FakeCore(.paired), session: waiting, failureStands: .seconds(3600))
    await model.decide("no such approval", .decline)
    #expect(model.banner == "That approval is no longer waiting")
    #expect(model.deciding.isEmpty)
    #expect(!model.popped)
  }
}

/// Background time as a test holds it: each task begun and ended, in order.
@MainActor
private final class HeldTime: BackgroundTasks {
  private(set) var held: [String] = []
  /// What each time begun asked to be called should iOS take it back first, in the order it was begun.
  private(set) var expiries: [@MainActor @Sendable () -> Void] = []
  private var next = 1

  func begin(expired: @escaping @MainActor @Sendable () -> Void) -> UIBackgroundTaskIdentifier {
    defer { next += 1 }
    held.append("begin \(next)")
    expiries.append(expired)
    return UIBackgroundTaskIdentifier(rawValue: next)
  }

  func end(_ task: UIBackgroundTaskIdentifier) {
    held.append("end \(task.rawValue)")
  }
}

@Suite @MainActor struct BackgroundTimeTests {
  @Test func timeIsHeldOnlyWhileAnActIsUnderWayAndNeverTwice() async {
    let tasks = HeldTime()
    let (underWay, telling) = AsyncStream<Bool>.makeStream()
    for busy in [false, true, true, false, true] {
      telling.yield(busy)
    }
    telling.finish()
    await BackgroundTime(tasks: tasks).follow(underWay)
    #expect(tasks.held == ["begin 1", "end 1", "begin 2", "end 2"], "and let go as the stream ends")
  }

  @Test func timeIOSTakesBackIsEndedAndHeldAfreshByTheNextAct() async {
    let tasks = HeldTime()
    let time = BackgroundTime(tasks: tasks)
    let (underWay, telling) = AsyncStream<Bool>.makeStream()
    let following = Task { await time.follow(underWay) }
    telling.yield(true)
    while tasks.held.isEmpty { await Task.yield() }
    tasks.expiries[0]()
    #expect(tasks.held == ["begin 1", "end 1"], "ended as iOS takes it back")
    telling.yield(false)
    telling.yield(true)
    telling.finish()
    await following.value
    #expect(tasks.held == ["begin 1", "end 1", "begin 2", "end 2"], "never ended twice")
  }

  @Test func theFakeSaysAnActIsUnderWayWhileOneRuns() async {
    let core = FakeCore(.paired)
    var heard = core.actsUnderWay().makeAsyncIterator()
    #expect(await heard.next() == false)
    let opened = core.openSession("workstation-key/1")
    let sending = Task { try? await opened.send("Steer it") }
    #expect(await heard.next() == true)
    #expect(await heard.next() == false)
    await sending.value
  }
}

@Suite @MainActor struct TryAgainModelTests {
  /// The slim row the fake's listing gives the Remote `name`.
  private func row(on core: FakeCore, _ name: String) async throws -> UnreachableRow {
    var listings = core.listing().makeAsyncIterator()
    let listing = try #require(await listings.next())
    return try #require(listing.unreachable.first { $0.words.hasPrefix(name) || name.isEmpty })
  }

  @Test func aRemoteThatAnswersAgainIsSaidToAndLeavesTheListing() async throws {
    let core = FakeCore(.unreachable)
    let row = try await row(on: core, "homelab")
    #expect(row.sheet.hint == .localNetwork)
    #expect(row.sheet.openSettings == "Open Settings")
    let model = TryAgainModel(core: core, remote: row.remote)

    let trying = Task { await model.tryAgain() }
    await Task.yield()
    #expect(model.trying, "it takes no second try meanwhile")
    #expect(await trying.value)
    #expect(!model.trying)
    #expect(model.failure == nil)
    var listings = core.listing().makeAsyncIterator()
    #expect(try #require(await listings.next()).unreachable.isEmpty)
  }

  @Test func aRemoteStillUnreachableSaysWhyInTheCoresWords() async throws {
    let core = FakeCore(.offline)
    let row = try await row(on: core, "")
    #expect(row.words == "Server unreachable", "one server is never named")
    #expect(row.sheet.hint == .noNetwork)
    #expect(row.sheet.openSettings == nil)
    let model = TryAgainModel(core: core, remote: row.remote)

    #expect(await model.tryAgain() == false)
    #expect(model.failure == "This phone has no network connection")
    #expect(!model.trying)
  }
}

@Suite struct QuestionnaireDraftTests {
  private func question(
    _ choices: [String] = [], multiple: Bool = false, field: AnswerField? = nil, required: Bool = true,
    secret: Bool = false
  ) -> PendingQuestion {
    PendingQuestion(
      position: "Question", heading: nil, text: "Which?", hint: nil,
      choices: choices.map { ChoiceCard(choice: $0, label: $0, description: nil, badge: nil) }, multiple: multiple,
      field: field, skip: required ? nil : "Skip", secret: secret ? "Answered (secret)" : nil)
  }

  private func questionnaire(_ questions: PendingQuestion...) -> PendingQuestionnaire {
    PendingQuestionnaire(
      reference: "questionnaire", questions: questions, note: nil, decline: "Decline", next: "Next",
      send: "Send answers", back: "Back")
  }

  private func field(combines: Bool, secure: Bool = false) -> AnswerField {
    AnswerField(label: combines ? "Additional text" : "Answer", combines: combines, secure: secure)
  }

  @Test func oneChoiceReplacesAnotherAndSeveralMayBeTakenWhereTheQuestionTakesSeveral() {
    let one = question(["staging", "local"])
    let several = question(["unit", "soak"], multiple: true)
    var draft = QuestionnaireDraft(questionnaire(one, several))
    draft.choose("staging", of: one)
    draft.choose("local", of: one)
    #expect(draft.taken("local") && !draft.taken("staging"))
    draft.choose("local", of: one)
    #expect(!draft.taken("local"), "taken again, it is let go")
    draft.choose("staging", of: one)
    #expect(draft.next(questionnaire(one, several)) == false)
    draft.choose("unit", of: several)
    draft.choose("soak", of: several)
    #expect(draft.taken("unit") && draft.taken("soak"))
    #expect(draft.answers.map(\.choices) == [["staging"], ["unit", "soak"]])
  }

  @Test func writingBesideAChoiceKeepsBothAndOtherwiseEachClearsTheOther() {
    let beside = question(["staging"], field: field(combines: true))
    var draft = QuestionnaireDraft(questionnaire(beside))
    draft.choose("staging", of: beside)
    draft.write("after lunch", of: beside)
    #expect(draft.taken("staging") && draft.written == "after lunch", "both stand")

    let instead = question(["staging"], field: field(combines: false))
    var other = QuestionnaireDraft(questionnaire(instead))
    other.choose("staging", of: instead)
    other.write("the old box", of: instead)
    #expect(!other.taken("staging"), "writing clears the choice")
    other.choose("staging", of: instead)
    #expect(other.written.isEmpty, "choosing clears the text")
    #expect(other.taken("staging"))
  }

  @Test func aRequiredQuestionHoldsNextUntilAnsweredAndOneNotRequiredIsSkipped() {
    let required = question(field: field(combines: false))
    let optional = question(["unit"], required: false)
    let asked = questionnaire(required, optional)
    var draft = QuestionnaireDraft(asked)
    #expect(!draft.goesOn(asked))
    #expect(draft.next(asked) == false)
    #expect(draft.page == 0, "Next is held")
    draft.write("   ", of: required)
    #expect(!draft.goesOn(asked), "blanks answer nothing")
    draft.write("staging", of: required)
    #expect(draft.goesOn(asked))
    #expect(draft.next(asked) == false)
    #expect(draft.page == 1)
    #expect(draft.onLast(asked))
    #expect(draft.goesOn(asked), "one not required goes on unanswered")
    draft.choose("unit", of: optional)
    draft.skip(asked)
    #expect(draft.page == 1, "Skip on the last page stays, for Send answers alone sends")
    #expect(!draft.taken("unit"), "it leaves its Question unanswered")
    #expect(draft.answers == [GivenAnswer(choices: [], text: "staging"), GivenAnswer(choices: [], text: "")])
    #expect(draft.next(asked) == true, "Send answers sends")
  }

  @Test func aSecretLeftBehindReadsAnsweredAndShowsNothingOfItUntilReplaced() {
    let secret = question(["prod"], field: field(combines: true, secure: true), required: false, secret: true)
    let after = question(["unit"])
    let asked = questionnaire(secret, after)
    var draft = QuestionnaireDraft(asked)
    draft.choose("prod", of: secret)
    draft.write("hunter2", of: secret)
    #expect(!draft.sealed, "while it is written it is the reader's own")
    _ = draft.next(asked)
    draft.back()
    #expect(draft.sealed)
    #expect(draft.readsSecret)
    #expect(draft.written.isEmpty && !draft.taken("prod"), "nothing of it is shown again")
    #expect(draft.answers[0] == GivenAnswer(choices: ["prod"], text: "hunter2"), "it is still given")
    draft.write("", of: secret)
    #expect(draft.sealed, "an empty field changes nothing")
    draft.write("s", of: secret)
    #expect(!draft.sealed)
    #expect(draft.answers[0] == GivenAnswer(choices: [], text: "s"), "replaced whole")
  }

  @Test func aSecretsChoiceTakenMarksNoCardAndReadsAnswered() {
    let secret = question(["prod", "staging"], required: false, secret: true)
    var draft = QuestionnaireDraft(questionnaire(secret))
    #expect(!draft.readsSecret)
    draft.choose("prod", of: secret)
    #expect(!draft.taken("prod"), "no card shows which was taken")
    #expect(draft.readsSecret)
    #expect(draft.answers[0] == GivenAnswer(choices: ["prod"], text: ""), "it is still given")
  }

  @Test func aSecretWrittenIsSealedWhenTheSheetIsSetAside() {
    let secret = question(field: field(combines: false, secure: true), required: false, secret: true)
    let open = question(field: field(combines: false))
    var draft = QuestionnaireDraft(questionnaire(secret))
    draft.write("hunter2", of: secret)
    draft.setAside()
    #expect(draft.sealed && draft.readsSecret && draft.written.isEmpty)
    var plain = QuestionnaireDraft(questionnaire(open))
    plain.write("staging", of: open)
    plain.setAside()
    #expect(!plain.sealed && plain.written == "staging", "only a secret is sealed")
  }

  @Test func backReturnsToThePageBeforeKeepingWhatWasGiven() {
    let first = question(["staging"])
    let second = question(["unit"])
    let asked = questionnaire(first, second)
    var draft = QuestionnaireDraft(asked)
    draft.back()
    #expect(draft.page == 0)
    draft.choose("staging", of: first)
    _ = draft.next(asked)
    draft.choose("unit", of: second)
    draft.back()
    #expect(draft.page == 0 && draft.taken("staging"))
    _ = draft.next(asked)
    #expect(draft.taken("unit"))
  }
}

@Suite @MainActor struct QuestionnaireModelTests {
  private let waiting = "workstation-key/13"

  private func until(_ model: SessionModel, _ shown: @MainActor (SessionModel) -> Bool) async throws {
    for _ in 0..<300 where !shown(model) {
      try await Task.sleep(for: .milliseconds(10))
    }
    #expect(shown(model))
  }

  private func presented(_ model: SessionModel) throws -> PendingQuestionnaire {
    guard case .questionnaire(let questionnaire) = try #require(model.presented).intervention else {
      throw CancellationError()
    }
    return questionnaire
  }

  @Test func halfGivenAnswersAreKeptWhileItWaitsBehindReview() async throws {
    let model = SessionModel(core: FakeCore(.paired), session: waiting)
    let following = Task { await model.follow() }
    defer { following.cancel() }
    try await until(model) { $0.presented != nil }
    let questionnaire = try presented(model)
    #expect(model.presented?.id == "questionnaire:\(questionnaire.reference)")
    let machine = questionnaire.questions[0]
    model.changeDraft(of: questionnaire) { $0.choose("local", of: machine) }
    model.changeDraft(of: questionnaire) { $0.write("after lunch", of: machine) }
    #expect(model.changeDraft(of: questionnaire) { $0.next(questionnaire) } == false)

    model.dismissInterventions()
    try await until(model) { $0.presented == nil }
    #expect(model.frame?.bar?.words == "Questionnaire waiting")
    #expect(model.frame?.bar?.review == Review(label: "Review", opens: .interventions))
    model.review()
    try await until(model) { $0.presented != nil }
    let kept = model.draft(of: try presented(model))
    #expect(kept.page == 1, "on the page it was left on")
    #expect(kept.answers[0] == GivenAnswer(choices: ["local"], text: "after lunch"))
  }

  @Test func answersSentDimTheSheetUntilTheServerAnswersAndGoWithIt() async throws {
    let model = SessionModel(core: FakeCore(.paired), session: waiting)
    let following = Task { await model.follow() }
    defer { following.cancel() }
    try await until(model) { $0.presented != nil }
    let questionnaire = try presented(model)
    model.changeDraft(of: questionnaire) { $0.choose("staging", of: questionnaire.questions[0]) }
    let sending = Task { await model.sendAnswers(questionnaire) }
    try await until(model) { $0.answering == [questionnaire.reference] }
    await sending.value
    #expect(model.answering.isEmpty)
    #expect(model.banner == nil)
    #expect(model.questionnaireDrafts.isEmpty, "what was given goes once it is answered")
    try await until(model) { $0.presented == nil }
    #expect(model.frame?.bar?.kind == .working)
  }

  @Test func answersRefusedStandInTheBannerAndAsItAsksAgainWhatWasGivenIsKept() async throws {
    let model = SessionModel(core: FakeCore(.paired), session: waiting, failureStands: .seconds(3600))
    let following = Task { await model.follow() }
    defer { following.cancel() }
    try await until(model) { $0.presented != nil }
    let questionnaire = try presented(model)
    model.changeDraft(of: questionnaire) { $0.write("fail, please", of: questionnaire.questions[0]) }
    await model.sendAnswers(questionnaire)
    #expect(model.banner == "The agent did not take the answers.")
    try await until(model) { (try? presented($0).note) != nil }
    #expect(
      try presented(model).note == "Your answers were not delivered. Look them over and send them again.")
    #expect(model.draft(of: questionnaire).answers[0].text == "fail, please", "kept to send again")
  }

  @Test func aSecretWrittenThenSwipedAwayReadsAnsweredWhenReviewed() async throws {
    let model = SessionModel(core: FakeCore(.paired), session: waiting)
    let following = Task { await model.follow() }
    defer { following.cancel() }
    try await until(model) { $0.presented != nil }
    let questionnaire = try presented(model)
    model.changeDraft(of: questionnaire) { $0.choose("staging", of: questionnaire.questions[0]) }
    _ = model.changeDraft(of: questionnaire) { $0.next(questionnaire) }
    _ = model.changeDraft(of: questionnaire) { $0.next(questionnaire) }
    model.changeDraft(of: questionnaire) { $0.write("hunter2", of: questionnaire.questions[2]) }
    model.dismissInterventions()
    try await until(model) { $0.presented == nil }
    model.review()
    try await until(model) { $0.presented != nil }
    let kept = model.draft(of: try presented(model))
    #expect(kept.page == 2 && kept.readsSecret && kept.written.isEmpty)
  }

  @Test func whatWasHalfGivenGoesOnceTheFrameNoLongerListsItLive() async throws {
    let model = SessionModel(core: FakeCore(.paired), session: waiting)
    let following = Task { await model.follow() }
    defer { following.cancel() }
    try await until(model) { $0.presented != nil }
    let questionnaire = try presented(model)
    model.changeDraft(of: questionnaire) { $0.choose("local", of: questionnaire.questions[0]) }
    model.dismissInterventions()
    try await until(model) { $0.presented == nil }
    #expect(model.questionnaireDrafts.keys.contains(questionnaire.reference), "dismissed, it is kept")
    // The Turn stopped, the Questionnaire waits on no one, and what was given goes with it.
    await model.stop()
    try await until(model) { $0.frame?.waiting.isEmpty == true }
    #expect(model.questionnaireDrafts.isEmpty)
  }

  @Test func declinedItGoesWhole() async throws {
    let model = SessionModel(core: FakeCore(.paired), session: waiting)
    let following = Task { await model.follow() }
    defer { following.cancel() }
    try await until(model) { $0.presented != nil }
    await model.decline(try presented(model))
    #expect(model.banner == nil)
    try await until(model) { $0.presented == nil }
  }
}

@Suite @MainActor struct KeptDraftTests {
  /// Follows `model` until the Landing stands, for as long as a moment.
  private func landed(_ model: NewSessionModel) async throws {
    for _ in 0..<200 where model.landing == nil {
      try await Task.sleep(for: .milliseconds(10))
    }
    #expect(model.landing != nil)
  }

  /// The new-session screen over `core`, followed until its Landing stands, and what follows it.
  private func screen(over core: any Core) async throws -> (NewSessionModel, Task<Void, Never>) {
    let model = NewSessionModel(core: core)
    let following = Task { await model.follow() }
    try await landed(model)
    return (model, following)
  }

  @Test func whatTheLandingsFieldHeldAsTheAppLeftTheFrontIsInItTheNextRunOverTheLiveCore() async throws {
    let base = try scratch()
    defer { try? FileManager.default.removeItem(at: base) }
    let directory = try CoreDirectory.prepare(in: base)
    let key = try madeKey()
    do {
      let core = try LiveCore(directory: directory, signer: key)
      let (model, following) = try await screen(over: core)
      model.text = "half a thought"
      core.leftFront()
      following.cancel()
      await following.value
    }
    #expect(
      FileManager.default.fileExists(atPath: directory.appending(path: "drafts.json").path(percentEncoded: false)),
      "kept in the core's directory")

    let core = try LiveCore(directory: directory, signer: key)
    let (model, following) = try await screen(over: core)
    defer { following.cancel() }
    #expect(model.text == "half a thought")
  }

  @Test func theFakeKeepsItsDraftsInTheFileItIsGivenAsTheAppLeavesTheFront() async throws {
    let base = try scratch()
    defer { try? FileManager.default.removeItem(at: base) }
    let file = base.appending(path: "drafts.json")
    let core = FakeCore(.composing, keepingDraftsIn: file)
    core.setDraft("half a thought", version: 0, for: FakeOpenedSession.landingDraft)
    core.setDraft("  ", version: 0, for: "workstation-key/1")
    #expect(!FileManager.default.fileExists(atPath: file.path(percentEncoded: false)), "not before it leaves")
    core.leftFront()

    let again = FakeCore(.composing, keepingDraftsIn: file)
    #expect(again.draft(for: FakeOpenedSession.landingDraft) == Draft(text: "half a thought", version: 0))
    let (model, following) = try await screen(over: again)
    defer { following.cancel() }
    #expect(model.text == "half a thought")
    #expect(FakeCore(.composing).draft(for: FakeOpenedSession.landingDraft).text == "", "kept only where told to")
  }
}

@Suite @MainActor struct NewSessionModelTests {
  /// Follows `model` until `shown` holds of it, for as long as a moment.
  private func until(_ model: NewSessionModel, _ shown: @MainActor (NewSessionModel) -> Bool) async throws {
    for _ in 0..<200 where !shown(model) {
      try await Task.sleep(for: .milliseconds(10))
    }
    #expect(shown(model))
  }

  @Test func theScreenOpensOnTheLandingsOwnDraftAndHandsTheCoreEachChange() async throws {
    let core = FakeCore(.composing)
    core.setDraft("half a thought", version: 0, for: FakeOpenedSession.landingDraft)
    let model = NewSessionModel(core: core)
    let following = Task { await model.follow() }
    defer { following.cancel() }
    try await until(model) { $0.landing != nil }
    #expect(model.landing?.place == "shaman", "with one Remote no Origin is named")
    #expect(model.landing?.agent == "Claude · Fable 5.2 · high")
    #expect(model.text == "half a thought")
    model.text = "half a thought, finished"
    #expect(core.draft(for: FakeOpenedSession.landingDraft).text == "half a thought, finished")
    #expect(core.draft(for: "workstation-key/1").text == "", "a draft of its own")
  }

  @Test func sendingTurnsTheScreenIntoTheSessionBegunAndEmptiesTheDraft() async throws {
    let core = FakeCore(.composing)
    let model = NewSessionModel(core: core)
    var adopted: SessionModel?
    model.adopted = { adopted = $0 }
    let following = Task { await model.follow() }
    defer { following.cancel() }
    try await until(model) { $0.landing != nil }
    model.text = "Tidy the build"
    model.send()
    let begun = try #require(model.begun)
    #expect(adopted === begun, "the navigation stack holds it")
    #expect(begun.session.hasPrefix("workstation-key/"), "under the identity the core chose")
    #expect(model.text == "")
    #expect(core.draft(for: FakeOpenedSession.landingDraft).text == "")
  }

  @Test func whereNothingCanBeginSendDoesNothing() async throws {
    let model = NewSessionModel(core: FakeCore(.paired))
    let following = Task { await model.follow() }
    defer { following.cancel() }
    try await until(model) { $0.landing != nil }
    #expect(model.landing?.canSend == false)
    model.text = "Tidy the build"
    model.send()
    #expect(model.begun == nil)
    #expect(model.text == "Tidy the build", "kept as the draft")
  }

  @Test func theStackHoldsTheSessionBegunForAsLongAsItsScreenIsPushed() async throws {
    let core = FakeCore(.composing)
    let stack = SessionStack(core: core)
    let route = LandingRoute()
    stack.path = [.landing(route)]
    let model = stack.landing(for: route)
    #expect(stack.landing(for: route) === model, "made once for its screen")
    let following = Task { await model.follow() }
    defer { following.cancel() }
    try await until(model) { $0.landing != nil }
    model.text = "Tidy the build"
    model.send()
    let begun = try #require(model.begun)
    #expect(stack.model(for: begun.route) === begun, "what it pushes reads through it")
    stack.path.append(.work(begun.route, turn: "turn-begun"))
    stack.path.removeLast()
    let session = try #require(core.opened.withLock { $0.last })
    #expect(!session.isClosed, "its screen is still pushed")
    stack.path = []
    #expect(session.isClosed, "popped to the listing, so closed")
  }

  @Test func theSessionBegunChosenInTheTreeOfOnePushedOverItIsPoppedBackTo() async throws {
    let core = FakeCore(.composing)
    let stack = SessionStack(core: core)
    let route = LandingRoute()
    stack.path = [.landing(route)]
    let model = stack.landing(for: route)
    let following = Task { await model.follow() }
    defer { following.cancel() }
    try await until(model) { $0.landing != nil }
    model.text = "Tidy the build"
    model.send()
    let begun = try #require(model.begun)
    let subagent = SessionRoute(session: "\(begun.session).1")
    stack.path.append(.session(subagent))

    stack.show(begun.session, over: subagent)
    #expect(stack.path == [.landing(route)], "it stands on the path as the screen it was begun from")
  }

  @Test func aRefusedBeginningLeftIsTheLandingsDraftAgain() async throws {
    let core = FakeCore(.composing)
    let model = NewSessionModel(core: core)
    let following = Task { await model.follow() }
    defer { following.cancel() }
    try await until(model) { $0.landing != nil }
    model.text = "Refuse this one"
    model.send()
    let begun = try #require(model.begun)
    let frames = Task { await begun.follow() }
    defer { frames.cancel() }
    for _ in 0..<400 where begun.frame?.composer?.canSend != true {
      try await Task.sleep(for: .milliseconds(10))
    }
    #expect(begun.frame?.bar == nil, "refused")
    begun.close()
    #expect(core.draft(for: FakeOpenedSession.landingDraft).text == "Refuse this one")
  }

  @Test func aNewWorktreeChosenIsMadeBeforeTheSessionStarts() async throws {
    let core = FakeCore(.composing)
    let model = NewSessionModel(core: core)
    let following = Task { await model.follow() }
    defer { following.cancel() }
    try await until(model) { $0.landing?.worktree != nil }
    #expect(model.landing?.worktree?.main == "On main")
    #expect(model.landing?.worktree?.new == "In a new worktree")
    #expect(!model.newWorktree, "the main Worktree as the screen opens")
    model.newWorktree = true
    model.text = "Trim the quiet zone"
    model.send()
    let begun = try #require(model.begun)
    let frames = Task { await begun.follow() }
    defer { frames.cancel() }
    for _ in 0..<200 where begun.frame == nil {
      try await Task.sleep(for: .milliseconds(10))
    }
    #expect(begun.frame?.bar?.words == "Creating worktree")
  }

  @Test func turningToAnotherWorkspaceStartsTheWorktreeChoiceOnTheMainOneAgain() async throws {
    let core = FakeCore(.composing)
    let model = NewSessionModel(core: core)
    let following = Task { await model.follow() }
    defer { following.cancel() }
    try await until(model) { $0.landing?.worktree != nil }
    model.newWorktree = true
    let picker = model.picker()
    #expect(
      await picker.choose(
        WorkspaceRow(
          workspace: "workspace/workstation/dotfiles", glyph: nil, name: "dotfiles", description: nil, chosen: false,
          unavailable: nil)))
    try await until(model) { $0.landing?.workspace?.name == "dotfiles" }
    #expect(!model.newWorktree)
  }

  @Test func aBareRepositoryOffersNoWorktreeAndNoSend() async throws {
    let core = FakeCore(.composing)
    let model = NewSessionModel(core: core)
    let following = Task { await model.follow() }
    defer { following.cancel() }
    try await until(model) { $0.landing != nil }
    try await core.chooseWorkspace("workspace/workstation/mirror.git")
    try await until(model) { $0.landing?.workingCopy != nil }
    #expect(model.landing?.worktree == nil)
    #expect(model.landing?.canSend == false)
    model.text = "Mirror the tags"
    model.send()
    #expect(model.begun == nil)
  }
}

@Suite @MainActor struct WorkspacePickerModelTests {
  private func until(_ model: WorkspacePickerModel, _ shown: @MainActor (WorkspacePickerModel) -> Bool) async throws {
    for _ in 0..<200 where !shown(model) {
      try await Task.sleep(for: .milliseconds(10))
    }
    #expect(shown(model))
  }

  private func row(_ model: WorkspacePickerModel, _ workspace: String) throws -> WorkspaceRow {
    try #require(model.picker?.sections.flatMap(\.rows).first { $0.workspace == workspace })
  }

  @Test func everyRemotesWorkspacesStandUnderItsHeaderAndChoosingOneTicksIt() async throws {
    let core = FakeCore(.several)
    let model = WorkspacePickerModel(core: core)
    let following = Task { await model.follow() }
    defer { following.cancel() }
    try await until(model) { $0.picker != nil }
    #expect(model.picker?.title == "Begin in")
    #expect(model.picker?.sections.map(\.header) == ["workstation", "laptop", "homelab"])
    let billing = try row(model, "workspace/laptop/billing-api")
    #expect(!billing.chosen)
    #expect(await model.choose(billing))
    try await until(model) { (try? self.row($0, "workspace/laptop/billing-api"))?.chosen == true }
    var outlooks = core.outlook().makeAsyncIterator()
    let outlook = await outlooks.next()
    guard case .remote(let name, let beginsIn) = outlook else {
      Issue.record("the Outlook stands nowhere")
      return
    }
    #expect(name == "laptop")
    #expect(beginsIn.place == "laptop · billing-api")
  }

  @Test func aWorkspaceItsServerCannotReadIsNotChosen() async throws {
    let model = WorkspacePickerModel(core: FakeCore(.paired))
    let following = Task { await model.follow() }
    defer { following.cancel() }
    try await until(model) { $0.picker != nil }
    let archive = try row(model, "workspace/workstation/photo-archive")
    #expect(archive.unavailable == "Unavailable: its directory is gone")
    #expect(await model.choose(archive) == false)
    #expect(model.banner == nil)
  }

  @Test func aSearchIsHandedToTheCoreWhichSaysWhereItFindsNothing() async throws {
    let model = WorkspacePickerModel(core: FakeCore(.several))
    let following = Task { await model.follow() }
    defer { following.cancel() }
    try await until(model) { $0.picker != nil }
    model.search("relay")
    try await until(model) { $0.picker?.sections.count == 1 }
    #expect(model.picker?.sections.first?.rows.map(\.name) == ["relay-infra"])
    model.search("kernel")
    try await until(model) { $0.picker?.empty != nil }
    #expect(model.picker?.empty == "No workspaces found")
  }

  @Test func aSearchSaysWhichRemotesHaveYetToListTheirsAndNeverThatItFoundNothing() async throws {
    let model = WorkspacePickerModel(core: FakeCore(.reaching))
    let following = Task { await model.follow() }
    defer { following.cancel() }
    try await until(model) { $0.picker != nil }
    model.search("kernel")
    try await until(model) { $0.picker?.sections.count == 1 }
    #expect(model.picker?.empty == nil, "the home lab may yet list one")
    #expect(model.picker?.sections.first?.header == "homelab")
    #expect(model.picker?.sections.first?.line == "Loading workspaces…")
  }

}

@Suite struct KeptSceneTests {
  @Test func whatTheSceneKeepsCrossesItsPlainStringWhole() {
    let kept = KeptScene(
      scope: "workspace/workstation-key/dotfiles",
      pushed: [.session("workstation-key/12"), .session("workstation-key/12.1")]
    )
    #expect(KeptScene(stored: kept.stored) == kept)
    #expect(KeptScene(stored: KeptScene(pushed: [.landing]).stored).pushed == [.landing])
  }

  @Test func aNewSceneOrOneKeptByAnotherBuildKeepsNothing() {
    for stored in ["", "not json", "{\"pushed\": 3}"] {
      #expect(KeptScene(stored: stored) == KeptScene(), "\(stored)")
    }
  }
}

@Suite @MainActor struct RestoredStackTests {
  /// Waits until `shown` holds, for as long as a moment.
  private func until(_ shown: @MainActor () -> Bool) async throws {
    for _ in 0..<300 where !shown() {
      try await Task.sleep(for: .milliseconds(10))
    }
    #expect(shown())
  }

  @Test func theSceneKeepsEachSessionPushedAndTheNewSessionScreenButNoTurnsWorkOrActivity() {
    let core = FakeCore(.paired)
    let parent = SessionRoute(session: "workstation-key/12")
    let explorer = SessionRoute(session: "workstation-key/12.1")
    let stack = SessionStack(core: core)
    #expect(stack.kept.isEmpty)
    stack.path = [
      .session(parent), .work(parent, turn: "work"), .session(explorer), .work(explorer, turn: "work"),
      .detail(explorer, activity: "check"),
    ]
    #expect(stack.kept == [.session("workstation-key/12"), .session("workstation-key/12.1")])
    stack.path = [.landing(LandingRoute())]
    #expect(stack.kept == [.landing])
  }

  @Test func aStackKeptIsPushedAgainEachScreenReadAfresh() {
    let core = FakeCore(.paired)
    let stack = SessionStack(
      core: core, restoring: [.session("workstation-key/12"), .session("workstation-key/12.1")])
    guard case .session(let parent) = stack.path.first, case .session(let explorer) = stack.path.last,
      stack.path.count == 2
    else {
      Issue.record("two Sessions' screens pushed: \(stack.path)")
      return
    }
    #expect(parent.session == "workstation-key/12")
    #expect(explorer.session == "workstation-key/12.1")
    #expect(core.opened.withLock { $0.isEmpty }, "nothing opened until its screen is drawn")
    _ = stack.model(for: explorer)
    #expect(core.opened.withLock { $0.count } == 1)
    #expect(stack.kept == [.session("workstation-key/12"), .session("workstation-key/12.1")])

    let landing = SessionStack(core: core, restoring: [.landing])
    guard case .landing = landing.path.first, landing.path.count == 1 else {
      Issue.record("the new-session screen pushed: \(landing.path)")
      return
    }
  }

  @Test func aSessionKeptThatIsGoneSendsTheReaderBackToTheListing() async throws {
    // The `several` phone's home lab is not held by the `paired` one.
    let core = FakeCore(.paired)
    let stack = SessionStack(core: core, restoring: [.session("workstation-key/1"), .session("homelab-key/6")])
    guard case .session(let gone) = stack.path.last else {
      Issue.record("a Session pushed")
      return
    }
    let model = stack.model(for: gone)
    let following = Task { await model.follow() }
    defer { following.cancel() }
    try await until { stack.path.isEmpty }
    #expect(core.opened.withLock { $0.allSatisfy(\.isClosed) }, "every Session closed with its screen")
  }

  @Test func aSessionKeptIsSentBackToTheListingOnlyUntilItHasBeenRead() async throws {
    let core = FakeCore(.paired)
    let stack = SessionStack(core: core, restoring: [.session("workstation-key/7")])
    guard case .session(let route) = stack.path.first else {
      Issue.record("a Session pushed")
      return
    }
    let model = stack.model(for: route)
    #expect(model.whenGone != nil, "found gone as it is read again, it leaves nothing to return to")
    let following = Task { await model.follow() }
    defer { following.cancel() }
    try await until { !model.conversation.shown.isEmpty }
    // Read, it is the Session the reader is in: gone later, its screen says so, as a tapped one's does.
    try await until { model.whenGone == nil }
    #expect(stack.path.count == 1)
  }

  @Test func aSessionPushedWhileTheAppRunsThatIsGoneStaysToSaySo() async throws {
    let core = FakeCore(.paired)
    let stack = SessionStack(core: core)
    let gone = SessionRoute(session: "homelab-key/6")
    stack.path = [.session(gone)]
    let model = stack.model(for: gone)
    let following = Task { await model.follow() }
    defer { following.cancel() }
    try await until { model.frame?.gone != nil }
    #expect(stack.path.count == 1, "its screen says it is gone")
  }

  @Test func aNewSessionScreenThatHasBegunItsSessionIsKeptAsThatSession() async throws {
    let core = FakeCore(.composing)
    let route = LandingRoute()
    let stack = SessionStack(core: core)
    stack.path = [.landing(route)]
    let landing = stack.landing(for: route)
    let following = Task { await landing.follow() }
    defer { following.cancel() }
    try await until { landing.landing != nil }
    #expect(stack.kept == [.landing], "before its first Prompt")
    landing.text = "Map the workspace"
    landing.send()
    let begun = try #require(landing.begun)
    #expect(stack.kept == [.session(begun.session)])
  }
}

@Suite @MainActor struct RestoredListingTests {
  /// Waits until the model's listing shows what `shown` looks for.
  private func listing(of model: ListingModel, until shown: (Listing) -> Bool) async throws -> Listing {
    for _ in 0..<200 {
      if let listing = model.listing, shown(listing) { return listing }
      try await Task.sleep(for: .milliseconds(10))
    }
    Issue.record("the listing never showed what was looked for")
    return try #require(model.listing)
  }

  @Test func theScopeKeptNarrowsTheListingAgainAndTurnsNothing() async throws {
    let core = FakeCore(.several)
    let model = ListingModel(core: core)
    let following = Task { await model.follow(restoring: "workspace/laptop-key/billing-api") }
    defer { following.cancel() }
    let listing = try await listing(of: model) { _ in true }
    #expect(listing.title == "laptop · billing-api")
    #expect(listing.scope == "workspace/laptop-key/billing-api")
    var outlooks = core.outlook().makeAsyncIterator()
    guard case .remote(let name, _) = await outlooks.next() else {
      Issue.record("an Outlook")
      return
    }
    #expect(name == "workstation", "the Outlook stays where it stood")
  }

  @Test func aScopeKeptThatIsGoneNarrowsTheListingToEverywhere() async throws {
    for gone in ["remote/homelab-key", "workspace/homelab-key/home-assistant", "workspace/workstation-key/gone"] {
      let model = ListingModel(core: FakeCore(.paired))
      let following = Task { await model.follow(restoring: gone) }
      defer { following.cancel() }
      let listing = try await listing(of: model) { _ in true }
      #expect(listing.scope == "everywhere", "\(gone)")
      #expect(listing.title == "All workspaces", "\(gone)")
    }
  }
}

/// A browser that records what it was asked to open and close.
@MainActor
private final class RecordingBrowser: LoginBrowser {
  private(set) var opened: [URL] = []
  private(set) var closed = 0

  func open(_ url: URL) {
    opened.append(url)
  }

  func close() {
    closed += 1
  }
}

@Suite @MainActor struct RelayLoginModelTests {
  @Test func theFirstCodeIsCopiedAndItsAddressOpenedAndBothCloseOnceTheRelayConfirms() async {
    let core = FakeCore(.relayLogin, loginConfirmsAfter: .milliseconds(300))
    let browser = RecordingBrowser()
    var copied: [String] = []
    let model = RelayLoginModel(core: core, relay: FakeCore.relay, browser: browser) { copied.append($0) }

    let confirmed = await model.follow()

    #expect(confirmed)
    #expect(copied == ["WDJB-MJHT"])
    #expect(browser.opened == [URL(string: "https://github.com/login/device")!])
    #expect(browser.closed >= 1, "the authentication sheet is cancelled once the relay confirms")
    #expect(model.title == "Log in at relay.example.com")
    guard case .done = model.login else {
      Issue.record("the login is done: \(String(describing: model.login))")
      return
    }
  }

  @Test func leavingTheFrontGivesTheLoginUpAndClosesTheSheet() async throws {
    let core = FakeCore(.relayLogin, loginConfirmsAfter: .seconds(60))
    let browser = RecordingBrowser()
    let model = RelayLoginModel(core: core, relay: FakeCore.relay, browser: browser) { _ in }
    let following = Task { await model.follow() }
    while browser.opened.isEmpty {
      try await Task.sleep(for: .milliseconds(20))
    }

    core.leftFront()

    #expect(await following.value == false)
    #expect(browser.closed >= 1)
    guard case .failed(_, let message, _) = model.login else {
      Issue.record("the login failed: \(String(describing: model.login))")
      return
    }
    #expect(message.contains("left the screen"))
  }

  @Test func cancellingWhatFollowsTheLoginGivesItUpAndKeepsNothing() async throws {
    let core = FakeCore(.relayLogin, loginConfirmsAfter: .seconds(60))
    let browser = RecordingBrowser()
    let model = RelayLoginModel(core: core, relay: FakeCore.relay, browser: browser) { _ in }
    let following = Task { await model.follow() }
    while browser.opened.isEmpty {
      try await Task.sleep(for: .milliseconds(20))
    }
    var servers = core.servers().makeAsyncIterator()
    #expect(await servers.next()?.relays.first?.state == .loggingIn)

    following.cancel()

    #expect(await following.value == false)
    #expect(browser.closed >= 1)
    var after = core.servers().makeAsyncIterator()
    #expect(await after.next()?.relays.isEmpty == true, "nothing is kept of a login given up")
  }

  @Test func copyingAndOpeningAgainUseTheCodeShown() async throws {
    let core = FakeCore(.relayLogin, loginConfirmsAfter: .seconds(60))
    let browser = RecordingBrowser()
    var copied: [String] = []
    let model = RelayLoginModel(core: core, relay: FakeCore.relay, browser: browser) { copied.append($0) }
    let following = Task { await model.follow() }
    while browser.opened.isEmpty {
      try await Task.sleep(for: .milliseconds(20))
    }

    model.copyAgain()
    model.openAgain()

    #expect(copied == ["WDJB-MJHT", "WDJB-MJHT"])
    #expect(browser.opened.count == 2)
    following.cancel()
    _ = await following.value
  }
}

@Suite @MainActor struct ServersSectionTests {
  /// The model of a sheet over `core`, following its Remotes, once it holds them.
  private func following(_ core: FakeCore) async throws -> (DeviceModel, Task<Void, Never>) {
    let model = DeviceModel(core: core)
    let task = Task { await model.followServers() }
    while model.servers == nil {
      try await Task.sleep(for: .milliseconds(10))
    }
    return (model, task)
  }

  private func row(_ name: String, in model: DeviceModel) throws -> ServerRow {
    try #require(model.servers?.remotes.first { $0.name == name })
  }

  /// Waits until `done` holds.
  private func until(_ done: () -> Bool) async throws {
    for _ in 0..<300 where !done() {
      try await Task.sleep(for: .milliseconds(10))
    }
    #expect(done())
  }

  @Test func aRefusedNameStaysBesideWhyAndAKeptOneReadsAsTheCoreHoldsIt() async throws {
    let (model, task) = try await following(FakeCore(.several))
    defer { task.cancel() }
    let workstation = try row("workstation", in: model)
    let name = model.name(of: workstation)

    name.wrappedValue = "laptop"
    model.rename(workstation)
    try await until { model.renamingRemote == nil && model.nameRefusals[workstation.remote] != nil }
    #expect(model.nameRefusals[workstation.remote] == "This phone holds another server as laptop; choose another name")
    model.leftNaming(workstation.remote)
    #expect(name.wrappedValue == "laptop", "what was refused stays beside why")

    name.wrappedValue = "studio"
    model.rename(workstation)
    #expect(model.renamingRemote == workstation.remote, "taken at once, before the field is left")
    model.leftNaming(workstation.remote)
    try await until { model.renamingRemote == nil }
    #expect(model.nameRefusals[workstation.remote] == nil)
    try await until { model.servers?.remotes.first?.name == "studio" }
    #expect(model.name(of: try row("studio", in: model)).wrappedValue == "studio")
  }

  @Test func aNameLeftUnkeptReadsAsTheCoreHoldsItAgain() async throws {
    let (model, task) = try await following(FakeCore(.several))
    defer { task.cancel() }
    let laptop = try row("laptop", in: model)
    let name = model.name(of: laptop)
    name.wrappedValue = "attic"
    model.leftNaming(laptop.remote)
    #expect(name.wrappedValue == "laptop")
    model.rename(laptop)
    #expect(model.renamingRemote == nil, "nothing changed, nothing asked")
  }

  @Test func aRemovalConfirmedGoesAndOneOfARemoteGoneSaysSoInTheBanner() async throws {
    let (model, task) = try await following(FakeCore(.several))
    defer { task.cancel() }
    let workstation = try row("workstation", in: model)
    model.removingRemote = workstation
    #expect(model.removingRemoteShown.wrappedValue)

    await model.remove(workstation)
    #expect(model.removingRemote == nil && model.removingNow == nil)
    #expect(model.failure == nil)
    try await until { model.servers?.remotes.map(\.name) == ["laptop", "homelab"] }

    await model.remove(workstation)
    #expect(model.failure == "This phone is no longer paired with that server")
  }
}

@Suite struct LiveRemotesTests {
  @Test func aRemoteTheCoreDoesNotHoldIsGoneToRenameOrRemove() async throws {
    let base = try scratch()
    defer { try? FileManager.default.removeItem(at: base) }
    let core = try LiveCore(directory: try CoreDirectory.prepare(in: base), signer: try madeKey())
    let gone = ActError.Gone(message: "This phone is no longer paired with that server")
    await #expect(throws: gone) { try await core.renameRemote("not-a-remote", to: "study") }
    await #expect(throws: gone) { try await core.removeRemote("not-a-remote") }
  }
}

/// A scanner that counts how often it was asked whether it may scan.
@MainActor
private final class CountedScanner {
  private(set) var asked = 0
  private let access: ScanAccess

  init(_ access: ScanAccess) {
    self.access = access
  }

  var scanner: CodeScanner {
    CodeScanner(
      access: { [self] in
        asked += 1
        return access
      },
      view: { _, _ in AnyView(EmptyView()) })
  }
}

@Suite @MainActor struct ScanOfferTests {
  @Test func theCameraIsAskedForOnlyOnceScanIsPressed() async {
    let counted = CountedScanner(.granted)
    let offer = ScanOffer()
    #expect(counted.asked == 0)
    #expect(offer.access == nil)
    #expect(await offer.scan(with: counted.scanner))
    #expect(counted.asked == 1)
    #expect(offer.access == .granted)
  }

  @Test func aRefusedCameraOpensNoScannerAndSaysSo() async {
    let counted = CountedScanner(.refused)
    let offer = ScanOffer()
    #expect(await offer.scan(with: counted.scanner) == false)
    #expect(offer.access == .refused)
    // Pressed again, as after allowing it in the Settings app, it asks again.
    #expect(await offer.scan(with: counted.scanner) == false)
    #expect(counted.asked == 2)
  }

  @Test func aPhoneWithNoCameraOpensNoScanner() async {
    let offer = ScanOffer()
    #expect(await offer.scan(with: CountedScanner(.unavailable).scanner) == false)
    #expect(offer.access == .unavailable)
  }
}

@Suite @MainActor struct ScanningModelTests {
  /// A model whose screen stands at the top of its stack, as one just pushed does.
  private func atTop() -> ScanningModel {
    let model = ScanningModel(core: FakeCore(.fresh))
    model.stands(atTop: true)
    return model
  }

  @Test func itScansOnlyOnceItStandsAtTheTop() {
    let model = ScanningModel(core: FakeCore(.fresh))
    #expect(model.scanning == false)
    #expect(model.heard(.read(FakeCore.invite)) == nil)
    model.stands(atTop: true)
    #expect(model.scanning)
  }

  @Test func whatIsNoInviteIsPassedOverAndTheFirstInviteStopsTheScanner() {
    let model = atTop()
    for other in ["https://example.com", "WIFI:S:home;T:WPA;P:secret;;", "shaman"] {
      #expect(model.heard(.read(other)) == nil)
      #expect(model.scanning, "it keeps scanning past \(other)")
      #expect(model.passedOver == other)
    }
    #expect(model.heard(.read(FakeCore.invite)) == FakeCore.invite)
    #expect(model.scanning == false)
    #expect(model.passedOver == nil)
    #expect(model.heard(.read("shaman-v1-fake-another")) == nil, "only the first Invite is taken")
  }

  @Test func thePassedOverNoteGoesOnceItsCodeLeavesTheFrame() {
    let model = atTop()
    #expect(model.heard(.read("https://example.com")) == nil)
    #expect(model.heard(.left("https://elsewhere.example.com")) == nil)
    #expect(model.passedOver == "https://example.com", "another code leaving leaves the note")
    #expect(model.heard(.left("https://example.com")) == nil)
    #expect(model.passedOver == nil)
  }

  @Test func aMalformedInviteIsTakenSoItsConfirmationSaysWhatIsWrong() {
    let model = atTop()
    #expect(model.heard(.read("shaman-v2-from-a-newer-shaman")) == "shaman-v2-from-a-newer-shaman")
  }

  /// A back swipe begun from the confirmation and given up shows the scanner for a moment, and takes it off screen
  /// again, while its path still ends at the confirmation: nothing it sees then is taken.
  @Test func coveredOrOffScreenItTakesNothing() {
    let model = atTop()
    #expect(model.heard(.read(FakeCore.invite)) == FakeCore.invite)
    model.stands(atTop: false)
    #expect(model.scanning == false)
    #expect(model.heard(.left(FakeCore.invite)) == nil)
    #expect(model.heard(.read("shaman-v1-fake-another")) == nil)
    model.stands(atTop: false)
    #expect(model.heard(.read("shaman-v1-fake-another")) == nil)
  }

  @Test func returnedToItPassesOverTheInviteItTookUntilThatCodeLeavesTheFrame() {
    let model = atTop()
    #expect(model.heard(.read("shaman-v1-dead")) == "shaman-v1-dead")
    model.stands(atTop: false)
    model.stands(atTop: true)
    #expect(model.scanning)
    #expect(model.heard(.read("shaman-v1-dead")) == nil, "the dead Invite still in front of the camera")
    #expect(model.scanning)
    #expect(model.passedOver == nil, "an Invite is not said to be no invite")
    #expect(model.heard(.read(FakeCore.invite)) == FakeCore.invite, "another is taken")
    model.stands(atTop: false)
    model.stands(atTop: true)
    #expect(model.heard(.left(FakeCore.invite)) == nil)
    #expect(model.heard(.read(FakeCore.invite)) == FakeCore.invite, "once it left, it is read again")
  }

  @Test func aCameraThatFailsScansNoMore() {
    let model = atTop()
    #expect(model.heard(.failed) == nil)
    #expect(model.failed)
    #expect(model.scanning == false)
    #expect(model.heard(.read(FakeCore.invite)) == nil)
  }

  @Test func itSaysWhatTheCoreWords() {
    let model = ScanningModel(core: FakeCore(.fresh))
    #expect(model.words == FakeCore(.fresh).scanWords())
    #expect(model.words.looking == "Looking for a Shaman invite")
  }
}

@Suite struct QRCodeTests {
  /// An Invite as long as the largest a Serving Server is estimated to issue: four addresses and a Relay.
  private func largestInvite() throws -> String {
    let base64URL = { (data: Data) in
      data.base64EncodedString().replacingOccurrences(of: "+", with: "-").replacingOccurrences(of: "/", with: "_")
        .replacingOccurrences(of: "=", with: "")
    }
    let payload: [String: Any] = [
      "ways": [
        ["direct": "192.168.1.20:7791"], ["direct": "100.64.0.7:7791"], ["direct": "10.0.0.12:7791"],
        ["direct": "[fd7a:115c:a1e0::7]:7791"], ["relay": "relay.example.com"],
      ],
      "server_key": base64URL(Data(repeating: 7, count: 91)),
      "token": base64URL(Data(repeating: 1, count: 32)),
      "hostname": "workstation",
    ]
    let json = try JSONSerialization.data(withJSONObject: payload, options: [.sortedKeys])
    return "shaman-v1-" + base64URL(json)
  }

  /// Read back by Core Image's detector, as Vision's needs hardware the simulator lacks.
  @Test func anInviteDrawnAsAServerDrawsItReadsBackWhole() throws {
    let invite = try largestInvite()
    let image = try #require(QRCode.image(of: invite)?.cgImage)
    let detector = try #require(
      CIDetector(ofType: CIDetectorTypeQRCode, context: nil, options: [CIDetectorAccuracy: CIDetectorAccuracyHigh]))
    let read = detector.features(in: CIImage(cgImage: image)).compactMap { ($0 as? CIQRCodeFeature)?.messageString }
    #expect(read == [invite])
  }
}
