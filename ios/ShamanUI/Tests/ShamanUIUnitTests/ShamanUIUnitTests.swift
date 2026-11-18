import Foundation
import ShamanCore
import Synchronization
import Testing

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
}

@Suite struct LiveCoreTests {
  @Test func aNewPhoneIsFirstOfferedAsIPhoneAndARefusalIsInTheCoresWords() async throws {
    let base = try scratch()
    defer { try? FileManager.default.removeItem(at: base) }
    let core = try LiveCore(directory: try CoreDirectory.prepare(in: base))
    var heard = core.thisDevice().makeAsyncIterator()
    #expect(await heard.next()?.name == "iPhone")

    await #expect(throws: ActError.Invalid(message: "A name needs something other than spaces")) {
      try await core.renameDevice("   ")
    }
    try await core.renameDevice("Pocket")
    #expect(await heard.next()?.name == "Pocket")
  }
}
