import SwiftUI
import Testing
import UIKit

@testable import ShamanUI

/// Pairing at the root of a phone holding no Remote, in a window of the app's own scene as large as a Duo's inner
/// display, in landscape or in portrait, folded as a test says: the fold is the test's own, handed to where pairing
/// reads it, as no test folds a Duo. Its scanner is the test's, which records what it is told.
@MainActor private final class HostedPairing {
  static let landscape = CGSize(width: 951, height: 669)
  static let portrait = CGSize(width: 669, height: 951)
  /// The band kept for the fold, partly folded like a book and sat like a laptop.
  static let book = BookFold(leading: 455.5, window: 951)
  static let laptop = Fold(top: 455.5, bottom: 495.5, window: 951)

  let window: UIWindow
  let pose = Pose()
  let scanner = RecordingScanner()

  /// Pairing at `path`, over the fake holding no Remote, in a window of `size`, at regular width.
  init(at path: [PairingRoute] = [], size: CGSize) async throws {
    let scene = try #require(UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }.first)
    window = UIWindow(windowScene: scene)
    window.traitOverrides.horizontalSizeClass = .regular
    window.frame = CGRect(origin: .zero, size: size)
    let screen = PairingScreen(core: FakeWindow(.fresh), path: path, pose: pose)
      .environment(\.codeScanner, scanner.scanner)
    window.rootViewController = UIHostingController(rootView: screen)
    window.windowLevel = .alert + 1
    window.makeKeyAndVisible()
    try await settle()
  }

  /// Folds it as the test says, the fold read once it is drawn, as it is read where pairing stands.
  func fold(book: BookFold? = nil, laptop: Fold? = nil) async throws {
    pose.book = book
    pose.fold = laptop
    try await settle()
  }

  /// Where each form on screen stands in the window: where pairing begins, or the confirmation.
  var forms: [CGRect] {
    Self.all(UICollectionView.self, in: window).filter { $0.window != nil && !$0.isHidden }
      .map { $0.convert($0.bounds, to: window) }
  }

  /// Where the scanner's picture stands in the window.
  var picture: CGRect? {
    scanner.picture.flatMap { $0.window == nil ? nil : $0.convert($0.bounds, to: window) }
  }

  func until(_ shown: () -> Bool, _ said: @autoclosure () -> String, sourceLocation: SourceLocation = #_sourceLocation)
    async throws
  {
    for _ in 0..<500 where !shown() {
      try await Task.sleep(for: .milliseconds(10))
    }
    #expect(shown(), "\(said())", sourceLocation: sourceLocation)
  }

  /// Lets whatever is animated end.
  func settle() async throws {
    try await Task.sleep(for: .milliseconds(500))
  }

  func close() {
    window.isHidden = true
  }

  private static func all<Kind: UIView>(_ kind: Kind.Type, in view: UIView) -> [Kind] {
    (view as? Kind).map { [$0] } ?? view.subviews.flatMap { all(kind, in: $0) }
  }
}

/// A scanner that reads nothing by itself and records what it is told: each time it is made, and whether it scans each
/// time it is drawn. What it hears is the test's to say.
@MainActor private final class RecordingScanner {
  private(set) var made = 0
  /// Whether it was to scan, each time it was drawn.
  private(set) var scanning: [Bool] = []
  /// Its picture, the view it was last made with.
  private(set) weak var picture: UIView?
  /// What it tells of the codes in front of it.
  private(set) var heard: (@MainActor (ScanEvent) -> Void)?

  var scanner: CodeScanner {
    CodeScanner(
      access: { .granted },
      view: { [self] scanning, heard in
        AnyView(Recording(recorder: self, scanning: scanning, heard: heard))
      })
  }

  private struct Recording: UIViewControllerRepresentable {
    let recorder: RecordingScanner
    let scanning: Bool
    let heard: @MainActor (ScanEvent) -> Void

    func makeUIViewController(context: Context) -> UIViewController {
      let made = UIViewController()
      made.view.backgroundColor = .black
      recorder.made += 1
      recorder.picture = made.view
      return made
    }

    func updateUIViewController(_ controller: UIViewController, context: Context) {
      recorder.scanning.append(scanning)
      recorder.heard = heard
    }
  }
}

@Suite(.serialized) @MainActor struct PairingFoldTests {
  /// Flat on the inner display in landscape, where pairing begins and the confirmation read at most 700 points across,
  /// in the middle of the window, and the scanner's picture is the whole window but the bar's.
  @Test(arguments: [[], [PairingRoute.confirm(FakeCore.invite)]])
  func flatTheFormsReadInAColumnAndTheScannerTakesTheWholeWindow(at path: [PairingRoute]) async throws {
    let hosted = try await HostedPairing(at: path, size: HostedPairing.landscape)
    defer { hosted.close() }
    try await hosted.until({ !hosted.forms.isEmpty }, "a form")
    for form in hosted.forms {
      #expect(form.width <= 700.5 && form.width > 600, "\(form)")
      #expect(abs(form.midX - HostedPairing.landscape.width / 2) <= 1, "in the middle: \(form)")
    }

    let scanning = try await HostedPairing(at: [.scan], size: HostedPairing.landscape)
    defer { scanning.close() }
    try await scanning.until({ scanning.picture != nil }, "the scanner")
    let picture = try #require(scanning.picture)
    #expect(picture.width > 900 && abs(picture.maxY - HostedPairing.landscape.height) < 0.5, "\(picture)")
  }

  /// Partly folded like a book, where pairing begins, the confirmation and the scanner stand in the leading half, the
  /// scanner's picture whole in it, and none of them past the fold.
  @Test(arguments: [[], [PairingRoute.confirm(FakeCore.invite)], [.scan]])
  func partlyFoldedLikeABookItStandsInTheLeadingHalf(at path: [PairingRoute]) async throws {
    let hosted = try await HostedPairing(at: path, size: HostedPairing.landscape)
    defer { hosted.close() }
    try await hosted.fold(book: HostedPairing.book)
    let fold = HostedPairing.book.leading
    if path == [.scan] {
      try await hosted.until({ hosted.picture.map { abs($0.maxX - fold) < 0.5 } == true }, "\(hosted.picture as Any)")
      let picture = try #require(hosted.picture)
      #expect(picture.minX < 0.5 && abs(picture.maxY - HostedPairing.landscape.height) < 0.5, "\(picture)")
    } else {
      try await hosted.until(
        { !hosted.forms.isEmpty && hosted.forms.allSatisfy { $0.maxX <= fold + 0.5 } }, "\(hosted.forms)")
      #expect(hosted.forms.allSatisfy { $0.minX > -0.5 && $0.width > 300 }, "\(hosted.forms)")
    }
    // And opened flat once more, across the window.
    try await hosted.fold()
    if path == [.scan] {
      try await hosted.until({ hosted.picture.map { $0.width > 900 } == true }, "\(hosted.picture as Any)")
    } else {
      try await hosted.until({ hosted.forms.allSatisfy { $0.maxX > fold + 100 } }, "\(hosted.forms)")
    }
  }

  /// Sat like a laptop, the scanner's picture stands whole in the upper half, ending at the fold, and the forms run
  /// through it to the window's foot, scrolling through the fold.
  @Test(arguments: [[], [PairingRoute.confirm(FakeCore.invite)], [.scan]])
  func satLikeALaptopTheScannerStandsAboveTheFoldAndTheFormsScrollThroughIt(at path: [PairingRoute]) async throws {
    let hosted = try await HostedPairing(at: path, size: HostedPairing.portrait)
    defer { hosted.close() }
    try await hosted.fold(laptop: HostedPairing.laptop)
    let fold = HostedPairing.laptop.top
    if path == [.scan] {
      try await hosted.until({ hosted.picture.map { abs($0.maxY - fold) < 0.5 } == true }, "\(hosted.picture as Any)")
      let picture = try #require(hosted.picture)
      #expect(picture.minY > 0 && abs(picture.width - HostedPairing.portrait.width) < 0.5, "\(picture)")
    } else {
      try await hosted.until({ !hosted.forms.isEmpty }, "a form")
      #expect(
        hosted.forms.allSatisfy { $0.minY < fold && $0.maxY > HostedPairing.laptop.bottom + 300 }, "\(hosted.forms)")
    }
  }

  /// A fold coming and going tells the scanner nothing: it is made once, never stops, and takes the Invite it then
  /// reads.
  @Test(arguments: [false, true]) func theScannerGoesOnScanningAsTheDeviceFoldsAndOpens(laptop: Bool) async throws {
    let hosted = try await HostedPairing(
      at: [.scan], size: laptop ? HostedPairing.portrait : HostedPairing.landscape)
    defer { hosted.close() }
    let scanner = hosted.scanner
    try await hosted.until({ scanner.scanning.last == true }, "it scans: \(scanner.scanning)")
    let picture = try #require(scanner.picture)
    if laptop {
      try await hosted.fold(laptop: HostedPairing.laptop)
    } else {
      try await hosted.fold(book: HostedPairing.book)
    }
    try await hosted.until({ hosted.picture?.maxY ?? 0 < 600 || hosted.picture?.maxX ?? 0 < 600 }, "folded")
    try await hosted.fold()
    try await hosted.until({ hosted.picture.map { $0.maxY > 600 && $0.maxX > 600 } == true }, "opened")
    #expect(scanner.made == 1 && scanner.picture === picture, "the same scanner, made once")
    #expect(scanner.scanning.allSatisfy { $0 }, "it never stopped: \(scanner.scanning)")

    scanner.heard?(.read(FakeCore.invite))
    try await hosted.until({ scanner.scanning.last == false }, "it took the Invite: \(scanner.scanning)")
  }
}
