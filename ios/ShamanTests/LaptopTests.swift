import SwiftUI
import Testing
import UIKit

@testable import ShamanUI

/// The columns in a window as large as a Duo's inner display in portrait, sat like a laptop as a test says: the fold is
/// the test's own, handed down from where it is read, as no test folds a Duo.
@MainActor private struct Laptop {
  static let size = CGSize(width: 669, height: 951)
  static let fold = Fold(top: 455.5, bottom: 495.5, window: 951)

  let hosted: HostedColumns

  /// The columns over `open`, flat in portrait, with the Session on top read.
  init(_ open: [Pushed]) async throws {
    hosted = try await HostedColumns(open: open, wide: true, sessions: true)
    stack.pose.take(WindowFoot(safeArea: 34, keyboard: 0))
    try await portrait()
    let laptop = self
    try await until { laptop.model?.conversation.read == true && laptop.conversation != nil }
  }

  var stack: SessionStack { hosted.stack }
  var aside: Aside { hosted.stack.aside }

  /// The Session open on top.
  var model: SessionModel? { stack.openSession.map(stack.model(for:)) }

  /// Lays it flat, and turns the window to portrait or to landscape.
  func portrait() async throws {
    stack.pose.fold = nil
    hosted.size(width: Self.size.width, height: Self.size.height, regular: true)
    try await hosted.settle()
  }

  func landscape() async throws {
    stack.pose.fold = nil
    hosted.size(width: Self.size.height, height: Self.size.width, regular: true)
    try await hosted.settle()
  }

  func until(_ shown: () -> Bool, sourceLocation: SourceLocation = #_sourceLocation) async throws {
    try await hosted.until(
      shown,
      "below \(asideBar?.topItem?.title ?? "nothing"), of \(asideBar?.items?.count ?? 0), beside "
        + "\(beside?.topItem?.title ?? "nothing"), the conversation to \(conversationEnds ?? 0)",
      sourceLocation: sourceLocation)
  }

  /// Sits it like a laptop, or lays it flat.
  func sit(_ sat: Bool = true) async throws {
    stack.pose.fold = sat ? Self.fold : nil
    try await hosted.settle()
  }

  /// Has a keyboard come up over the half below the fold, or go. Over it the window's bottom safe area reaches past the
  /// band kept for the fold, of which the system counts as much the keyboard's as `counted`.
  func keyboard(_ up: Bool, counted: CGFloat = 495.3) async throws {
    stack.pose.take(WindowFoot(safeArea: up ? 495.3 : 34, keyboard: up ? counted : 0))
    try await hosted.settle()
  }

  /// The screen on top of the main view.
  var screen: UIView { hosted.main.topViewController!.view }

  /// Every view of `Kind` on that screen that is in the window, or in `view`.
  func all<Kind: UIView>(_ kind: Kind.Type, in view: UIView? = nil) -> [Kind] {
    let view = view ?? screen
    return (view as? Kind).map { $0.window == nil ? [] : [$0] } ?? view.subviews.flatMap { all(kind, in: $0) }
  }

  func frame(_ view: UIView) -> CGRect { view.convert(view.bounds, to: nil) }

  var conversation: FootCollectionView? { all(FootCollectionView.self).first }
  /// Where the conversation ends in the window, above what its foot holds.
  var conversationEnds: CGFloat? { conversation.map { frame($0).maxY - $0.adjustedContentInset.bottom } }
  var field: UITextView? { all(UITextView.self).first { $0.isEditable } }
  /// How far above the window's bottom edge the field's box, 40 points tall, stands.
  var fieldStands: CGFloat? { field.map { hosted.window.bounds.maxY - frame($0).midY - 20 } }
  /// The Aside's own bar on that screen, and its list.
  var asideBar: UINavigationBar? { all(UINavigationBar.self).first }
  var asideList: UICollectionView? { all(UICollectionView.self).first { !($0 is FootCollectionView) } }

  /// Its bar in the column beside the main view.
  var beside: UINavigationBar? {
    hosted.columns.viewController(for: .inspector).flatMap { all(UINavigationBar.self, in: $0.view).first }
  }

  /// What an Activity's whole, pushed in the Aside, is titled once it is read.
  func pushedTitle() async throws -> String {
    try await until { asideBar?.items?.count == 2 && asideBar?.topItem?.title?.isEmpty == false }
    return asideBar?.topItem?.title ?? ""
  }

  /// Waits until the Aside stands below the fold on the main view's screen, under its own bar reading `title`, with
  /// Back in it where it is `pushed`, over the field; and the conversation ends at the fold.
  func stands(
    at title: String, pushed: Bool = false, sourceLocation: SourceLocation = #_sourceLocation
  ) async throws {
    try await until(
      {
        asideBar?.topItem?.title == title && asideBar?.items?.count == (pushed ? 2 : 1)
          && (conversationEnds ?? .infinity) <= Self.fold.top + 0.5
      }, sourceLocation: sourceLocation)
    try await hosted.settle()
    let bar = try #require(asideBar, sourceLocation: sourceLocation)
    #expect(bar.topItem?.title == title, sourceLocation: sourceLocation)
    #expect(bar.items?.count == (pushed ? 2 : 1), sourceLocation: sourceLocation)
    #expect(frame(bar).minY >= Self.fold.bottom - 0.5, "\(frame(bar))", sourceLocation: sourceLocation)
    #expect(frame(bar).width == Self.size.width, "\(frame(bar))", sourceLocation: sourceLocation)
    let ends = try #require(conversationEnds, sourceLocation: sourceLocation)
    #expect(ends <= Self.fold.top + 0.5, sourceLocation: sourceLocation)
    if let field {
      #expect(abs((fieldStands ?? 0) - 22) < 0.5, "\(frame(field))", sourceLocation: sourceLocation)
      if let list = asideList {
        #expect(frame(list).maxY <= frame(field).midY - 20, "\(frame(list))", sourceLocation: sourceLocation)
      }
    }
    #expect(stack.asideIsOnScreen && !aside.standsBeside, sourceLocation: sourceLocation)
    #expect(hosted.columns.viewController(for: .inspector) == nil, sourceLocation: sourceLocation)
  }

  /// Waits until no Aside stands on the main view's screen.
  func standsNowhere(sourceLocation: SourceLocation = #_sourceLocation) async throws {
    try await until({ asideBar == nil && !stack.asideIsOnScreen }, sourceLocation: sourceLocation)
    try await hosted.settle()
    #expect(asideBar == nil && asideList == nil && !stack.asideIsOnScreen, sourceLocation: sourceLocation)
  }

  func close() { hosted.close() }
}

@Suite(.serialized) @MainActor struct LaptopHostedTests {
  private let settled = SessionRoute(session: "workstation-key/7")
  private let working = SessionRoute(session: "workstation-key/1")
  private let delegating = SessionRoute(session: "workstation-key/12")

  @Test func theConversationEndsAtTheFoldWithItsLinesAndTheAsideStandsBelowOverTheField() async throws {
    let laptop = try await Laptop([.session(settled)])
    defer { laptop.close() }
    try await laptop.sit()
    try await laptop.stands(at: "Worked for 3m 12s")
    // With no line at its foot, right at it.
    #expect(abs((laptop.conversationEnds ?? 0) - Laptop.fold.top) < 0.5, "\(laptop.conversationEnds ?? 0)")
    laptop.stack.open(working.session)
    try await laptop.stands(at: "Working")
    // And by as much above it as the bar is tall, with what it keeps clear above and below.
    let bar = Laptop.fold.top - (laptop.conversationEnds ?? 0)
    #expect(bar > 8 + 24 + 8 && bar < 70, "\(bar)")
    // With the keyboard up there is the field alone, under the bar, and no Aside.
    try await laptop.keyboard(true)
    try await laptop.standsNowhere()
    #expect(abs((laptop.fieldStands ?? 0) - 8) < 0.5)
    #expect((laptop.conversationEnds ?? 0) > 800)
    // It stands clear of the band, above the fold, though the keyboard that lifts it is counted less than the band.
    for counted: CGFloat in [455.3, 455.5, 495.3] {
      try await laptop.keyboard(true, counted: counted)
      let stands = counted + (laptop.fieldStands ?? 0)
      #expect(abs(stands - 495.3 - 8) < 0.5 && stands > Laptop.fold.below, "\(counted): \(stands)")
    }
    // Dragged down, the last of it, which is not up, has the Aside back with the lines still at the fold.
    laptop.stack.pose.take(WindowFoot(safeArea: 34, keyboard: 20))
    try await laptop.until { laptop.asideBar != nil }
    try await laptop.hosted.settle()
    let ends = Laptop.fold.top + 20 - (laptop.conversationEnds ?? 0)
    #expect(abs(ends - bar) < 0.5, "\(ends), and not \(bar)")
    try await laptop.keyboard(false)
    try await laptop.stands(at: "Working")
  }

  @Test func theFieldIsOneViewThatKeepsItsFocusAndWhatIsTypedAsTheSecondPartComesAndGoes() async throws {
    let laptop = try await Laptop([.session(settled)])
    defer { laptop.close() }
    let field = try #require(laptop.field)
    let model = try #require(laptop.model)
    #expect(field.becomeFirstResponder())
    var typed = ""
    for (sat, keyboard) in [
      (true, false), (true, true), (true, false), (true, true), (false, true), (true, true), (true, false),
      (false, false),
    ] {
      laptop.stack.pose.fold = sat ? Laptop.fold : nil
      try await laptop.keyboard(keyboard)
      #expect((laptop.asideBar != nil) == (sat && !keyboard))
      #expect(laptop.field === field)
      #expect(field.isFirstResponder)
      field.insertText("a")
      typed += "a"
      try await laptop.until { model.text == typed }
    }
    // And put down, it is not taken up again.
    field.resignFirstResponder()
    for sat in [true, false] {
      try await laptop.sit(sat)
      #expect(laptop.field === field && !field.isFirstResponder)
    }
  }

  @Test func leavingThePoseChangesNothing() async throws {
    let laptop = try await Laptop([.session(working)])
    defer { laptop.close() }
    let screens = laptop.hosted.main.viewControllers
    let conversation = try #require(laptop.conversation)
    let field = try #require(laptop.field)
    try await laptop.hosted.settle()
    let stood = [laptop.frame(conversation), laptop.frame(field)]
    let offset = conversation.contentOffset
    for shown in [false, true] {
      laptop.aside.shown = shown
      for _ in 0..<2 {
        try await laptop.sit()
        try await laptop.stands(at: "Working")
        try await laptop.sit(false)
        try await laptop.standsNowhere()
        #expect(laptop.hosted.main.viewControllers == screens)
        #expect(laptop.conversation === conversation && laptop.field === field)
        #expect([laptop.frame(conversation), laptop.frame(field)] == stood)
        #expect(abs(conversation.contentOffset.y - offset.y) < 0.5)
        #expect(laptop.aside.shown == shown && laptop.aside.kind == .work)
        #expect(laptop.stack.path == [.session(working)])
      }
    }
  }

  @Test func aConversationAtItsFootStaysThereIntoThePoseAndOutOfIt() async throws {
    let laptop = try await Laptop([.session(working)])
    defer { laptop.close() }
    let conversation = try #require(laptop.conversation)
    let follower = try #require(conversation.follower)
    for (sat, keyboard) in [(true, false), (true, true), (true, false), (false, false), (true, false), (false, false)] {
      laptop.stack.pose.fold = sat ? Laptop.fold : nil
      try await laptop.keyboard(keyboard)
      #expect(conversation.distanceToFoot < 1, "\(conversation.distanceToFoot)")
      #expect(follower.following)
    }
    // It has more than the half above the fold shows, so it did scroll there.
    try await laptop.sit()
    #expect(conversation.contentOffset.y > -conversation.adjustedContentInset.top + 1)
  }

  @Test func theAsideStandsBelowWheneverASessionIsOpenShownBesideOrNot() async throws {
    let laptop = try await Laptop([.session(settled)])
    defer { laptop.close() }
    for shown in [false, true] {
      laptop.aside.shown = shown
      laptop.stack.open(settled.session)
      try await laptop.sit()
      try await laptop.stands(at: "Worked for 3m 12s")
      laptop.stack.open(working.session)
      try await laptop.stands(at: "Working")
      // One with no Turn, whose work's bar says nothing.
      laptop.stack.open("workstation-key/11")
      try await laptop.stands(at: "")
      // A Subagent's that has ended, which has no field.
      laptop.stack.open("workstation-key/12.2")
      try await laptop.stands(at: "Worked for 41s")
      #expect(laptop.field == nil)
      // Beside the main view it stands as it was asked to.
      try await laptop.landscape()
      #expect((laptop.hosted.columns.viewController(for: .inspector) != nil) == shown)
      #expect(laptop.aside.shown == shown)
      try await laptop.portrait()
    }
    // While none is open there is none.
    laptop.stack.compose()
    try await laptop.sit()
    try await laptop.standsNowhere()
  }

  @Test func aCardAndTheTreeButtonTurnItAndNeitherPushNorPresent() async throws {
    let laptop = try await Laptop([.session(delegating)])
    defer { laptop.close() }
    try await laptop.sit()
    try await laptop.stands(at: "Working")
    let screens = laptop.hosted.main.viewControllers
    for _ in 0..<2 {
      // As a "Review" that leads to the tree does.
      #expect(laptop.stack.showTree(), "with no sheet")
      try await laptop.stands(at: "Subagents")
      laptop.stack.lead(to: .work(delegating, turn: "work"))
      try await laptop.stands(at: "Working")
      #expect(laptop.hosted.main.viewControllers == screens)
      #expect(laptop.hosted.columns.presentedViewController == nil)
      #expect(!laptop.aside.shown)
    }
    // The tree chosen in the pose is what stands beside the main view once it is asked for there.
    #expect(laptop.stack.showTree())
    try await laptop.landscape()
    #expect(laptop.hosted.columns.viewController(for: .inspector) == nil, "as it was not shown")
    laptop.aside.shown = true
    try await laptop.until { laptop.beside?.topItem?.title == "Subagents" }
  }

  @Test func anActivityPushedBelowTheFoldStandsPushedBesideTheMainViewAndTheOtherWayAbout() async throws {
    let laptop = try await Laptop([.session(settled)])
    defer { laptop.close() }
    laptop.aside.shown = true
    try await laptop.sit()
    try await laptop.stands(at: "Worked for 3m 12s")
    // The first row of the work, as a tap has it: handed no other way, it pushes within the Aside.
    let list = try #require(laptop.asideList)
    let screens = laptop.hosted.main.viewControllers
    list.delegate?.collectionView?(list, didSelectItemAt: IndexPath(item: 0, section: 0))
    try await laptop.until { !laptop.aside.place(of: settled).pushed.isEmpty }
    let pushed = laptop.aside.place(of: settled)
    let title = try await laptop.pushedTitle()
    try await laptop.stands(at: title, pushed: true)
    #expect(laptop.hosted.main.viewControllers == screens)
    #expect(laptop.stack.path == [.session(settled)])
    // Through a keyboard, which takes it down.
    try await laptop.keyboard(true)
    try await laptop.standsNowhere()
    try await laptop.keyboard(false)
    try await laptop.stands(at: title, pushed: true)
    // Flat and in landscape, in its column, and back below.
    for _ in 0..<2 {
      try await laptop.landscape()
      try await laptop.until { laptop.beside?.topItem?.title == title && laptop.beside?.items?.count == 2 }
      #expect(laptop.aside.standsBeside && laptop.asideBar == nil)
      #expect(laptop.aside.place(of: settled) == pushed)
      try await laptop.portrait()
      try await laptop.standsNowhere()
      try await laptop.sit()
      try await laptop.stands(at: title, pushed: true)
    }
    // Popped in its column, it is popped below.
    try await laptop.landscape()
    try await laptop.until { laptop.beside?.items?.count == 2 }
    laptop.aside.push([], latest: laptop.model?.conversation.latestTurn, in: settled)
    try await laptop.until { laptop.beside?.items?.count == 1 }
    try await laptop.portrait()
    try await laptop.sit()
    try await laptop.stands(at: "Worked for 3m 12s")
    #expect(laptop.aside.shown)
  }

  @Test func itFollowsTheLatestTurnBelowTheFoldAndBackReturnsAParentsPlace() async throws {
    let laptop = try await Laptop([.session(settled)])
    defer { laptop.close() }
    try await laptop.sit()
    try await laptop.stands(at: "Worked for 3m 12s")
    let model = try #require(laptop.model)
    model.text = "Go on"
    await model.send()
    try await laptop.until { model.conversation.latestTurn == "turn-sent-1" }
    try await laptop.stands(at: "Working")
    await model.stop()
    model.text = "And again"
    await model.send()
    try await laptop.until { model.conversation.latestTurn == "turn-sent-2" }
    try await laptop.stands(at: "Working")
    laptop.stack.lead(to: .work(settled, turn: "work"))
    try await laptop.stands(at: "Worked for 3m 12s")
    // Another Session's screen over it shows that Session's; popped, the first's is as it was left.
    laptop.stack.push(.session(delegating))
    try await laptop.stands(at: "Working")
    laptop.hosted.main.popViewController(animated: true)
    try await laptop.until { laptop.stack.path == [.session(settled)] }
    try await laptop.stands(at: "Worked for 3m 12s")
    #expect(laptop.aside.place(of: settled) == Aside.Place(held: "work"))
    // And it reads on: what it is turned to, and a Turn begun while its screen was beneath another.
    laptop.stack.lead(to: .work(settled, turn: "turn-sent-2"))
    try await laptop.stands(at: "Working")
    laptop.stack.push(.session(delegating))
    try await laptop.stands(at: "Working")
    await model.stop()
    model.text = "And once more"
    await model.send()
    laptop.hosted.main.popViewController(animated: true)
    try await laptop.until { model.conversation.latestTurn == "turn-sent-3" }
    try await laptop.stands(at: "Working")
    #expect(laptop.stack.turnInAside(of: model) == "turn-sent-3")
  }
}
