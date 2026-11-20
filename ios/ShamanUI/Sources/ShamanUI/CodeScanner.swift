import AVFoundation
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

/// What reads QR codes for pairing, behind a seam that yields each code's text: the camera, through VisionKit's data
/// scanner, or in a debug build a fake that reads the codes it is given. The screens find it in the environment, the
/// camera unless the app says otherwise.
public struct CodeScanner: Sendable {
  /// Finds whether codes can be scanned, asking for the camera where its user has not been asked yet. Only pressing
  /// Scan calls it, so the camera is never asked for at launch.
  let access: @MainActor @Sendable () async -> ScanAccess
  /// What scans while it is up and `scanning`, telling `read` the text of each code it reads.
  let view: @MainActor @Sendable (_ scanning: Bool, _ read: @escaping @MainActor (String) -> Void) -> AnyView

  /// A scanner that finds access by `access` and scans in `view`.
  public init(
    access: @escaping @MainActor @Sendable () async -> ScanAccess,
    view: @escaping @MainActor @Sendable (_ scanning: Bool, _ read: @escaping @MainActor (String) -> Void) -> AnyView
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
    view: { scanning, read in AnyView(CameraScanner(scanning: scanning, read: read)) }
  )
}

extension EnvironmentValues {
  /// What reads QR codes for pairing.
  @Entry public var codeScanner = CodeScanner.camera
}

/// VisionKit's data scanner, reading QR codes while `scanning` and telling `read` the text of each new one.
private struct CameraScanner: UIViewControllerRepresentable {
  let scanning: Bool
  let read: @MainActor (String) -> Void

  func makeCoordinator() -> Coordinator {
    Coordinator(read: read)
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
    context.coordinator.read = read
    if scanning {
      if !scanner.isScanning {
        try? scanner.startScanning()
      }
    } else if scanner.isScanning {
      scanner.stopScanning()
    }
  }

  static func dismantleUIViewController(_ scanner: DataScannerViewController, coordinator: Coordinator) {
    scanner.stopScanning()
  }

  /// Hears the codes the data scanner reads.
  final class Coordinator: NSObject, DataScannerViewControllerDelegate {
    var read: @MainActor (String) -> Void

    init(read: @escaping @MainActor (String) -> Void) {
      self.read = read
    }

    func dataScanner(
      _ dataScanner: DataScannerViewController, didAdd addedItems: [RecognizedItem], allItems: [RecognizedItem]
    ) {
      for item in addedItems {
        if case .barcode(let code) = item, let text = code.payloadStringValue {
          read(text)
        }
      }
    }
  }
}

/// The scanner's screen: what it scans, and the line saying it is looking for a Shaman invite. The first code that is
/// an Invite stops it and is handed on; anything else is passed over, and it keeps scanning.
struct ScanScreen: View {
  @State private var model: ScanningModel
  @Environment(\.codeScanner) private var scanner
  /// Takes the Invite read.
  private let found: (String) -> Void

  init(core: any Core, found: @escaping (String) -> Void) {
    _model = State(initialValue: ScanningModel(core: core))
    self.found = found
  }

  var body: some View {
    scanner.view(model.scanning) { text in
      if let invite = model.read(text) {
        found(invite)
      }
    }
    .ignoresSafeArea(edges: .bottom)
    .overlay(alignment: .bottom) {
      VStack(spacing: 4) {
        Text("Looking for a Shaman invite")
          .font(.headline)
          .accessibilityIdentifier("scan.looking")
        if model.passedOver {
          Text("That code is not a Shaman invite")
            .font(.footnote)
            .foregroundStyle(.secondary)
            .accessibilityIdentifier("scan.passedOver")
        }
      }
      .padding(.horizontal, 20)
      .padding(.vertical, 12)
      .background(.regularMaterial, in: .capsule)
      .padding(.bottom, 32)
    }
    .navigationTitle("Scan")
    .navigationBarTitleDisplayMode(.inline)
    // Returned to, as "Scan again" returns, it scans afresh.
    .onAppear { model.resume() }
  }
}

/// What the scanner's screen holds while it is up: whether it scans, and whether it passed a code over.
@MainActor @Observable
final class ScanningModel {
  private let core: any Core
  /// Whether the scanner scans: until it reads an Invite, and again once returned to.
  private(set) var scanning = true
  /// Whether the last code read was passed over, not being an Invite.
  private(set) var passedOver = false

  init(core: any Core) {
    self.core = core
  }

  /// Takes the text of a code the scanner read: the Invite it is, which stops the scanner, or nothing where it is no
  /// Invite, which is passed over, or where the scanner has stopped already.
  func read(_ text: String) -> String? {
    guard scanning else { return nil }
    guard core.isInvite(text) else {
      passedOver = true
      return nil
    }
    scanning = false
    passedOver = false
    return text
  }

  /// Scans again, as when the screen is returned to.
  func resume() {
    scanning = true
    passedOver = false
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
    /// A scanner that finds access as `access` says, and, granted, reads `codes` in turn, a `moment` apart while it
    /// scans, as a camera held over each would. Each is read once over the scanner's life, so a scanner returned to
    /// goes on to the next.
    public static func reading(
      _ codes: [String], access: ScanAccess = .granted, every moment: Duration = .seconds(1)
    ) -> CodeScanner {
      let queue = FakeCodes(codes)
      return CodeScanner(
        access: { access },
        view: { scanning, read in
          AnyView(FakeScanner(codes: queue, moment: moment, scanning: scanning, read: read))
        })
    }
  }

  /// The codes a fake scanner has yet to read.
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

  /// What the fake scanner shows: the code last read, drawn as the QR code a camera would see.
  private struct FakeScanner: View {
    let codes: FakeCodes
    let moment: Duration
    let scanning: Bool
    let read: @MainActor (String) -> Void
    @State private var shown: String?

    var body: some View {
      ZStack {
        Color.black
        if let shown, let code = QRCode.image(of: shown) {
          Image(uiImage: code)
            .interpolation(.none)
            .resizable()
            .scaledToFit()
            .frame(width: 240)
        }
      }
      .task(id: scanning) {
        guard scanning else { return }
        while true {
          try? await Task.sleep(for: moment)
          guard !Task.isCancelled, let code = codes.next() else { return }
          shown = code
          read(code)
        }
      }
    }
  }
#endif
