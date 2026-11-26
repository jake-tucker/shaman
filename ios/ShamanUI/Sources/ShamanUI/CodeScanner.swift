import AVFoundation
import ShamanCore
import SwiftUI
import VisionKit

/// Whether the scanner can read codes, as pressing Scan finds.
public enum ScanAccess: Sendable, Equatable {
  case granted
  /// Shaman may not use the camera, as its user chose in the Settings app or a restriction says.
  case refused
  /// This phone has no camera that scans, as the simulator has none.
  case unavailable
}

/// What the scanner tells of the codes in front of it.
public enum ScanEvent: Sendable, Equatable {
  /// A code came into sight, with its text.
  case read(String)
  /// The code with this text left the frame.
  case left(String)
  /// The camera stopped and will scan no more, as when it failed to start or became unavailable.
  case failed
}

/// What reads QR codes for pairing, behind a seam that yields each code's text: the camera, through VisionKit's data
/// scanner, or in a debug build a fake that reads the codes it is given. The screens find it in the environment, the
/// camera unless the app says otherwise.
public struct CodeScanner: Sendable {
  /// Finds whether codes can be scanned, asking for the camera where its user has not been asked yet. Only pressing
  /// Scan calls it, so the camera is never asked for at launch.
  let access: @MainActor @Sendable () async -> ScanAccess
  /// What scans from when `scanning` turns true until it turns false or the view goes, telling `heard` what it sees.
  let view: @MainActor @Sendable (_ scanning: Bool, _ heard: @escaping @MainActor (ScanEvent) -> Void) -> AnyView

  /// A scanner that finds access by `access` and scans in `view`.
  public init(
    access: @escaping @MainActor @Sendable () async -> ScanAccess,
    view:
      @escaping @MainActor @Sendable (_ scanning: Bool, _ heard: @escaping @MainActor (ScanEvent) -> Void) ->
      AnyView
  ) {
    self.access = access
    self.view = view
  }

  /// The camera, through VisionKit's data scanner, reading QR codes alone.
  public static let camera = CodeScanner(
    access: {
      // A phone with no camera that scans is not asked for one.
      guard DataScannerViewController.isSupported else { return .unavailable }
      switch AVCaptureDevice.authorizationStatus(for: .video) {
      case .authorized: break
      case .notDetermined:
        guard await AVCaptureDevice.requestAccess(for: .video) else { return .refused }
      default: return .refused
      }
      return DataScannerViewController.isAvailable ? .granted : .refused
    },
    view: { scanning, heard in AnyView(CameraScanner(scanning: scanning, heard: heard)) }
  )
}

extension EnvironmentValues {
  /// What reads QR codes for pairing.
  @Entry public var codeScanner = CodeScanner.camera
}

/// VisionKit's data scanner, reading QR codes while `scanning` and telling `heard` of each that comes and goes, and of
/// the camera failing.
private struct CameraScanner: UIViewControllerRepresentable {
  let scanning: Bool
  let heard: @MainActor (ScanEvent) -> Void

  func makeCoordinator() -> Coordinator {
    Coordinator(heard: heard)
  }

  func makeUIViewController(context: Context) -> DataScannerViewController {
    let scanner = DataScannerViewController(
      recognizedDataTypes: [.barcode(symbologies: [.qr])], qualityLevel: .balanced, recognizesMultipleItems: false,
      isHighFrameRateTrackingEnabled: false, isPinchToZoomEnabled: true, isGuidanceEnabled: false,
      isHighlightingEnabled: true)
    scanner.delegate = context.coordinator
    return scanner
  }

  func updateUIViewController(_ scanner: DataScannerViewController, context: Context) {
    context.coordinator.heard = heard
    if scanning {
      guard !scanner.isScanning else { return }
      do {
        try scanner.startScanning()
      } catch {
        heard(.failed)
      }
    } else if scanner.isScanning {
      scanner.stopScanning()
    }
  }

  static func dismantleUIViewController(_ scanner: DataScannerViewController, coordinator: Coordinator) {
    scanner.stopScanning()
  }

  /// Hears the codes the data scanner reads and loses, and its camera failing.
  final class Coordinator: NSObject, DataScannerViewControllerDelegate {
    var heard: @MainActor (ScanEvent) -> Void

    init(heard: @escaping @MainActor (ScanEvent) -> Void) {
      self.heard = heard
    }

    func dataScanner(
      _ dataScanner: DataScannerViewController, didAdd addedItems: [RecognizedItem], allItems: [RecognizedItem]
    ) {
      for text in addedItems.compactMap(Self.text) {
        heard(.read(text))
      }
    }

    func dataScanner(
      _ dataScanner: DataScannerViewController, didRemove removedItems: [RecognizedItem], allItems: [RecognizedItem]
    ) {
      for text in removedItems.compactMap(Self.text) {
        heard(.left(text))
      }
    }

    func dataScanner(
      _ dataScanner: DataScannerViewController,
      becameUnavailableWithError error: DataScannerViewController.ScanningUnavailable
    ) {
      heard(.failed)
    }

    /// The text of a QR code the scanner recognised.
    private static func text(of item: RecognizedItem) -> String? {
      guard case .barcode(let code) = item else { return nil }
      return code.payloadStringValue
    }
  }
}

/// The scanner's screen: what it scans, and the line saying it is looking for a Shaman invite. It scans only while
/// `model` says it stands at the top of its stack, which the stack it is pushed on tells the model; the first code that
/// is an Invite stops it and is handed on, and anything else is passed over while it keeps scanning. A camera that
/// fails says so, offering Paste.
struct ScanScreen: View {
  let model: ScanningModel
  /// Takes the Invite read, or pasted once the camera failed.
  let found: (String) -> Void
  @Environment(\.codeScanner) private var scanner

  var body: some View {
    scanner.view(model.scanning) { event in
      if let invite = model.heard(event) {
        found(invite)
      }
    }
    .ignoresSafeArea(edges: .bottom)
    .overlay(alignment: .bottom) {
      VStack(spacing: 8) {
        if model.failed {
          Text(model.words.cameraFailed)
            .font(.headline)
            .multilineTextAlignment(.center)
            .accessibilityIdentifier("scan.failed")
          PasteButton(payloadType: String.self) { strings in
            guard let invite = strings.first else { return }
            Task { @MainActor in found(invite) }
          }
          .accessibilityIdentifier("scan.paste")
        } else {
          Text(model.words.looking)
            .font(.headline)
            .accessibilityIdentifier("scan.looking")
          if model.passedOver != nil {
            Text(model.words.passedOver)
              .font(.footnote)
              .foregroundStyle(.secondary)
              .accessibilityIdentifier("scan.passedOver")
          }
        }
      }
      .padding(.horizontal, 20)
      .padding(.vertical, 12)
      .background(.regularMaterial, in: .rect(cornerRadius: 24))
      .padding(.horizontal, 16)
      .padding(.bottom, 32)
    }
    .navigationTitle("Scan")
    .navigationBarTitleDisplayMode(.inline)
    // Appearing does not resume it: a back swipe begun over it and given up shows it for a moment while the
    // confirmation stays pushed. Only its stack's path coming back to it does.
    .onDisappear { model.stands(atTop: false) }
  }
}

/// What the scanner's screen holds, kept by the stack it is pushed on, which says whenever its path changes whether the
/// scanner stands at the top: whether it scans, the code it passed over, and the Invite it took last, which it passes
/// over once returned to until that code leaves the frame.
@MainActor @Observable
final class ScanningModel {
  private let core: any WindowCore
  /// What the screen says, as the core words it.
  let words: ScanWords
  /// Whether the screen stands at the top of its stack and on screen.
  private var atTop = false
  /// The Invite taken while the screen stood at the top, which stops the scanner until it is returned to.
  private var taken: String?
  /// The Invite taken before the screen was returned to, passed over while it stays in sight, as a dead Invite does
  /// before "Scan again".
  private var ignoring: String?
  /// The code last passed over, not being an Invite, while it stays in sight.
  private(set) var passedOver: String?
  /// Whether the camera failed, after which it scans no more.
  private(set) var failed = false

  init(core: any WindowCore) {
    self.core = core
    words = core.device.scanWords()
  }

  /// Whether the scanner scans: while the screen stands at the top, until it takes an Invite or the camera fails.
  var scanning: Bool {
    atTop && taken == nil && !failed
  }

  /// Says whether the screen stands at the top of its stack and on screen. Returned to, it scans afresh, passing over
  /// the Invite it took last while that code stays in sight.
  func stands(atTop: Bool) {
    if atTop && !self.atTop {
      if let taken {
        ignoring = taken
      }
      taken = nil
      passedOver = nil
    }
    self.atTop = atTop
  }

  /// Takes what the scanner told: the Invite a code read is, which stops the scanner, or nothing where it read no
  /// Invite, which is passed over, or the Invite it took last, still in sight, or it has stopped already.
  func heard(_ event: ScanEvent) -> String? {
    switch event {
    case .left(let text):
      if text == ignoring {
        ignoring = nil
      }
      if text == passedOver {
        passedOver = nil
      }
      return nil
    case .failed:
      failed = true
      return nil
    case .read(let text):
      guard scanning, text != ignoring else { return nil }
      guard core.device.isInvite(text) else {
        passedOver = text
        return nil
      }
      taken = text
      passedOver = nil
      return text
    }
  }
}

/// The offer to scan an Invite, where pairing begins: what pressing Scan last found.
@MainActor @Observable
final class ScanOffer {
  /// What pressing Scan last found, or nothing where it has not been pressed.
  private(set) var access: ScanAccess?
  /// Whether Scan is finding access, as while the camera is asked for.
  private(set) var asking = false

  /// Finds whether `scanner` can scan, asking for the camera where it has not been asked yet: answers whether the
  /// scanner may open.
  func scan(with scanner: CodeScanner) async -> Bool {
    asking = true
    defer { asking = false }
    let access = await scanner.access()
    self.access = access
    return access == .granted
  }
}

#if DEBUG
  extension CodeScanner {
    /// A scanner that finds access as `access` says, and, granted, scans as the camera does, from when it is started
    /// until it is stopped, whether or not it is on screen: it sees again the code last in front of it, then holds
    /// each of `codes` in front of it in turn, a `moment` apart, the one before leaving the frame. Each is held up once
    /// over the scanner's life. Where it is `failing`, the camera fails as it starts.
    public static func reading(
      _ codes: [String], access: ScanAccess = .granted, every moment: Duration = .seconds(1), failing: Bool = false
    ) -> CodeScanner {
      let queue = FakeCodes(codes)
      return CodeScanner(
        access: { access },
        view: { scanning, heard in
          AnyView(FakeScanner(codes: queue, moment: moment, failing: failing, scanning: scanning, heard: heard))
        })
    }
  }

  /// The codes a fake scanner has yet to hold up.
  @MainActor
  private final class FakeCodes: Sendable {
    private var codes: [String]

    nonisolated init(_ codes: [String]) {
      self.codes = codes
    }

    func next() -> String? {
      codes.isEmpty ? nil : codes.removeFirst()
    }
  }

  /// The fake scanner, started and stopped as the camera's data scanner is, drawing the code in front of it as the QR
  /// code a camera would see.
  private struct FakeScanner: UIViewControllerRepresentable {
    let codes: FakeCodes
    let moment: Duration
    let failing: Bool
    let scanning: Bool
    let heard: @MainActor (ScanEvent) -> Void

    func makeUIViewController(context: Context) -> FakeScannerController {
      FakeScannerController(codes: codes, moment: moment, failing: failing)
    }

    func updateUIViewController(_ scanner: FakeScannerController, context: Context) {
      scanner.heard = heard
      if scanning {
        scanner.startScanning()
      } else {
        scanner.stopScanning()
      }
    }

    static func dismantleUIViewController(_ scanner: FakeScannerController, coordinator: ()) {
      scanner.stopScanning()
    }
  }

  /// What the fake scanner runs on: a task reading codes from when it starts until it stops.
  private final class FakeScannerController: UIViewController {
    private let codes: FakeCodes
    private let moment: Duration
    private let failing: Bool
    var heard: @MainActor (ScanEvent) -> Void = { _ in }
    private var scanning: Task<Void, Never>?
    private let drawn = UIImageView()
    /// The code in front of the scanner, which it sees again as it starts.
    private var shown: String? {
      didSet { drawn.image = shown.flatMap { QRCode.image(of: $0) } }
    }

    init(codes: FakeCodes, moment: Duration, failing: Bool) {
      self.codes = codes
      self.moment = moment
      self.failing = failing
      super.init(nibName: nil, bundle: nil)
    }

    required init?(coder: NSCoder) {
      fatalError("not drawn from a storyboard")
    }

    override func viewDidLoad() {
      super.viewDidLoad()
      view.backgroundColor = .black
      drawn.contentMode = .scaleAspectFit
      drawn.layer.magnificationFilter = .nearest
      drawn.frame = view.bounds.insetBy(dx: 60, dy: 60)
      drawn.autoresizingMask = [.flexibleWidth, .flexibleHeight]
      view.addSubview(drawn)
    }

    func startScanning() {
      guard scanning == nil else { return }
      if failing {
        heard(.failed)
        return
      }
      scanning = Task { [weak self, moment] in
        if let shown = self?.shown {
          try? await Task.sleep(for: moment)
          guard !Task.isCancelled, let self else { return }
          heard(.read(shown))
        }
        while true {
          try? await Task.sleep(for: moment)
          guard !Task.isCancelled, let self, let next = codes.next() else { return }
          if let shown {
            heard(.left(shown))
          }
          shown = next
          heard(.read(next))
        }
      }
    }

    func stopScanning() {
      scanning?.cancel()
      scanning = nil
    }
  }
#endif
