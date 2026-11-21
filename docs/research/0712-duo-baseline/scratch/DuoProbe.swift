// SCRATCH for #712. Never commit: it writes what the root of each scene measures to the app's temporary directory.
#if DEBUG
  import SwiftUI
  import UIKit

  @available(iOS 27.1, *)
  struct DuoProbe: View {
    @State private var id = String(UUID().uuidString.prefix(4))
    @State private var hinge = "unread"
    @State private var hingeRuns = 0
    @Environment(\.horizontalSizeClass) private var horizontal
    @Environment(\.verticalSizeClass) private var vertical
    @Environment(\.scenePhase) private var phase
    @Environment(\.supportsMultipleWindows) private var multipleWindows
    @Environment(\.toolbarVerticalEdge) private var barEdge

    var body: some View {
      TimelineView(.periodic(from: .now, by: 1)) { _ in
        GeometryReader { proxy in
          let line = describe(proxy)
          Color.clear.onChange(of: line, initial: true) { _, new in DuoProbe.write(id, new) }
        }
      }
      .onHingeChange { _, new in
        hingeRuns += 1
        if let h = new.hinge {
          hinge = "\(h.status) \(Int(h.angle.degrees.rounded()))"
        } else {
          hinge = "none"
        }
      }
      .allowsHitTesting(false)
      .accessibilityHidden(true)
      .task {
        guard ProcessInfo.processInfo.arguments.contains("--second-window"), !DuoProbe.asked else { return }
        DuoProbe.asked = true
        try? await Task.sleep(for: .seconds(4))
        DuoProbe.write("ask", "second window asked")
        UIApplication.shared.activateSceneSession(for: UISceneSessionActivationRequest(role: .windowApplication)) {
          error in
          let said = "second window refused: \(error)"
          Task { @MainActor in DuoProbe.write("ask", said) }
        }
      }
    }

    private static var asked = false

    private func describe(_ proxy: GeometryProxy) -> String {
      let insets = proxy.safeAreaInsets
      var parts: [String] = []
      parts.append("scene=\(id)")
      parts.append("size=\(Int(proxy.size.width))x\(Int(proxy.size.height))")
      let global = proxy.frame(in: .global)
      parts.append("global=\(Int(global.minX)),\(Int(global.minY)),\(Int(global.width)),\(Int(global.height))")
      parts.append(
        "insets(t,l,b,r)=\(Int(insets.top)),\(Int(insets.leading)),\(Int(insets.bottom)),\(Int(insets.trailing))")
      parts.append("h=\(horizontal.map { "\($0)" } ?? "nil") v=\(vertical.map { "\($0)" } ?? "nil")")
      parts.append("phase=\(phase)")
      parts.append("multipleWindows=\(multipleWindows)")
      parts.append("barEdge=\(barEdge.map { "\($0)" } ?? "nil")")
      parts.append("hinge=\(hinge) runs=\(hingeRuns)")
      for (name, kind) in [("division", ReservedRegion.Kind.division), ("occlusion", ReservedRegion.Kind.occlusion)] {
        let regions = proxy.reservedRegions(kind: kind, options: .includeInactive)
        let text = regions.map { r in
          "[\(Int(r.frame.minX)),\(Int(r.frame.minY)),\(Int(r.frame.width)),\(Int(r.frame.height)) active=\(r.isActive) margins=\(Int(r.margins.top)),\(Int(r.margins.leading)),\(Int(r.margins.bottom)),\(Int(r.margins.trailing))]"
        }.joined()
        parts.append("\(name)=\(text.isEmpty ? "none" : text)")
      }
      parts.append("appMultipleScenes=\(UIApplication.shared.supportsMultipleScenes)")
      parts.append("deviceOrientation=\(UIDevice.current.orientation.rawValue)")
      let scenes = UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }
      parts.append("scenes=\(scenes.count)")
      for scene in scenes {
        var s = "{state=\(scene.activationState.rawValue) orientation=\(scene.effectiveGeometry.interfaceOrientation.rawValue)"
        if let window = scene.windows.first {
          let b = window.bounds
          let i = window.safeAreaInsets
          let m = window.layoutMargins
          let t = window.traitCollection
          s += " window=\(Int(b.width))x\(Int(b.height))"
          s += " safe(t,l,b,r)=\(Int(i.top)),\(Int(i.left)),\(Int(i.bottom)),\(Int(i.right))"
          s += " margins(t,l,b,r)=\(Int(m.top)),\(Int(m.left)),\(Int(m.bottom)),\(Int(m.right))"
          s += " traits=\(t.horizontalSizeClass.rawValue)/\(t.verticalSizeClass.rawValue) idiom=\(t.userInterfaceIdiom.rawValue)"
        }
        parts.append(s + "}")
      }
      return parts.joined(separator: " | ")
    }

    private static func write(_ id: String, _ line: String) {
      let directory = URL(fileURLWithPath: "/tmp/duo712/probe")
      try? line.write(to: directory.appendingPathComponent("duo-probe-\(id).txt"), atomically: true, encoding: .utf8)
      let log = directory.appendingPathComponent("duo-probe.log")
      let stamped = "\(Date().timeIntervalSince1970) \(line)\n"
      if let handle = try? FileHandle(forWritingTo: log) {
        handle.seekToEndOfFile()
        handle.write(Data(stamped.utf8))
        try? handle.close()
      } else {
        try? stamped.write(to: log, atomically: true, encoding: .utf8)
      }
    }
  }
#endif
