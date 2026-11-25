import ShamanCore
import SwiftUI
import Testing
import UIKit

@testable import ShamanUI

/// What each of a set of agent Messages' Markdown measures as Textual draws it, in a window of the app's own scene so
/// that it draws as it does on the phone: empty, then into place over a few passes.
@MainActor private final class Drawn {
  static let width: CGFloat = 370
  private(set) var heights: [String: CGFloat] = [:]

  init(_ markdowns: Set<String>, size: DynamicTypeSize = .large) async throws {
    let scene = try #require(UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }.first)
    let window = UIWindow(windowScene: scene)
    defer { window.isHidden = true }
    let measured = { [weak self] (markdown: String, height: CGFloat) -> Void in self?.heights[markdown] = height }
    let controller = UIHostingController(
      rootView: ScrollView {
        VStack(spacing: 0) {
          ForEach(markdowns.sorted(), id: \.self) { markdown in
            AgentBlockView(kind: .paragraph, markdown: markdown, model: nil)
              .fixedSize(horizontal: false, vertical: true)
              .onGeometryChange(for: CGFloat.self) {
                $0.size.height
              } action: { height in
                measured(markdown, height)
              }
          }
        }
        .frame(width: Self.width)
        .dynamicTypeSize(size)
      })
    window.rootViewController = controller
    window.windowLevel = .alert + 1
    window.makeKeyAndVisible()
    // Until everything has been drawn, and has measured the same for a while.
    let deadline = ContinuousClock.now + .seconds(60)
    var settled = (heights: heights, since: ContinuousClock.now)
    while ContinuousClock.now < deadline {
      try await Task.sleep(for: .milliseconds(50))
      if settled.heights != heights {
        settled = (heights, .now)
      }
      let drawn = heights.count == markdowns.count && heights.values.allSatisfy { $0 > 0 }
      if drawn, ContinuousClock.now - settled.since > .seconds(1) { break }
    }
  }
}

/// The gaps the conversation stands a Message's blocks apart by, which are Textual's own: drawn by blocks, a Message
/// stands as tall as Textual draws it whole.
@Suite(.serialized) @MainActor struct AgentBlockHostedTests {
  /// The rows of the Message whose Markdown is `markdown`, as the core cuts it.
  private static func rows(_ markdown: String) -> [(row: ConversationRow, markdown: String)] {
    agentMessageItems(message: "message", markdown: markdown, model: nil).compactMap { item in
      guard case .agentBlock(_, _, _, let markdown, _) = item.kind else { return nil }
      return (.item(item), markdown)
    }
  }

  /// Draws each of `messages` whole and by blocks, and expects the two as tall as each other.
  private static func expectAsTallByBlocks(_ messages: [String], size: DynamicTypeSize = .large) async throws {
    let blocks = Dictionary(messages.map { ($0, rows($0)) }, uniquingKeysWith: { first, _ in first })
    let drawn = try await Drawn(Set(messages + blocks.values.flatMap { $0.map(\.markdown) }), size: size)
    let em = UIFont.preferredFont(
      forTextStyle: .body,
      compatibleWith: UITraitCollection(preferredContentSizeCategory: UIContentSizeCategory(size))
    ).pointSize
    for message in messages {
      let rows = blocks[message] ?? []
      let whole = try #require(drawn.heights[message])
      var tall = try rows.map { try #require(drawn.heights[$0.markdown]) }.reduce(0, +)
      for (upper, lower) in zip(rows, rows.dropFirst()) {
        tall += try #require(lower.row.gap(below: upper.row), "two blocks of one Message") * em
      }
      #expect(rows.count > 1 && whole > 0)
      #expect(abs(whole - tall) < 0.5, "\(whole) whole and \(tall) by blocks:\n\(message)")
    }
  }

  /// One of each kind of block, and of each that a list or a quotation may hold.
  private static let parts = [
    "A paragraph of words, long enough that it wraps on to a second line of them.",
    "# A heading", "### A smaller one", "```\nlet a = 1\n```", "| a | b |\n|---|---|\n| 1 | 2 |", "***",
    "> quoted words", "> ```\n> quoted code\n> ```", "> # A quoted heading\n>\n> and words", "- one\n- two",
    "1. one\n\n2. two", "+ one\n\n  ```\n  code\n  ```", "* | a | b |\n  |---|---|\n  | 1 | 2 |",
    "1) # A heading\n\n   words", "<div>\nhi\n</div>",
  ]

  /// Each kind of block above each. Two lists that go on as one are left out, and HTML above HTML: Foundation reads the
  /// two as one, with a line between where a gap would be, and the core cuts them apart.
  @Test func aMessageIsAsTallByItsBlocksAsWholeWhicheverTwoStandTogether() async throws {
    let pairs = Self.parts.flatMap { upper in Self.parts.map { (upper, $0) } }
      .filter { !$0.hasPrefix("<") || !$1.hasPrefix("<") }
      .map { "\($0)\n\n\($1)" }
    let held = pairs.filter { Self.rows($0).count == 2 }
    #expect(held.count > 200, "\(held.count) of \(pairs.count)")
    try await Self.expectAsTallByBlocks(held)
  }

  @Test(arguments: [DynamicTypeSize.large, .xSmall, .accessibility2])
  func aMessageIsAsTallByItsBlocksAsWholeAtAnySizeOfText(size: DynamicTypeSize) async throws {
    try await Self.expectAsTallByBlocks(
      [
        Self.parts.joined(separator: "\n\n"), Self.parts.reversed().joined(separator: "\n\n"),
        "See [the docs][d].\n\n- and [again][d]\n\n[d]: https://d.example",
      ], size: size)
  }
}
