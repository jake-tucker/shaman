// PROTOTYPE for #729, "What the build asks to tell a Duo's foot from an ordinary iPhone's". Throwaway: never merge.
//
// A probe over #725's lab (`--lab uikit --lab-aside inspector`): readers of the vertical bar's edge, the window, the
// safe area and the reserved regions at the app's root and inside each column and a sheet, and a screen in the main
// view's column whose foot stands on the bottom edge one of several ways.
//
//   --foot <variant>   how the foot stands: band, a-content, a-inner, a-outer, b-negpad, c-bar, c-bar-ignore,
//                      c-additional, c-safepad, follow, follow-b, follow-additional
//   --foot-bar-off     asks for the vertical bar to be turned off, everywhere it can be asked
//   --foot-log <name>  the log is /tmp/duo729/<name>.log, and commands are read from /tmp/duo729/<name>.cmd
//
// Commands, one a line: `dump <label>`, `open`, `aside`, `sheet medium|large|off`, `focus`, `blur`, `end`,
// `variant <variant>`, `bar on|off`.

#if DEBUG
  import SwiftUI
  import UIKit

  enum Foot {
    static let directory = "/tmp/duo729"
    static var logName: String { Lab.value("--foot-log") ?? "app" }
    static var on: Bool { Lab.value("--foot") != nil }
    static let dump = Notification.Name("foot.dump")
    static let end = Notification.Name("foot.end")
  }

  @MainActor @Observable
  final class FootBus {
    static let shared = FootBus()
    var variant = Lab.value("--foot") ?? "band"
    var barOff = Lab.has("--foot-bar-off")
    var sheet: String?
    var focus = false
    var dumps = 0
    var windowBottom: CGFloat = 0
    @ObservationIgnored var label = ""
    @ObservationIgnored var frame = 0
    @ObservationIgnored var hinge = "never called"
    @ObservationIgnored var hingeRuns = 0
    @ObservationIgnored weak var state: LabState?
    /// How many times the main run loop has gone round: two lines of one turn were committed to the screen together.
    @ObservationIgnored var turn = 0

    init() {
      let observer = CFRunLoopObserverCreateWithHandler(nil, CFRunLoopActivity.beforeWaiting.rawValue, true, .max) {
        _, _ in
        MainActor.assumeIsolated { FootBus.shared.turn += 1 }
      }
      CFRunLoopAddObserver(CFRunLoopGetMain(), observer, .commonModes)
    }
  }

  @MainActor func footLog(_ line: String) {
    labLog("f=\(FootBus.shared.frame) t=\(FootBus.shared.turn) \(line)")
  }

  private func text(_ rect: CGRect) -> String {
    "\(fmt(rect.minX)),\(fmt(rect.minY)) \(fmt(rect.width))x\(fmt(rect.height))"
  }

  private func fmt(_ value: CGFloat) -> String {
    value == value.rounded() ? "\(Int(value))" : String(format: "%.1f", value)
  }

  private func text(_ insets: UIEdgeInsets) -> String {
    "\(fmt(insets.top)),\(fmt(insets.left)),\(fmt(insets.bottom)),\(fmt(insets.right))"
  }

  private func text(_ insets: EdgeInsets) -> String {
    "\(fmt(insets.top)),\(fmt(insets.leading)),\(fmt(insets.bottom)),\(fmt(insets.trailing))"
  }

  @available(iOS 27.1, *)
  private func text(_ edge: UIVerticalBarEdge) -> String {
    switch edge {
    case .unspecified: "nil"
    case .leading: "leading"
    case .trailing: "trailing"
    @unknown default: "?\(edge.rawValue)"
    }
  }

  private func text(_ edge: HorizontalEdge?) -> String {
    switch edge {
    case .none: "nil"
    case .leading?: "leading"
    case .trailing?: "trailing"
    }
  }

  // MARK: - The root's reader: fills the window, samples every frame

  @available(iOS 27.1, *)
  final class RootProbeView: UIView {
    private var link: CADisplayLink?
    private var last = ""
    private var lastChange = 0
    private var lastWindow = ""

    override init(frame: CGRect) {
      super.init(frame: frame)
      isUserInteractionEnabled = false
      let center = NotificationCenter.default
      for name in [
        UIScene.didActivateNotification, UIScene.willDeactivateNotification, UIScene.didEnterBackgroundNotification,
        UIScene.willEnterForegroundNotification, UIResponder.keyboardWillShowNotification,
        UIResponder.keyboardDidShowNotification, UIResponder.keyboardWillHideNotification,
        UIResponder.keyboardDidHideNotification, UIResponder.keyboardWillChangeFrameNotification,
      ] {
        center.addObserver(self, selector: #selector(said(_:)), name: name, object: nil)
      }
      center.addObserver(self, selector: #selector(dump), name: Foot.dump, object: nil)
    }

    @available(*, unavailable) required init?(coder: NSCoder) { fatalError() }

    @objc private func said(_ note: Notification) {
      var line = "system: \(note.name.rawValue)"
      if let end = note.userInfo?[UIResponder.keyboardFrameEndUserInfoKey] as? CGRect { line += " end=\(text(end))" }
      footLog(line)
    }

    override func didMoveToWindow() {
      super.didMoveToWindow()
      link?.invalidate()
      link = nil
      guard window != nil else { return }
      let link = CADisplayLink(target: self, selector: #selector(tick))
      link.add(to: .main, forMode: .common)
      self.link = link
    }

    private func regions(_ view: UIView, _ who: String) -> String {
      var parts: [String] = []
      let height = view.bounds.height
      for (name, kind) in [("occlusion", UIView.ReservedRegion.Kind.occlusion), ("division", .division)] {
        for (label, options) in [("", UIView.ReservedRegion.QueryOptions()), ("+inactive", .includeInactive)] {
          let found = view.reservedRegions(kind: kind, options: options)
          let said = found.map { region in
            let low = region.frame.maxY > height - 34 && name == "occlusion" ? " IN-BOTTOM-34" : ""
            return "[\(text(region.frame)) active=\(region.isActive)\(low)]"
          }.joined()
          parts.append("\(who).\(name)\(label)=\(said.isEmpty ? "none" : said)")
        }
      }
      return parts.joined(separator: " ")
    }

    private var reading: String {
      guard let window else { return "not in a window" }
      let own = convert(bounds, to: nil)
      return "reader=\(text(own)) window=\(text(window.bounds)) " + regions(self, "reader") + " "
        + regions(window, "window")
    }

    @objc private func tick() {
      let bus = FootBus.shared
      bus.frame += 1
      if let window {
        let bottom = window.safeAreaInsets.bottom
        if bus.windowBottom != bottom { bus.windowBottom = bottom }
        let now =
          "window \(fmt(window.bounds.width))x\(fmt(window.bounds.height)) safe=\(text(window.safeAreaInsets)) edge=\(text(window.traitCollection.verticalBarEdge)) orientation=\(window.windowScene?.effectiveGeometry.interfaceOrientation.rawValue ?? -1) scene=\(window.windowScene?.activationState.rawValue ?? -9)"
        if now != lastWindow {
          lastWindow = now
          footLog("root tick: \(now)")
        }
      }
      let now = reading
      if now != last {
        last = now
        lastChange = bus.frame
        footLog("regions changed: \(now)")
      } else if bus.frame % 300 == 0 {
        footLog("regions sampled every frame, unchanged since f=\(lastChange)")
      }
      if bus.frame % 4 == 0 { poll() }
    }

    @objc private func dump() {
      let bus = FootBus.shared
      footLog("dump[\(bus.label)] root regions: \(reading)")
      footLog("dump[\(bus.label)] root hinge: \(bus.hinge) runs=\(bus.hingeRuns)")
    }

    private func poll() {
      let path = "\(Foot.directory)/\(Foot.logName).cmd"
      guard let commands = try? String(contentsOfFile: path, encoding: .utf8) else { return }
      try? FileManager.default.removeItem(atPath: path)
      for line in commands.split(separator: "\n") { run(String(line)) }
    }

    private func run(_ command: String) {
      let bus = FootBus.shared
      let words = command.split(separator: " ", maxSplits: 1).map(String.init)
      let rest = words.count > 1 ? words[1] : ""
      footLog("------- command: \(command)")
      switch words.first {
      case "dump":
        bus.label = rest
        NotificationCenter.default.post(name: Foot.dump, object: nil)
        bus.dumps += 1
      case "open": bus.state?.open("S1")
      case "aside": bus.state?.toggleAside()
      case "sheet": bus.sheet = rest == "off" ? nil : rest
      case "focus": bus.focus = true
      case "blur": bus.focus = false
      case "end": NotificationCenter.default.post(name: Foot.end, object: nil)
      case "variant": bus.variant = rest
      case "bar":
        bus.barOff = rest == "off"
        var controller = window?.rootViewController
        while let now = controller {
          now.setNeedsUpdateOfVerticalBarConfiguration()
          for child in now.children { child.setNeedsUpdateOfVerticalBarConfiguration() }
          controller = now.presentedViewController
        }
      default: footLog("unknown command")
      }
    }
  }

  @available(iOS 27.1, *)
  struct RootProbe: UIViewRepresentable {
    func makeUIView(context: Context) -> RootProbeView { RootProbeView() }
    func updateUIView(_ view: RootProbeView, context: Context) {}
  }

  // MARK: - A reader of the edge, placed anywhere

  @available(iOS 27.1, *)
  final class EdgeProbeView: UIView {
    let name: String
    private var last = ""

    init(name: String) {
      self.name = name
      super.init(frame: .zero)
      isUserInteractionEnabled = false
      registerForTraitChanges(UITraitCollection.systemTraitsAffectingVerticalBarEdge) { (self: Self, _) in
        self.read("traits")
      }
      NotificationCenter.default.addObserver(self, selector: #selector(dump), name: Foot.dump, object: nil)
    }

    @available(*, unavailable) required init?(coder: NSCoder) { fatalError() }

    private var line: String {
      var said = "edge=\(text(traitCollection.verticalBarEdge))"
      said += " classes=\(traitCollection.horizontalSizeClass.rawValue)/\(traitCollection.verticalSizeClass.rawValue)"
      guard let window else { return said + " NOT-IN-A-WINDOW" }
      said += " window=\(fmt(window.bounds.width))x\(fmt(window.bounds.height))"
      said += " windowSafe=\(text(window.safeAreaInsets))"
      said += " frameInWindow=\(text(convert(bounds, to: nil)))"
      return said
    }

    /// Each view controller above this view, with the edge its own traits hold.
    private var chain: String {
      var parts: [String] = []
      var responder: UIResponder? = self
      while let now = responder {
        if let controller = now as? UIViewController {
          let behavior = controller.preferredVerticalBarBehavior == .disabled ? " asks-off" : ""
          parts.append(
            "\(String(describing: type(of: controller)).prefix(34)):\(text(controller.traitCollection.verticalBarEdge))\(behavior)"
          )
        } else if let window = now as? UIWindow {
          parts.append("window:\(text(window.traitCollection.verticalBarEdge))")
        }
        responder = now.next
      }
      return parts.joined(separator: " < ")
    }

    private func read(_ why: String) {
      let now = line
      guard now != last else { return }
      last = now
      footLog("probe \(name) uikit(\(why)): \(now)")
    }

    override func layoutSubviews() {
      super.layoutSubviews()
      read("layout")
    }

    override func didMoveToWindow() {
      super.didMoveToWindow()
      read("moved")
    }

    @objc private func dump() {
      footLog("dump[\(FootBus.shared.label)] probe \(name) uikit: \(line) | chain: \(chain)")
    }
  }

  @available(iOS 27.1, *)
  struct EdgeProbeUIKit: UIViewRepresentable {
    let name: String
    func makeUIView(context: Context) -> EdgeProbeView { EdgeProbeView(name: name) }
    func updateUIView(_ view: EdgeProbeView, context: Context) {}
  }

  /// Reads SwiftUI's edge, the size and safe area its reader is given and the reserved regions, wherever it is put.
  /// Put it in a background that ignores the safe area on every edge.
  @available(iOS 27.1, *)
  struct EdgeProbe: View {
    let name: String
    @Environment(\.toolbarVerticalEdge) private var edge
    @Environment(\.horizontalSizeClass) private var horizontal
    @Environment(\.verticalSizeClass) private var vertical
    private let bus = FootBus.shared

    var body: some View {
      GeometryReader { proxy in
        let now = line(proxy)
        Color.clear
          .onChange(of: now, initial: true) { _, now in footLog("probe \(name) swiftui: \(now)") }
          .onChange(of: bus.dumps) { footLog("dump[\(bus.label)] probe \(name) swiftui: \(now)") }
      }
      .background { EdgeProbeUIKit(name: name) }
      .allowsHitTesting(false)
      .accessibilityHidden(true)
    }

    private func line(_ proxy: GeometryProxy) -> String {
      var said = "edge=\(text(edge))"
      said += " classes=\(horizontal.map { "\($0)" } ?? "nil")/\(vertical.map { "\($0)" } ?? "nil")"
      said += " size=\(fmt(proxy.size.width))x\(fmt(proxy.size.height))"
      said += " global=\(text(proxy.frame(in: .global)))"
      said += " safe=\(text(proxy.safeAreaInsets))"
      for (kind, kindName) in [(ReservedRegion.Kind.occlusion, "occlusion"), (.division, "division")] {
        for (options, label) in [(ReservedRegion.QueryOptions(), ""), (.includeInactive, "+inactive")] {
          let found = proxy.reservedRegions(kind: kind, options: options).map { region in
            "[\(text(region.frame)) active=\(region.isActive) margins=\(text(region.margins))]"
          }.joined()
          said += " \(kindName)\(label)=\(found.isEmpty ? "none" : found)"
        }
      }
      return said
    }
  }

  // MARK: - Where things stand, in the window's points

  final class MarkView: UIView {
    let name: String

    init(name: String) {
      self.name = name
      super.init(frame: .zero)
      isUserInteractionEnabled = false
      let center = NotificationCenter.default
      center.addObserver(self, selector: #selector(dump), name: Foot.dump, object: nil)
      if name == "ground" { center.addObserver(self, selector: #selector(end), name: Foot.end, object: nil) }
    }

    @available(*, unavailable) required init?(coder: NSCoder) { fatalError() }

    @objc private func dump() {
      guard let window else { return }
      let frame = convert(bounds, to: nil)
      let height = window.bounds.height
      footLog(
        "dump[\(FootBus.shared.label)] mark \(name): top=\(fmt(frame.minY)) bottom=\(fmt(frame.maxY)) fromWindowBottom=\(fmt(height - frame.maxY)) topFromWindowBottom=\(fmt(height - frame.minY)) x=\(fmt(frame.minX))..\(fmt(frame.maxX)) window=\(fmt(window.bounds.width))x\(fmt(height))"
      )
      if name == "ground", let list = MarkView.list(in: window) { footLog("dump[\(FootBus.shared.label)] \(said(list))") }
    }

    private func said(_ list: UIScrollView) -> String {
      let frame = list.convert(list.bounds, to: nil)
      let rest = list.contentSize.height - list.bounds.height + list.adjustedContentInset.bottom
      return
        "list \(type(of: list)): frameInWindow=\(text(frame)) content=\(fmt(list.contentSize.height)) adjustedInset=\(text(list.adjustedContentInset)) inset=\(text(list.contentInset)) offset=\(fmt(list.contentOffset.y)) endOffset=\(fmt(rest)) contentEndsAtFromWindowBottom=\(fmt((list.window?.bounds.height ?? 0) - (frame.minY + list.contentSize.height - list.contentOffset.y)))"
    }

    /// The long list's scroll view, found by its length.
    static func list(in view: UIView) -> UIScrollView? {
      if let scroll = view as? UIScrollView, scroll.contentSize.height > 3000 { return scroll }
      for child in view.subviews {
        if let found = list(in: child) { return found }
      }
      return nil
    }

    @objc private func end() {
      guard let window, let list = MarkView.list(in: window) else {
        footLog("end: no list found")
        return
      }
      let rest = list.contentSize.height - list.bounds.height + list.adjustedContentInset.bottom
      list.setContentOffset(CGPoint(x: list.contentOffset.x, y: rest), animated: false)
      footLog("end: scrolled to rest, \(said(list))")
    }
  }

  struct Mark: UIViewRepresentable {
    let name: String
    func makeUIView(context: Context) -> MarkView { MarkView(name: name) }
    func updateUIView(_ view: MarkView, context: Context) {}
  }

  // MARK: - The screen in the main view's column

  @available(iOS 27.1, *)
  struct FootItem: View {
    let state: LabState
    let id: String
    @State private var draft = ""
    @State private var screenBottom: CGFloat = 0
    @FocusState private var focused: Bool
    @Environment(\.toolbarVerticalEdge) private var edge
    private let bus = FootBus.shared

    private struct Seen: Equatable {
      var size: CGSize
      var bottom: CGFloat
    }

    var body: some View {
      placed
        .background { EdgeProbe(name: "main").ignoresSafeArea() }
        .onGeometryChange(for: Seen.self) { proxy in
          Seen(size: proxy.size, bottom: proxy.safeAreaInsets.bottom)
        } action: { now in
          screenBottom = now.bottom
          footLog(
            "main column geometry: size=\(fmt(now.size.width))x\(fmt(now.size.height)) bottomSafe=\(fmt(now.bottom)) edge=\(text(edge)) variant=\(bus.variant)"
          )
        }
        .onChange(of: edge, initial: true) { _, now in
          footLog("main column edge: \(text(now)) bottomSafe=\(fmt(screenBottom)) variant=\(bus.variant)")
        }
        .onChange(of: bus.focus) { _, now in focused = now }
        .onChange(of: focused) { _, now in footLog("field focused=\(now)") }
        .onChange(of: bus.variant) { _, now in footLog("variant now \(now), focused=\(focused)") }
        .navigationTitle(id)
        .navigationBarTitleDisplayMode(.inline)
        .toolbar {
          ToolbarItem(placement: .topBarTrailing) {
            Button {
              state.push()
            } label: {
              Label("Push", systemImage: "arrow.right.circle")
            }
            .accessibilityIdentifier("lab.push")
          }
          ToolbarItem(placement: .topBarTrailing) {
            Button {
              state.toggleAside()
            } label: {
              Label(state.asideWanted ? "Hide aside" : "Show aside", systemImage: "sidebar.trailing")
            }
            .accessibilityIdentifier("lab.aside")
          }
        }
        .toolbarVerticalBehavior(bus.barOff ? .disabled : .automatic)
        .sheet(
          isPresented: Binding(
            get: { bus.sheet != nil },
            set: { shown in
              if !shown { bus.sheet = nil }
            })
        ) {
          FootSheet(detent: bus.sheet ?? "medium")
        }
    }

    private var list: some View {
      List(1..<301) { n in
        Text(verbatim: n == 300 ? "Row 300, the last" : "Row \(n)")
          .listRowBackground(n == 300 ? AnyView(Color.green.opacity(0.4).overlay { Mark(name: "lastrow") }) : nil)
      }
      .listStyle(.plain)
    }

    /// The container's own bottom inset while no keyboard stands there, as #717's prototype told them apart.
    private var pull: CGFloat { screenBottom <= bus.windowBottom + 0.5 ? bus.windowBottom : 0 }

    @ViewBuilder
    private var placed: some View {
      switch bus.variant {
      case "a-content":
        list.safeAreaInset(edge: .bottom, spacing: 0) { foot().ignoresSafeArea(.container, edges: .bottom) }
      case "a-inner":
        list.safeAreaInset(edge: .bottom, spacing: 0) { foot(inner: .bottom) }
      case "a-outer":
        list.safeAreaInset(edge: .bottom, spacing: 0) { foot() }.ignoresSafeArea(.container, edges: .bottom)
      case "b-negpad":
        list.safeAreaInset(edge: .bottom, spacing: 0) { foot().padding(.bottom, -pull) }
      case "c-bar":
        list.safeAreaBar(edge: .bottom, spacing: 0) { foot() }
      case "c-bar-ignore":
        list.safeAreaBar(edge: .bottom, spacing: 0) { foot().ignoresSafeArea(.container, edges: .bottom) }
      case "c-bar-outer":
        list.safeAreaBar(edge: .bottom, spacing: 0) { foot() }.ignoresSafeArea(.container, edges: .bottom)
      case "c-safepad":
        list.safeAreaInset(edge: .bottom, spacing: 0) { foot() }.safeAreaPadding(.bottom, -pull)
      case "follow":
        // One structure whichever way the edge reads, so the field is the same field across the change.
        list.safeAreaInset(edge: .bottom, spacing: 0) { foot() }
          .ignoresSafeArea(.container, edges: edge != nil ? .bottom : [])
      case "follow-content":
        list.safeAreaInset(edge: .bottom, spacing: 0) {
          foot().ignoresSafeArea(.container, edges: edge != nil ? .bottom : [])
        }
      case "follow-b":
        list.safeAreaInset(edge: .bottom, spacing: 0) { foot().padding(.bottom, edge != nil ? -pull : 0) }
      default:
        // band, c-additional and follow-additional: the last two are the hosting controller's to do.
        list.safeAreaInset(edge: .bottom, spacing: 0) { foot() }
      }
    }

    private func foot(inner: Edge.Set = []) -> some View {
      HStack(spacing: 8) {
        TextField("Message", text: $draft)
          .textFieldStyle(.roundedBorder)
          .focused($focused)
          .accessibilityIdentifier("lab.field")
          .background { Mark(name: "field") }
          .onGeometryChange(for: CGRect.self) { $0.frame(in: .global) } action: { now in
            footLog(
              "field moved: global top=\(fmt(now.minY)) bottom=\(fmt(now.maxY)) edge=\(text(edge)) bottomSafe=\(fmt(screenBottom)) focused=\(focused)"
            )
          }
        Button("Send") {}
          .buttonStyle(.borderedProminent)
      }
      .ignoresSafeArea(.container, edges: inner)
      .padding(8)
      .background { Mark(name: "foot") }
      .background { Color(red: 1, green: 0.85, blue: 0.2).overlay { Mark(name: "ground") }.ignoresSafeArea(.container, edges: .bottom) }
    }
  }

  @available(iOS 27.1, *)
  struct FootSheet: View {
    let detent: String
    private let bus = FootBus.shared

    var body: some View {
      NavigationStack {
        List(1..<30) { n in Text(verbatim: "Sheet row \(n)") }
          .background { EdgeProbe(name: "sheet-\(detent)").ignoresSafeArea() }
          .navigationTitle("Sheet, \(detent)")
          .navigationBarTitleDisplayMode(.inline)
          .toolbar {
            ToolbarItem(placement: .confirmationAction) {
              Button {
                bus.sheet = nil
              } label: {
                Label("Done", systemImage: "checkmark")
              }
            }
            ToolbarItem(placement: .cancellationAction) {
              Button {
                bus.sheet = nil
              } label: {
                Label("Close", systemImage: "xmark")
              }
            }
          }
          .toolbarVerticalBehavior(bus.barOff ? .disabled : .automatic)
      }
      .presentationDetents([detent == "large" ? .large : .medium])
    }
  }

  /// What the root of the lab adds for #729.
  @available(iOS 27.1, *)
  struct FootRoot: ViewModifier {
    let state: LabState
    private let bus = FootBus.shared

    func body(content: Content) -> some View {
      content
        .background {
          ZStack {
            RootProbe()
            EdgeProbe(name: "root")
          }
          .ignoresSafeArea()
        }
        .onHingeChange { _, new in
          bus.hingeRuns += 1
          if let hinge = new.hinge {
            bus.hinge = "\(hinge.status) \(Int(hinge.angle.degrees.rounded()))"
          } else {
            bus.hinge = "called with a context holding no hinge"
          }
          footLog("onHingeChange run \(bus.hingeRuns): \(bus.hinge)")
        }
        .toolbarVerticalBehavior(bus.barOff ? .disabled : .automatic)
        .onAppear { bus.state = state }
    }
  }
#endif
