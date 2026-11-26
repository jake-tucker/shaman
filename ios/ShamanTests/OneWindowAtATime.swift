import Testing

/// Has each test of a suite wait until no test of another suite carrying it is under way, as `.serialized` does only
/// within one suite.
///
/// Every test that stands a window in the app's scene carries it. Run in parallel, as the Pro's row runs the hosted tests,
/// Swift Testing runs suites side by side in the one app, and a window one test makes key takes the keyboard from
/// another's field, or a fold read across both.
struct OneWindowAtATime: SuiteTrait, TestTrait, TestScoping {
  var isRecursive: Bool { true }

  /// Each case of a test, and never a suite whole or a test whose cases are each held.
  func scopeProvider(for test: Test, testCase: Test.Case?) -> Self? {
    testCase == nil ? nil : self
  }

  func provideScope(
    for test: Test, testCase: Test.Case?, performing function: @Sendable () async throws -> Void
  ) async throws {
    await Turns.shared.take()
    do {
      try await function()
    } catch {
      await Turns.shared.give()
      throw error
    }
    await Turns.shared.give()
  }
}

extension Trait where Self == OneWindowAtATime {
  /// Each test waits until no test of another suite with a window of its own is under way.
  static var oneWindowAtATime: Self { Self() }
}

/// Whose turn it is to stand a window, taken in the order asked.
private actor Turns {
  static let shared = Turns()
  private var taken = false
  private var waiting: [CheckedContinuation<Void, Never>] = []

  func take() async {
    guard taken else {
      taken = true
      return
    }
    await withCheckedContinuation { waiting.append($0) }
  }

  /// Hands the turn to the next waiting, who holds it from there.
  func give() {
    if waiting.isEmpty {
      taken = false
    } else {
      waiting.removeFirst().resume()
    }
  }
}
