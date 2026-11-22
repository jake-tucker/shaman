// PROTOTYPE for #725, "Whether the three columns can be built without the prototype's workarounds". Throwaway:
// never merge.
//
// A lab of placeholder screens in the app's own shape, a listing, a Session with screens pushed over it and an Aside,
// under containers that rest on nothing timed. `--lab <uikit|swiftui>` chooses the container:
//
//   uikit    UISplitViewController hosted directly, a UINavigationController in each column, each screen a hosting
//            controller. `--lab-aside inspector` stands the Aside in the split view's own inspector column;
//            `--lab-aside own` (the default) lays it out beside the split view by hand.
//   swiftui  NavigationSplitView with a NavigationStack in its detail column, as #714's prototype, and the Aside
//            laid out beside it by hand. `--lab-back <coordinator|appear>` chooses what closes what is open.
//
// `--lab-open <n>` opens a Session with n-1 screens pushed over it at launch, `--lab-aside-up` asks for the Aside,
// `--lab-hold` has the inspector held back by the app through a change of width, and `--lab-widen` widens it to
// the half past the fold. Every event is a line in /tmp/duo725/app.log.

#if DEBUG
  import SwiftUI
  import UIKit

  public enum Lab {
    public static var variant: String? { value("--lab") }

    static func value(_ flag: String) -> String? {
      let arguments = ProcessInfo.processInfo.arguments
      return arguments.firstIndex(of: flag).flatMap { arguments.dropFirst($0 + 1).first }
    }

    static func has(_ flag: String) -> Bool { ProcessInfo.processInfo.arguments.contains(flag) }
  }

  func labLog(_ line: String) {
    #if targetEnvironment(simulator)
      let url = URL(fileURLWithPath: "/tmp/duo725/app.log")
      let stamp = String(format: "%07.3f", Date().timeIntervalSince1970.truncatingRemainder(dividingBy: 1000))
      let data = Data("\(stamp) \(line)\n".utf8)
      if let handle = try? FileHandle(forWritingTo: url) {
        handle.seekToEndOfFile()
        handle.write(data)
        try? handle.close()
      } else {
        try? data.write(to: url)
      }
    #endif
  }

  @MainActor protocol LabDriver: AnyObject {
    func open(_ id: String)
    func push(_ id: String)
    func asideChanged()
  }

  @MainActor @Observable
  final class LabState {
    /// What is open in the main view: its root, then what is pushed over it.
    var path: [String] = []
    /// How often the reader came back to the listing while it stood alone.
    var returned = 0
    /// The reader's standing choice, and what is on screen.
    var asideWanted = Lab.has("--lab-aside-up")
    var asideShown = false
    var asidePath: [Int] = []
    @ObservationIgnored weak var driver: (any LabDriver)?

    init() {
      let count = Lab.value("--lab-open").flatMap(Int.init) ?? 0
      path = (0..<count).map { "S1" + String(repeating: "+", count: $0) }
    }

    var rest: [String] {
      get { Array(path.dropFirst()) }
      set { path = Array(path.prefix(1)) + newValue }
    }

    var summary: String {
      "path=[\(path.joined(separator: ">"))] returned=\(returned) wanted=\(asideWanted ? 1 : 0) aside=\(asideShown ? 1 : 0) work=\(asidePath.count)"
    }

    func open(_ id: String) {
      if let driver { driver.open(id) } else { path = [id] }
    }

    func push() {
      let next = (path.last ?? "?") + "+"
      if let driver { driver.push(next) } else { path.append(next) }
    }

    func toggleAside() {
      asideWanted.toggle()
      labLog("reader toggled aside, wanted=\(asideWanted)")
      driver?.asideChanged()
    }
  }

  // MARK: - The screens

  struct LabSidebar: View {
    let state: LabState
    @State private var query = ""

    var body: some View {
      List(1..<13) { n in
        Button {
          state.open("S\(n)")
        } label: {
          Text(verbatim: "Session \(n)")
            .frame(maxWidth: .infinity, alignment: .leading)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityIdentifier("lab.row.\(n)")
        .listRowBackground(state.path.first == "S\(n)" ? Color.accentColor.opacity(0.14) : nil)
      }
      .listStyle(.plain)
      .navigationTitle("Listing")
      .navigationBarTitleDisplayMode(.inline)
      .searchable(text: $query, placement: .navigationBarDrawer(displayMode: .always))
      .toolbar {
        ToolbarItem(placement: .bottomBar) {
          Button {
            state.open("New")
          } label: {
            Label("New session", systemImage: "square.and.pencil")
          }
        }
      }
    }
  }

  struct LabItem: View {
    let state: LabState
    let id: String
    @State private var draft = ""

    var body: some View {
      ScrollView {
        VStack(alignment: .leading, spacing: 8) {
          ForEach(0..<40) { line in
            Text(verbatim: "\(id) line \(line): the quick brown fox jumps over the lazy dog and keeps on going")
          }
        }
        .padding()
      }
      .safeAreaInset(edge: .bottom) {
        TextField("Message", text: $draft)
          .textFieldStyle(.roundedBorder)
          .padding(8)
          .background(.bar)
          .accessibilityIdentifier("lab.field")
      }
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
    }
  }

  struct LabLanding: View {
    var body: some View {
      ContentUnavailableView("New session", systemImage: "square.and.pencil")
        .navigationTitle("New session")
        .navigationBarTitleDisplayMode(.inline)
    }
  }

  /// The Aside: a Turn's work, each Activity's whole pushed within it. What is pushed is the state's, so it waits
  /// while the Aside has no room.
  struct LabAsideStack: View {
    @Bindable var state: LabState

    var body: some View {
      NavigationStack(path: $state.asidePath) {
        List(1..<9) { n in
          NavigationLink(value: n) { Text(verbatim: "Activity \(n)") }
            .accessibilityIdentifier("lab.activity.\(n)")
        }
        .navigationTitle("Work")
        .navigationBarTitleDisplayMode(.inline)
        .navigationDestination(for: Int.self) { n in
          Text(verbatim: "The whole of activity \(n)")
            .navigationTitle("Activity \(n)")
            .navigationBarTitleDisplayMode(.inline)
        }
      }
    }
  }

  // MARK: - The fold

  struct LabFold: Equatable, CustomStringConvertible {
    var size: CGSize
    var fold: CGRect?

    var description: String {
      "size=\(Int(size.width))x\(Int(size.height)) fold=" + (fold.map { "\(Int($0.minX)),\(Int($0.minY)) \(Int($0.width))x\(Int($0.height))" } ?? "none")
    }
  }

  @available(iOS 27.1, *)
  extension UIView {
    var labFold: LabFold {
      LabFold(size: bounds.size, fold: reservedRegions(kind: .division).first { $0.isActive }?.frame)
    }
  }

  /// A view that reads the fold in one of UIKit's passes and says when what it read changes.
  @available(iOS 27.1, *)
  final class FoldProbeView: UIView {
    enum Pass: String { case layoutSubviews, updateConstraints }
    let pass: Pass
    private var last: LabFold?
    private var runs = 0

    init(pass: Pass) {
      self.pass = pass
      super.init(frame: .zero)
      isUserInteractionEnabled = false
    }

    @available(*, unavailable) required init?(coder: NSCoder) { fatalError() }

    private func read() {
      runs += 1
      let now = labFold
      if now != last {
        last = now
        labLog("fold probe \(pass.rawValue) run \(runs): \(now)")
      }
    }

    override func didMoveToWindow() {
      super.didMoveToWindow()
      if pass == .updateConstraints { setNeedsUpdateConstraints() }
    }

    override func layoutSubviews() {
      super.layoutSubviews()
      if pass == .layoutSubviews { read() }
    }

    override func updateConstraints() {
      if pass == .updateConstraints { read() }
      super.updateConstraints()
    }
  }

  @available(iOS 27.1, *)
  struct FoldProbe: UIViewRepresentable {
    let pass: FoldProbeView.Pass
    func makeUIView(context: Context) -> FoldProbeView { FoldProbeView(pass: pass) }
    func updateUIView(_ view: FoldProbeView, context: Context) {}
  }

  /// The same question asked of SwiftUI three ways, each saying when what it read changes.
  @available(iOS 27.1, *)
  struct FoldProbes: View {
    var body: some View {
      ZStack {
        FoldProbe(pass: .layoutSubviews)
        FoldProbe(pass: .updateConstraints)
        Color.clear
          .onGeometryChange(for: LabFold.self) { proxy in
            LabFold(size: proxy.size, fold: proxy.reservedRegions(kind: .division).first { $0.isActive }?.frame)
          } action: { now in
            labLog("fold probe onGeometryChange: \(now)")
          }
        GeometryReader { proxy in
          let now = LabFold(size: proxy.size, fold: proxy.reservedRegions(kind: .division).first { $0.isActive }?.frame)
          Color.clear.onChange(of: now, initial: true) { _, now in
            labLog("fold probe GeometryReader: \(now)")
          }
        }
      }
      .allowsHitTesting(false)
      .accessibilityHidden(true)
    }
  }

  // MARK: - UIKit's split view, hosted directly

  final class LabHost: UIHostingController<AnyView> {
    let id: String

    init(id: String, _ view: some View) {
      self.id = id
      super.init(rootView: AnyView(view))
    }

    @available(*, unavailable) required init?(coder: NSCoder) { fatalError() }
  }

  @available(iOS 27.1, *)
  final class LabColumns: UISplitViewController, UISplitViewControllerDelegate, UINavigationControllerDelegate,
    LabDriver
  {
    let state: LabState
    let inspects: Bool
    let sidebar = UINavigationController()
    let main = UINavigationController()
    private(set) lazy var aside = UIHostingController(rootView: LabAsideStack(state: state))
    private let holds = Lab.has("--lab-hold")
    private let widens = Lab.has("--lab-widen")
    private var lastFold: LabFold?
    /// Between the system asking how to expand and saying it has.
    private var expanding = false
    /// Where the Aside is laid out by hand, whoever does so is asked to lay out again.
    var relayout: (() -> Void)?

    init(state: LabState, inspects: Bool) {
      self.state = state
      self.inspects = inspects
      super.init(style: .doubleColumn)
      delegate = self
      sidebar.delegate = self
      main.delegate = self
      sidebar.viewControllers = [LabHost(id: "listing", LabSidebar(state: state))]
      main.viewControllers = state.path.isEmpty ? [landing()] : state.path.map(screen)
      setViewController(sidebar, for: .primary)
      setViewController(main, for: .secondary)
      if inspects, !holds {
        setViewController(aside, for: .inspector)
        labLog("inspector set, wrapped in \(String(describing: aside.parent.map { type(of: $0) }))")
      }
      if holds, inspects {
        minimumSecondaryColumnWidth = Self.mainMinimum
        minimumInspectorColumnWidth = Self.asideMinimum
      }
      if !Lab.has("--lab-automatic") {
        preferredSplitBehavior = .tile
        preferredDisplayMode = .oneBesideSecondary
      }
    }

    @available(*, unavailable) required init?(coder: NSCoder) { fatalError() }

    private func landing() -> LabHost { LabHost(id: "landing", LabLanding()) }
    private func screen(_ id: String) -> LabHost { LabHost(id: id, LabItem(state: state, id: id)) }

    private func ids(_ nav: UINavigationController) -> [String] {
      nav.viewControllers.map { ($0 as? LabHost)?.id ?? "\(type(of: $0))" }
    }

    private var where_: String {
      "collapsed=\(isCollapsed) mode=\(displayMode.rawValue) sidebar=\(ids(sidebar)) main=\(ids(main))"
        + (inspects ? " inspector=\(isShowing(.inspector))" : "")
    }

    private func name(_ column: UISplitViewController.Column) -> String {
      switch column {
      case .primary: "sidebar"
      case .secondary: "main"
      case .inspector: "aside"
      case .supplementary: "supplementary"
      case .compact: "compact"
      @unknown default: "?"
      }
    }

    override func viewDidAppear(_ animated: Bool) {
      super.viewDidAppear(animated)
      labLog("columns appeared \(where_)")
      if inspects, !holds, state.asideWanted, !isCollapsed, !state.path.isEmpty { show(.inspector) }
      reconcile("appearing")
    }

    /// Whether the system stands columns side by side here: its own word for there being room.
    var tiles: Bool { !isCollapsed && view.bounds.width >= Self.mainMinimum + Self.asideMinimum }

    /// The least the main view keeps beside the Aside, and the least the Aside takes: given to the split view, and
    /// the app's own rule for whether the Aside has room.
    static let mainMinimum: CGFloat = 400
    static let asideMinimum: CGFloat = 290

    /// Shows the Aside where the reader has it shown and the system has room, at a moment the system named.
    private func reconcile(_ why: String) {
      guard inspects, holds else { return }
      let want = state.asideWanted && !state.path.isEmpty && tiles
      labLog("reconcile at \(why): want=\(want) width=\(Int(view.bounds.width)) \(where_)")
      if want {
        // The column holds the Aside only while the reader has it shown: a view controller set in it is shown.
        if viewController(for: .inspector) == nil { setViewController(aside, for: .inspector) }
        if !isShowing(.inspector) { show(.inspector) }
      } else if isShowing(.inspector), !state.asideWanted || state.path.isEmpty {
        hide(.inspector)
      }
    }

    // The reader's acts.

    func open(_ id: String) {
      labLog("open \(id)")
      state.path = [id]
      main.setViewControllers([id == "New" ? landing() : screen(id)], animated: false)
      show(.secondary)
    }

    func push(_ id: String) {
      labLog("push \(id)")
      main.pushViewController(screen(id), animated: true)
    }

    func asideChanged() {
      guard inspects else {
        relayout?()
        return
      }
      if holds {
        reconcile("the reader's toggle")
      } else if state.asideWanted {
        show(.inspector)
      } else {
        hide(.inspector)
      }
    }

    // What the system says.

    func splitViewController(
      _ svc: UISplitViewController, topColumnForCollapsingToProposedTopColumn proposed: UISplitViewController.Column
    ) -> UISplitViewController.Column {
      let top: UISplitViewController.Column = state.path.isEmpty ? .primary : .secondary
      labLog("topColumnForCollapsing proposed=\(name(proposed)) answered=\(name(top)) \(where_)")
      if holds, inspects, viewController(for: .inspector) != nil {
        // Alone on screen the system would stand the inspector as a sheet; with nothing in the column there is
        // nothing to stand.
        labLog("taking the aside out of its column before the collapse")
        setViewController(nil, for: .inspector)
      }
      return top
    }

    func splitViewController(
      _ svc: UISplitViewController,
      displayModeForExpandingToProposedDisplayMode proposed: UISplitViewController.DisplayMode
    ) -> UISplitViewController.DisplayMode {
      labLog("displayModeForExpanding proposed=\(proposed.rawValue) \(where_)")
      expanding = true
      return proposed
    }

    func splitViewControllerDidCollapse(_ svc: UISplitViewController) {
      labLog("didCollapse \(where_)")
      relayout?()
    }

    func splitViewControllerDidExpand(_ svc: UISplitViewController) {
      labLog("didExpand \(where_)")
      expanding = false
      relayout?()
      reconcile("didExpand")
    }

    func splitViewController(_ svc: UISplitViewController, willShow column: UISplitViewController.Column) {
      labLog("willShow \(name(column)) \(where_)")
    }

    func splitViewController(_ svc: UISplitViewController, willHide column: UISplitViewController.Column) {
      labLog("willHide \(name(column)) \(where_)")
    }

    func splitViewController(_ svc: UISplitViewController, didShow column: UISplitViewController.Column) {
      labLog("didShow \(name(column)) \(where_)")
      if column == .inspector { state.asideShown = true }
    }

    func splitViewController(_ svc: UISplitViewController, didHide column: UISplitViewController.Column) {
      labLog("didHide \(name(column)) \(where_)")
      if column == .inspector { state.asideShown = false }
    }

    func splitViewController(
      _ svc: UISplitViewController, willChangeTo displayMode: UISplitViewController.DisplayMode
    ) {
      labLog("willChangeTo mode=\(displayMode.rawValue) \(where_)")
    }

    func navigationController(_ nav: UINavigationController, willShow vc: UIViewController, animated: Bool) {
      let which = nav === main ? "main" : "sidebar"
      let shown = (vc as? LabHost)?.id ?? "\(type(of: vc))"
      guard let coordinator = nav.transitionCoordinator else {
        labLog("nav \(which) willShow \(shown), no transition")
        return
      }
      labLog("nav \(which) willShow \(shown), transition interactive=\(coordinator.isInteractive)")
      coordinator.notifyWhenInteractionChanges { context in
        labLog(
          "nav \(which) finger lifted at \(String(format: "%.2f", context.percentComplete)) cancelled=\(context.isCancelled)"
        )
      }
      coordinator.animate(alongsideTransition: nil) { context in
        labLog("nav \(which) transition to \(shown) ended cancelled=\(context.isCancelled)")
      }
    }

    func navigationController(_ nav: UINavigationController, didShow vc: UIViewController, animated: Bool) {
      let which = nav === main ? "main" : "sidebar"
      labLog("nav \(which) didShow \((vc as? LabHost)?.id ?? "\(type(of: vc))") \(where_)")
      let narrow = (view.window ?? view).traitCollection.horizontalSizeClass == .compact
      if nav === sidebar, sidebar.topViewController === vc, sidebar.viewControllers.count == 1, !state.path.isEmpty {
        labLog("the listing is its navigation controller's top: narrow=\(narrow) expanding=\(expanding)")
      }
      if nav === sidebar, isCollapsed, narrow, sidebar.topViewController === vc, sidebar.viewControllers.count == 1,
        !state.path.isEmpty
      {
        // The listing's own navigation controller has shown the listing, alone on screen: the reader came back.
        state.returned += 1
        state.path = []
        main.setViewControllers([landing()], animated: false)
        labLog("the reader returned to the listing, \(state.returned)")
      }
      if nav === main {
        let path = ids(main).filter { $0 != "landing" && $0 != "New" }
        if path != state.path, !(path.isEmpty && isCollapsed) {
          labLog("path \(state.path) -> \(path)")
          state.path = path
        }
      }
    }

    override func viewWillTransition(to size: CGSize, with coordinator: any UIViewControllerTransitionCoordinator) {
      labLog("viewWillTransition to \(Int(size.width))x\(Int(size.height)) \(where_)")
      super.viewWillTransition(to: size, with: coordinator)
      coordinator.animate(alongsideTransition: nil) { [weak self] _ in self?.reconcile("the size transition's end") }
    }

    override func willTransition(
      to newCollection: UITraitCollection, with coordinator: any UIViewControllerTransitionCoordinator
    ) {
      labLog("willTransition to width class \(newCollection.horizontalSizeClass.rawValue) \(where_)")
      super.willTransition(to: newCollection, with: coordinator)
    }

    override func viewDidLayoutSubviews() {
      super.viewDidLayoutSubviews()
      // What is on screen is asked of the split view: it says nothing of a column set and shown at once.
      if inspects, state.asideShown != isShowing(.inspector) { state.asideShown = isShowing(.inspector) }
      let now = view.labFold
      guard now != lastFold else { return }
      lastFold = now
      labLog("columns laid out: \(now) \(where_)")
      guard inspects, widens else { return }
      if let fold = now.fold, fold.height > fold.width, isShowing(.inspector) {
        let width = now.size.width - fold.minX - 20
        labLog("widening the aside to \(Int(width))")
        minimumInspectorColumnWidth = width
        maximumInspectorColumnWidth = width
        preferredInspectorColumnWidth = width
      } else if now.fold == nil, preferredInspectorColumnWidth != UISplitViewController.automaticDimension {
        labLog("giving the aside its own width back")
        minimumInspectorColumnWidth = UISplitViewController.automaticDimension
        maximumInspectorColumnWidth = UISplitViewController.automaticDimension
        preferredInspectorColumnWidth = UISplitViewController.automaticDimension
      }
    }
  }

  /// The Aside laid out by hand beside a split view of two columns: where it stands is read from the window and the
  /// fold in one layout pass, so nothing is presented and nothing is taken down.
  @available(iOS 27.1, *)
  final class LabOwn: UIViewController {
    let state: LabState
    let columns: LabColumns
    private var last = ""

    init(state: LabState) {
      self.state = state
      columns = LabColumns(state: state, inspects: false)
      super.init(nibName: nil, bundle: nil)
      columns.relayout = { [weak self] in self?.view.setNeedsLayout() }
    }

    @available(*, unavailable) required init?(coder: NSCoder) { fatalError() }

    override func viewDidLoad() {
      super.viewDidLoad()
      for child in [columns, columns.aside] as [UIViewController] {
        addChild(child)
        view.addSubview(child.view)
        child.didMove(toParent: self)
      }
      registerForTraitChanges([UITraitHorizontalSizeClass.self]) { (self: Self, _) in
        labLog("own: width class \(self.traitCollection.horizontalSizeClass.rawValue)")
        self.view.setNeedsLayout()
      }
    }

    override func viewWillLayoutSubviews() {
      super.viewWillLayoutSubviews()
      let bounds = view.bounds
      let now = view.labFold
      let wide = traitCollection.horizontalSizeClass == .regular && bounds.width >= 700
      let laptop = now.fold.map { $0.width > $0.height } ?? false
      let on = state.asideWanted && !state.path.isEmpty && (wide || laptop)
      var main = bounds
      var aside = CGRect(x: bounds.maxX, y: 0, width: 0, height: bounds.height)
      if on {
        if let fold = now.fold, laptop {
          main.size.height = fold.minY
          aside = CGRect(x: 0, y: fold.maxY, width: bounds.width, height: bounds.height - fold.maxY)
        } else {
          let width = now.fold.map { bounds.width - $0.minX - 20 } ?? min(360, max(290, bounds.width * 0.36))
          main.size.width = bounds.width - width
          aside = CGRect(x: main.maxX, y: 0, width: width, height: bounds.height)
        }
      }
      columns.view.frame = main
      columns.aside.view.frame = aside
      columns.aside.view.isHidden = !on
      let mode: UISplitViewController.DisplayMode = on && !laptop ? .secondaryOnly : .oneBesideSecondary
      if columns.preferredDisplayMode != mode { columns.preferredDisplayMode = mode }
      if state.asideShown != on { state.asideShown = on }
      let said = "own laid out: \(now) aside=\(on) main=\(Int(main.width))x\(Int(main.height))"
      if said != last {
        last = said
        labLog(said)
      }
    }
  }

  @available(iOS 27.1, *)
  struct LabUIKit: UIViewControllerRepresentable {
    let state: LabState

    func makeUIViewController(context: Context) -> UIViewController {
      if Lab.value("--lab-aside") == "inspector" {
        let columns = LabColumns(state: state, inspects: true)
        state.driver = columns
        return columns
      }
      let own = LabOwn(state: state)
      state.driver = own.columns
      return own
    }

    func updateUIViewController(_ controller: UIViewController, context: Context) {}
  }

  // MARK: - SwiftUI's split view

  /// A view controller among a SwiftUI screen's own, told as UIKit tells one of appearing.
  final class AppearProbeController: UIViewController {
    var name = ""
    var willAppear: ((AppearProbeController) -> Void)?

    override func viewWillAppear(_ animated: Bool) {
      super.viewWillAppear(animated)
      let coordinator = transitionCoordinator
      let from = coordinator?.viewController(forKey: .from).map { "\(type(of: $0))" } ?? "nil"
      let to = coordinator?.viewController(forKey: .to).map { "\(type(of: $0))" } ?? "nil"
      labLog(
        "probe \(name) willAppear animated=\(animated) transition=\(coordinator != nil) interactive=\(coordinator?.isInteractive ?? false) from=\(from.prefix(40)) to=\(to.prefix(40)) collapsed=\(String(describing: splitViewController?.isCollapsed))"
      )
      willAppear?(self)
    }

    override func viewDidAppear(_ animated: Bool) {
      super.viewDidAppear(animated)
      labLog("probe \(name) didAppear")
    }

    override func viewDidDisappear(_ animated: Bool) {
      super.viewDidDisappear(animated)
      labLog("probe \(name) didDisappear")
    }
  }

  struct AppearProbe: UIViewControllerRepresentable {
    let name: String
    var willAppear: ((AppearProbeController) -> Void)?

    func makeUIViewController(context: Context) -> AppearProbeController { AppearProbeController() }

    func updateUIViewController(_ controller: AppearProbeController, context: Context) {
      controller.name = name
      controller.willAppear = willAppear
    }
  }

  @available(iOS 27.1, *)
  struct LabSwiftUI: View {
    @Bindable var state: LabState
    @Environment(\.horizontalSizeClass) private var width
    @State private var columns = NavigationSplitViewVisibility.all
    @State private var compact = NavigationSplitViewColumn.sidebar
    @State private var geometry = LabFold(size: .zero, fold: nil)
    private let closesOnAppear = Lab.value("--lab-back") == "appear"

    private var asideOn: Bool {
      state.asideWanted && !state.path.isEmpty && width == .regular && geometry.size.width >= 700
    }

    private var asideWidth: CGFloat {
      geometry.fold.map { geometry.size.width - $0.minX - 20 } ?? min(360, max(290, geometry.size.width * 0.36))
    }

    var body: some View {
      HStack(spacing: 0) {
        NavigationSplitView(columnVisibility: $columns, preferredCompactColumn: $compact) {
          LabSidebar(state: state)
            .background {
              AppearProbe(name: "listing") { probe in listingWillAppear(probe) }
            }
            .onAppear {
              labLog("swiftui listing onAppear width=\(String(describing: width)) path=\(state.path)")
              if closesOnAppear, width == .compact, !state.path.isEmpty {
                state.returned += 1
                state.path = []
              }
            }
            .onDisappear { labLog("swiftui listing onDisappear") }
        } detail: {
          NavigationStack(path: $state.rest) {
            Group {
              if let first = state.path.first {
                LabItem(state: state, id: first)
                  .id(first)
                  .onAppear { labLog("swiftui root \(first) onAppear") }
                  .onDisappear { labLog("swiftui root \(first) onDisappear") }
              } else {
                LabLanding()
              }
            }
            .navigationDestination(for: String.self) { LabItem(state: state, id: $0) }
          }
        }
        .navigationSplitViewStyle(.balanced)
        if asideOn {
          LabAsideStack(state: state)
            .frame(width: asideWidth)
        }
      }
      .onGeometryChange(for: LabFold.self) { proxy in
        LabFold(size: proxy.size, fold: proxy.reservedRegions(kind: .division).first { $0.isActive }?.frame)
      } action: { now in
        geometry = now
      }
      .onAppear { compact = state.path.isEmpty ? .sidebar : .detail }
      .onChange(of: state.path.isEmpty) { _, empty in compact = empty ? .sidebar : .detail }
      .onChange(of: compact) { labLog("swiftui compact column -> \(compact)") }
      .onChange(of: width) { labLog("swiftui width class -> \(String(describing: width))") }
      .onChange(of: state.path) { labLog("swiftui path -> \(state.path)") }
      .onChange(of: asideOn, initial: true) { _, on in
        labLog("swiftui aside on=\(on) \(geometry)")
        state.asideShown = on
        columns = on ? .detailOnly : .all
      }
    }

    /// The listing is about to be on screen, as UIKit says it. Alone on screen with something open, a transition
    /// from another screen that ends without being given up is the reader coming back to it.
    private func listingWillAppear(_ probe: AppearProbeController) {
      guard !closesOnAppear, probe.splitViewController?.isCollapsed ?? (width == .compact), !state.path.isEmpty
      else { return }
      guard let coordinator = probe.transitionCoordinator, coordinator.viewController(forKey: .from) != nil else {
        labLog("the listing appears with no transition from a screen: hosted again, passed over")
        return
      }
      coordinator.animate(alongsideTransition: nil) { context in
        guard !context.isCancelled else {
          labLog("the way back was given up")
          return
        }
        state.returned += 1
        state.path = []
        labLog("the reader returned to the listing, \(state.returned)")
      }
    }
  }

  // MARK: - The root

  public struct ColumnsLab: View {
    @State private var state = LabState()

    public init() {}

    public var body: some View {
      Group {
        if #available(iOS 27.1, *) {
          Group {
            if Lab.variant == "swiftui" {
              LabSwiftUI(state: state)
            } else {
              LabUIKit(state: state).ignoresSafeArea()
            }
          }
          .background { FoldProbes().ignoresSafeArea() }
        }
      }
      .overlay(alignment: .bottomTrailing) {
        Text(verbatim: state.summary)
          .font(.caption2)
          .padding(3)
          .background(Color.yellow.opacity(0.8))
          .accessibilityIdentifier("lab.state")
          .allowsHitTesting(false)
          .padding(.bottom, 70)
      }
      .onAppear { labLog("lab \(Lab.variant ?? "?") launched \(ProcessInfo.processInfo.arguments.dropFirst())") }
    }
  }
#endif
