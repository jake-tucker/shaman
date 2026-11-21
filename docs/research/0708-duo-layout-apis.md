# Which layout APIs a Duo app has, and from which iOS (#708)

Researched 2026-11-21 for the map in #706. The question is what the SDK gives the iOS app (`ios/`, SwiftUI, iOS 27.0,
iPhone only, portrait only) for laying out on an iPhone Duo, spelled as Xcode 27.2's SDK spells it, and what each piece
needs. What Apple's design guidance asks is in #707 and is not repeated here; the vertical bar APIs are in #709.

Every claim names what it rests on:

- **SDK**: declared in the installed SDK, quoted verbatim.
- **Apple docs**: stated by Apple's documentation or Human Interface Guidelines.
- **Apple talk**: said in a tech talk or WWDC session, with the time.
- **Apple engineer**: said by Apple staff in a Group Lab or on the developer forums. Looser than documentation, and
  sometimes hedged by the speaker.
- **Developer report**: something a developer measured and posted on the forums. Not Apple's word.
- **Inference**: my reasoning from the above.
- **Unverified**: I could not check it. Where it matters it is also listed under
  [What only the simulator can say](#what-only-the-simulator-can-say).

Nothing was built or run for this note, and no app code was changed.

## Answer in brief

- **The app needs no new API to be a Duo app.** Filling the display and standing bars vertically come from building
  with the iOS 27.1 SDK or later, which the app already does. Showing the listing beside a Session is
  `NavigationSplitView`, an iOS 16 API that gained no new declarations in 27.1; it collapses to a stack when closed,
  tiles on the inner display, and splits evenly at the fold when partly folded, all by itself.
- **Everything fold-specific is iOS 27.1**, declared `@available(anyAppleOS 27.1, *)` in SwiftUI and
  `API_AVAILABLE(ios(27.1), ...)` in UIKit: reserved regions, arrangement views, the hinge, and
  `contentMargins(for:)`. No Duo can run iOS 27.0, so a guard's other branch only ever runs on an ordinary iPhone.
- **The inner display is regular width and regular height, and it rotates whatever the app declares.** A portrait-only
  app stays portrait on the outer display and is laid out in landscape on the inner display when the phone is held
  that way. There is no sanctioned way to stop it and no way to stay narrow.
- **Scrolling content gets no help with the fold.** A `UICollectionView` can ask for the fold's region, as any
  `UIView` can, but nothing moves its content, and the fold adds no safe area inset and changes no trait.
- **Three things I could not settle from any source** and that need the simulator: the size classes at half the inner
  display in Split View multitasking and beside a pinned Picture in Picture video; which columns a two-column
  `NavigationSplitView` shows on the inner display in portrait; and what a `NavigationStack` path in the detail column
  does as the split view collapses and expands.

## What was read

| Source | How | Limits |
|---|---|---|
| The iOS 27.2 SDK inside Xcode 27.2 (27B52) | `grep` and `sed` over the `.swiftinterface` files and UIKit headers; doc comments pulled from SwiftUICore's `.swiftdoc` with `strings` | None. This is the build's own SDK |
| [Preparing your app for iPhone Duo][prep] and the API reference | The JSON behind each page (`developer.apple.com/tutorials/data/documentation/<path>.json`) | Pages for `ReservedRegion.Kind` and its members returned 404 at the paths I tried; the SDK covers them |
| HIG: [Designing for iPhone Duo][hig-duo], [Layout][hig-layout], [Split views][hig-split], [Multitasking][hig-multi] | The same JSON form | Layout and Multitasking say nothing Duo-specific about size classes |
| Tech talks [Prepare your app][talk-prepare], [Strike a pose][talk-pose], [Leverage multiple displays][talk-scenes], [Design][talk-design], [Raise the bar][talk-bar] | Transcripts from the pages | None |
| Group Labs [285][lab-285], [286][lab-286], [287][lab-287] | The pages carry no transcript. I downloaded the English subtitle track of each video's stream and read that | Machine captions of live speech: words are occasionally garbled, and speakers hedge |
| [A Summary of the iPhone Duo Group Lab][summary] (Apple DTS, 17 posts) | A fetch tool that answers questions about the page | The forums refuse a plain client, so I never saw the raw page. Quotes below are as the tool returned them, asked for verbatim. The thread's answers are terse rewrites of the talks, and two of them contradict each other (see [A portrait-only app](#a-portrait-only-app-on-the-inner-display)) |
| The [SwiftUI Q&A][qa-swiftui] and [UIKit Q&A][qa-uikit] forum activities | The same tool, three pages of each listing, then the threads that bear on this ticket | Many threads have no Apple reply, and for 847911 and 847933 I read only the question as the listing shows it. After some forty requests the forums answered 403, so four threads went unread: [847795][t-847795], [848000][t-848000], [836088][t-836088], [850470][t-850470] |
| [The SwiftUI cookbook for navigation][wwdc22-nav] (WWDC22) | Transcript | Predates the Duo |

SDK paths below are under
`/Applications/Xcode.app/Contents/Developer/Platforms/iPhoneOS.platform/Developer/SDKs/iPhoneOS27.2.sdk/System/Library/Frameworks/`.
The SwiftUI declarations live in `SwiftUICore.framework/Modules/SwiftUICore.swiftmodule/arm64e-apple-ios.swiftinterface`
(line numbers given as `Core:n`), which `import SwiftUI` re-exports, so app code writes `ReservedRegion`, not
`SwiftUICore::ReservedRegion`. The interface declares several types twice under `#if compiler(>=5.3) && $TildeSendable`;
I quote the first.

The SDK never names the device. The word "Duo" appears in no UIKit header, and once in SwiftUI's doc comments, on
`ContentMarginGuide.container`. The APIs are written as general ones: "reserved by another entity", "a hinge".

## Reserved regions and arrangement views

### Reserved regions in SwiftUI

**SDK**, `Core:4854`:

```swift
@available(anyAppleOS 27.1, *)
public struct ReservedRegion : Swift::Equatable, Swift::Hashable, Swift::Identifiable, Swift::Sendable {
  public var id: SwiftUICore::ReservedRegion.SwiftUICore::ID {
    get
  }
  public var kind: SwiftUICore::ReservedRegion.SwiftUICore::Kind {
    get
  }
  public var frame: CoreFoundation::CGRect {
    get
  }
  public var margins: SwiftUICore::EdgeInsets {
    get
  }
  public var isActive: Swift::Bool {
    get
  }
  ...
}
@available(anyAppleOS 27.1, *)
extension SwiftUICore::ReservedRegion {
  public struct ID : Swift::Hashable, Swift::Sendable { ... }
  public struct Kind : Swift::Hashable, Swift::Sendable {
    public static var occlusion: SwiftUICore::ReservedRegion.SwiftUICore::Kind {
      get
    }
    public static var division: SwiftUICore::ReservedRegion.SwiftUICore::Kind {
      get
    }
    ...
  }
  @frozen public struct QueryOptions : Swift::OptionSet, Swift::Equatable, Swift::Hashable {
    public let rawValue: Swift::Int
    public init(rawValue: Swift::Int)
    public static var includeInactive: SwiftUICore::ReservedRegion.SwiftUICore::QueryOptions {
      get
    }
    ...
  }
}
```

**SDK**, `Core:10337`, inside `public struct GeometryProxy`:

```swift
  @available(anyAppleOS 27.1, *)
  public func reservedRegions(kind: SwiftUICore::ReservedRegion.SwiftUICore::Kind, options: SwiftUICore::ReservedRegion.SwiftUICore::QueryOptions = [], layoutDirectionBehavior: SwiftUICore::LayoutDirectionBehavior = .mirrors) -> [SwiftUICore::ReservedRegion]
```

So the method the talks call `reservedRegions(kind:options:)` has a third parameter in SwiftUI,
`layoutDirectionBehavior:`, defaulting to `.mirrors`. `kind:` has no default; a call names `.division` or `.occlusion`.
The `CustomStringConvertible` conformances on `ReservedRegion`, its `ID` and its `Kind` are
`@available(anyAppleOS 27.2, *)` (`Core:4913`), so printing a region for a debug readout needs 27.2.

The doc comments in the SDK (`SwiftUICore.swiftmodule/arm64e-apple-ios.swiftdoc`) are one line each:

- `ReservedRegion`: "A region within a view's coordinate space that has been reserved by another entity."
- `Kind.division`: "A region where content should split into two separate regions."
- `Kind.occlusion`: "A region that is occluded by an element."
- `frame`: "The rect of the region in the view's coordinate space, including the margins."
- `margins`: "The margins included in the frame around the rect for interactive content."
- `QueryOptions.includeInactive`: "Include inactive reserved regions."

### Reserved regions in UIKit

**SDK**, `UIKit.framework/Headers/UIView.h:762`:

```objc
@interface UIView (ReservedRegion)

/// Returns the reserved regions of a given kind.
- (nonnull NSArray<UIViewReservedRegion *> *)reservedRegionsOfKind:(nonnull UIViewReservedRegionKind *)kind API_AVAILABLE(ios(27.1), tvos(27.1), visionos(27.1)) API_UNAVAILABLE(watchos) NS_REFINED_FOR_SWIFT;

/// Returns the reserved regions of a given kind and options.
- (nonnull NSArray<UIViewReservedRegion *> *)reservedRegionsOfKind:(nonnull UIViewReservedRegionKind *)kind options:(UIViewReservedRegionQueryOptions)options API_AVAILABLE(ios(27.1), tvos(27.1), visionos(27.1)) API_UNAVAILABLE(watchos) NS_REFINED_FOR_SWIFT;

@end
```

**SDK**, the Swift overlay, `UIKit.framework/Modules/UIKit.swiftmodule/arm64e-apple-ios.swiftinterface:6907`:

```swift
@available(iOS 27.1, tvOS 27.1, visionOS 27.1, *)
@available(watchOS, unavailable)
extension UIView {
  @available(iOS 27.1, tvOS 27.1, visionOS 27.1, *)
  @available(watchOS, unavailable)
  public struct ReservedRegion : Swift.Equatable, Swift.Hashable, Swift.Identifiable {
    public let id: UIView.ReservedRegion.ID
    public let kind: UIView.ReservedRegion.Kind
    public let frame: CoreFoundation.CGRect
    public let margins: UIEdgeInsets
    public let isActive: Swift.Bool
    ...
  }
  @available(iOS 27.1, tvOS 27.1, visionOS 27.1, *)
  @available(watchOS, unavailable)
  @_Concurrency.MainActor @preconcurrency public func reservedRegions(kind: UIView.ReservedRegion.Kind, options: UIView.ReservedRegion.QueryOptions = []) -> [UIView.ReservedRegion]
}
```

with `UIView.ReservedRegion.Kind` holding `public static let occlusion` and `public static let division`, and
`UIView.ReservedRegion.QueryOptions` holding `public static let includeInactive`. The Objective-C classes behind them
are `UIViewReservedRegion`, `UIViewReservedRegionIdentifier`, `UIViewReservedRegionKind` (`+occlusionRegionKind`,
`+divisionRegionKind`) and the options `UIViewReservedRegionQueryOptionsIncludeInactive`, all in
`UIKit.framework/Headers/UIViewReservedRegion.h`. The talk's "UIViewReservedRegion" is that Objective-C name; Swift
writes `UIView.ReservedRegion`.

There is no layout guide, trait or notification for a reserved region anywhere in the SDK: the only way to learn of
one is to ask a view (**SDK**, by the absence of any other declaration matching `reservedRegion`).

### What Apple says reserved regions do

All from [ReservedRegion][doc-rr] and [UIView.ReservedRegion][doc-uirr] (**Apple docs**) unless marked:

- The query "returns only the regions that overlap your view, including a partial overlap", with frames in "your
  view's own coordinates, not the screen's". A frame "can start at negative coordinates or extend past the view's
  size."
- "The frame already includes the region's margins ... so you don't need to add the margins yourself."
- The fold is a division. "When you partially fold iPhone Duo like a book, the fold runs vertically at the center of
  the display ... like a laptop, the fold runs horizontally". Tell which by comparing the frame's width and height.
  "The example uses the first division, because iPhone Duo has one fold."
- Active and inactive: "the fold of iPhone Duo becomes active when the device is partially open, and inactive again
  when it's fully open. The inner camera is active only while the camera is in use, and the outer camera is always
  active." By default only active regions come back. "An inactive region still has a frame, so you can leave room for
  it."
- **Apple talk** ([Strike a pose][talk-pose], 7:27): "When flat, it's inactive and has a width of zero."
- Occlusions include "the Dynamic Island, a camera, or window controls".
- Right to left: SwiftUI mirrors the frames by default; pass `LayoutDirectionBehavior.fixed` for unmirrored ones.
- "Most apps don't need to check for reserved regions. Framework-provided views, such as split views, tab bars,
  alerts, context menus, and sheets, already adjust for the fold and the camera."
- "Scrolling containers, such as lists, tables, scroll views, and collection views, don't split their content around
  a division by default. When the device is partially open, a row or item can run across the fold. If that makes your
  content harder to read or use, check for division regions and arrange the content yourself."
- "Check in the view that positions the content, so the frames you read use the same coordinates as the content you
  place."
- In UIKit: "UIKit tracks the regions you read in `updateConstraints()` and runs it again when they change"
  ([reservedRegions(kind:options:)][doc-uirr-method]).
- **Apple talk** ([Strike a pose][talk-pose], 6:38): in SwiftUI the proxy comes "from a GeometryReader or
  onGeometryChange modifier".

Where the sources disagree: both reference pages also say "A division is active only when the device is partially
open or closed." Every other statement, on the same pages and in the talk, says active only when partly folded. With
the phone closed the app is on the outer display, which has no fold, so nothing turns on the difference. I read
"or closed" as loose wording (**Inference**).

### A collection view hosted in SwiftUI

The conversation's list is a `UICollectionView` subclass (`FootCollectionView`, with a custom
`UICollectionViewLayout`) inside a `UIViewRepresentable` (`ios/ShamanUI/Sources/ShamanUI/ConversationList.swift`).

- It can ask. `reservedRegions(kind:options:)` is declared on `UIView`, so every subclass has it (**SDK**).
- Nothing happens unless it asks. A Frameworks Engineer, answering about compositional layouts in
  [thread 847879][t-847879] (**Apple engineer**): "There is no automatic fold avoidance to UICollectionView or
  compositional layout at this time, continuous or otherwise." And: "there are no traits or safe area insets applied by
  default by the division region." So the fold will not show up in `safeAreaInsets`, `layoutMargins` or the trait
  collection.
- Apple's advice is mostly not to ask. The same reply: "In general, we don't recommend having grids avoid the division
  regions, as long as their content is scrollable ... if you need to avoid the area of the fold, you should only do
  that in sections that aren't scrolling orthogonal to the fold's axis, by reading the reserved regions from the
  collection view and manually adjusting your compositional layout." [Strike a pose][talk-pose] (3:45, **Apple
  talk**): "Continuous scrolling content like articles, feeds, documents, and lists don't displace."
- Which way the scrolling runs against the fold matters. Lab [287][lab-287] (46:21, **Apple engineer**): "scrolling
  perpendicular to the hinge can work pretty well ... when you're scrolling is parallel to the hinge, it can be a
  little bit more difficult ... It's very difficult to reach in and actually perform a scroll right in that hinge
  area." **Inference** for the app: a conversation scrolls vertically. Seated like a laptop the fold runs
  horizontally, the conversation crosses it at a right angle, and it may pass through. Partly folded like a book the
  fold runs vertically, along the scrolling, and a full-width conversation would have a strip down its middle that is
  hard to read and to drag in. That case goes away if the conversation is a split view's detail column, since the
  split view puts each column on one side of the fold.
- When to ask again is thinly documented. The docs promise re-running only for `updateConstraints()`. A DTS Engineer in
  [thread 847876][t-847876] (**Apple engineer**) answers "when are we notified" by pointing at UIKit's automatic
  observation tracking in general and, failing that, at `UIHingeInteraction`. A developer asking in the same thread
  whether a region change is a trait or can be observed got no reply. Whether a read inside `layoutSubviews()` or a
  custom layout's `prepare()` is tracked the same way is **Unverified**.
- The SwiftUI side can ask instead and hand the answer down: a `GeometryReader` or `onGeometryChange` around the
  representable yields the regions in that view's coordinates, and `updateUIView` passes them in (**Inference** from
  the two APIs; SwiftUI re-evaluates a `GeometryReader` when its inputs change, which the docs imply by "your layout
  changes with the division" but do not state as a contract).
- Whether a view inside a `UIViewRepresentable` gets the same answers as one in a plain UIKit hierarchy is
  **Unverified**. Nothing suggests otherwise. One **Developer report** ([thread 847963][t-847963]) says the API
  returned nothing inside a keyboard extension on a 27.1 beta, which is a different process and proves nothing here.

### Arrangement views in SwiftUI

**SDK**, `Core:16710`, `Core:16735`, `Core:16739`:

```swift
@available(anyAppleOS 27.1, *)
nonisolated public struct ArrangementView<Primary, Secondary> : SwiftUICore::View, ~Swift::Sendable where Primary : SwiftUICore::View, Secondary : SwiftUICore::View {
  nonisolated public init(@SwiftUICore::ContentBuilder primary: () -> Primary, @SwiftUICore::ContentBuilder secondary: () -> Secondary)
  nonisolated public var body: some SwiftUICore::View {
    get
  }
  ...
}

extension SwiftUICore::View {
  @available(anyAppleOS 27.1, *)
  nonisolated public func arrangementViewStyle(_ style: some ArrangementViewStyle) -> some SwiftUICore::View
}

@available(anyAppleOS 27.1, *)
@preconcurrency @_Concurrency::MainActor public protocol ArrangementViewStyle {
  associatedtype Body : SwiftUICore::View
  typealias Configuration = SwiftUICore::ArrangementViewStyleConfiguration
  @SwiftUICore::ContentBuilder @_Concurrency::MainActor @preconcurrency func makeBody(configuration: Self.Configuration) -> Self.Body
}
```

The styles (**SDK**, `Core:4607`, `Core:12830`, `Core:19051`), each `@available(anyAppleOS 27.1, *)`:

```swift
extension SwiftUICore::ArrangementViewStyle where Self == SwiftUICore::SplitArrangementViewStyle {
  @export(implementation) nonisolated public static var split: SwiftUICore::SplitArrangementViewStyle {
    get { .init() }
  }
}
nonisolated public struct SplitArrangementViewStyle : SwiftUICore::ArrangementViewStyle, ~Swift::Sendable {
  nonisolated public init()
  nonisolated public func axes(_ axes: SwiftUICore::Axis.SwiftUICore::Set) -> SwiftUICore::SplitArrangementViewStyle
  ...
}

extension SwiftUICore::ArrangementViewStyle where Self == SwiftUICore::OverlayArrangementViewStyle {
  @export(implementation) nonisolated public static var overlay: SwiftUICore::OverlayArrangementViewStyle {
    get { .init() }
  }
}
nonisolated public struct OverlayArrangementViewStyle : SwiftUICore::ArrangementViewStyle, ~Swift::Sendable {
  nonisolated public init()
  nonisolated public func axes(_ axes: SwiftUICore::Axis.SwiftUICore::Set) -> SwiftUICore::OverlayArrangementViewStyle
  ...
}

extension SwiftUICore::ArrangementViewStyle where Self == SwiftUICore::AutomaticArrangementViewStyle {
  @available(anyAppleOS 27.1, *)
  @export(implementation) nonisolated public static var automatic: SwiftUICore::AutomaticArrangementViewStyle {
    get { .init() }
  }
}
```

What goes with them (**SDK**, `Core:4634`, `Core:4648`, `Core:12857`, `Core:12865`). Each member carries its own
`@available(anyAppleOS 27.1, *)` line, left out here, and the four extensions are merged into two:

```swift
extension SwiftUICore::View {
  nonisolated public func splitArrangementLayoutRatio(_ ratio: CoreFoundation::CGFloat?) -> some SwiftUICore::View
  nonisolated public func splitArrangementLayoutRatio(minHorizontal: CoreFoundation::CGFloat? = nil, idealHorizontal: CoreFoundation::CGFloat? = nil, maxHorizontal: CoreFoundation::CGFloat? = nil, minVertical: CoreFoundation::CGFloat? = nil, idealVertical: CoreFoundation::CGFloat? = nil, maxVertical: CoreFoundation::CGFloat? = nil) -> some SwiftUICore::View
  nonisolated public func splitArrangementLayoutSize(minWidth: CoreFoundation::CGFloat? = nil, idealWidth: CoreFoundation::CGFloat? = nil, maxWidth: CoreFoundation::CGFloat? = nil, minHeight: CoreFoundation::CGFloat? = nil, idealHeight: CoreFoundation::CGFloat? = nil, maxHeight: CoreFoundation::CGFloat? = nil) -> some SwiftUICore::View
  nonisolated public func splitArrangementFixedLayoutSize(horizontal: Swift::Bool = true, vertical: Swift::Bool = true) -> some SwiftUICore::View
  nonisolated public func overlayArrangementEdge(_ edge: SwiftUICore::VerticalEdge?) -> some SwiftUICore::View
  nonisolated public func overlayArrangementEdge(_ edge: SwiftUICore::HorizontalEdge?) -> some SwiftUICore::View
}
extension SwiftUICore::EnvironmentValues {
  public var splitArrangementAxis: SwiftUICore::Axis? {
    get
    set
  }
  public var overlayArrangementZIndex: Swift::Int {
    get
    set
  }
}
```

So the ticket's `.arrangementViewStyle(.split)` and `.overlay` are spelled as the talks give them. The SDK adds
`.automatic`, whose doc comment says "the arrangement view resolves to a `SplitArrangementViewStyle`", and
`.split.axes(_:)` and `.overlay.axes(_:)`. A custom style is possible through `ArrangementViewStyleConfiguration`
(`primary`, `secondary`).

### Arrangement views in UIKit

**SDK**, `UIKit.framework/Headers/UIArrangementViewController.h`:

```objc
/// A view controller that presents its container view controllers through an arrangement.
UIKIT_EXTERN
API_AVAILABLE(ios(27.1), tvos(27.1), visionos(27.1))
API_UNAVAILABLE(watchos)
NS_SWIFT_UI_ACTOR
@interface UIArrangementViewController : UIViewController

- (instancetype)init NS_DESIGNATED_INITIALIZER;
- (void)updateArrangement:(UIArrangement *)arrangement animated:(BOOL)animated NS_REFINED_FOR_SWIFT;
- (void)updateArrangement:(UIArrangement *)arrangement NS_REFINED_FOR_SWIFT;
- (nullable UIArrangementViewState *)stateForPlacement:(UIArrangementViewControllerViewPlacement)placement NS_REFINED_FOR_SWIFT;
- (nullable UIViewController *)viewControllerForPlacement:(UIArrangementViewControllerViewPlacement)placement NS_REFINED_FOR_SWIFT;
- (UIArrangementViewControllerViewPlacement)placementForViewController:(UIViewController *)viewController NS_REFINED_FOR_SWIFT;
- (void)setViewController:(nullable UIViewController *)viewController forPlacement:(UIArrangementViewControllerViewPlacement)placement animated:(BOOL)animated NS_REFINED_FOR_SWIFT;
- (void)setViewController:(nullable UIViewController *)viewController forPlacement:(UIArrangementViewControllerViewPlacement)placement NS_REFINED_FOR_SWIFT;

@end
```

(doc comments between the methods left out). In Swift (**SDK**, the UIKit overlay, line 6056):

```swift
  @_Concurrency.MainActor @preconcurrency final public func updateArrangement<A>(_ arrangement: A, animated: Swift.Bool = false) where A : UIArrangementViewController.Arrangement
  @_Concurrency.MainActor @preconcurrency final public func state(for placement: UIArrangementViewController.ViewPlacement) -> UIArrangementViewController.ViewState?
  @_Concurrency.MainActor @preconcurrency final public func viewController(for placement: UIArrangementViewController.ViewPlacement) -> UIViewController?
  @_Concurrency.MainActor @preconcurrency final public func placement(for viewController: UIViewController) -> UIArrangementViewController.ViewPlacement?
  @_Concurrency.MainActor @preconcurrency final public func setViewController(_ viewController: UIViewController?, for placement: UIArrangementViewController.ViewPlacement, animated: Swift.Bool = false)
```

with `ViewPlacement.primary` and `.secondary`, `ViewState` (`zIndex`, `splitAxis`, `isHidden`), the arrangements
`UISplitArrangement` (`.split`) and `UIOverlayArrangement` (`.overlay`), each with `axes(_:)` and per-placement view
properties, and `UIViewController.arrangementViewController`, the nearest ancestor. Headers:
`UISplitArrangement.h`, `UIOverlayArrangement.h`. The app is SwiftUI and has no use for these unless it hosts one.

### What Apple says arrangement views do

From [ArrangementView][doc-av] and [Preparing your app][prep] (**Apple docs**) unless marked:

- Split: the two views side by side "when the containing view is wider than it is tall", primary leading; primary
  above secondary when taller than wide. "The arrangement view adjusts the placement of the primary and secondary
  views to adapt to reserved regions, such as the folding region."
- "When someone is holding a closed iPhone Duo in landscape orientation, the split arrangement only shows the primary
  view."
- Overlay: primary over secondary in z-order while no division is active, "when iPhone Duo is closed or fully open";
  partly open, primary goes to "the trailing or bottom part of the display relative to the fold" and secondary to the
  leading or top part.
- With `axes(_:)` limiting the axes: "If the split arrangement cannot split among an axis, and it's the primary axis,
  the arrangement view chooses to only show a single view" (**Apple talk**, [Strike a pose][talk-pose], 12:33).
- That it shows one view alone is by design. A DTS Engineer in [thread 847800][t-847800] (**Apple engineer**):
  "unlike Stacks, `ArrangementView` is an adaptive container that collapses to displaying only one view, prioritizing
  the view with higher `.layoutPriority`, or the primary view by default whenever its split axes cannot be satisfied
  within the current aspect ratio, pose, or compact size class." The follow-up, how a person then reaches the hidden
  view, got no Apple reply. A developer in [thread 847787][t-847787] reports a split arrangement showing one view on
  an iPad as well; no reply (**Developer report**).
- Where it may stand: "avoid adding an arrangement view inside another container such as a scroll view, navigation
  split view, grid, or other container view that lays out other views. Also avoid placing some container views inside
  an arrangement view, such as a navigation split view or tab view." It goes inside a `NavigationStack`: the talk's
  own example adds "an ArrangementView inside of the stack" (11:20).
- It is not a split view. [Strike a pose][talk-pose] (8:59) introduces it for "a layout that's kind of like a split
  view, but I don't really need all the expanding and collapsing behavior", and (16:15) "ArrangementViews don't
  provide navigation infrastructure to your app."
- It is not Duo-only: the reference lists iOS, iPadOS, Mac Catalyst, macOS, tvOS, visionOS and watchOS 27.1.

### The other 27.1 layout pieces

- `ContentMarginGuide.container`, `View.contentMargins(for:edges:alignment:)` and
  `GeometryProxy.contentMargins(for:edges:)`, all `@available(anyAppleOS 27.1, *)` (**SDK**, `Core:824`, `Core:10388`,
  `Core:19943`). The doc comment is the one place the SDK names the device: "On iPhone Duo, the margin can also be
  asymmetric, applying different values on each side of the device when next to a vertical bar." Covered in #709.
- The hinge (**SDK**, `Core:16798`, `Core:24303`): `DeviceHinge` (`status`, `angle: Angle`), `DeviceHinge.Status`
  (`.closed`, `.partiallyOpen`, `.fullyOpen`), `DeviceHingeContext` (`hinge: DeviceHinge?`), and

  ```swift
  @available(anyAppleOS 27.1, *)
  extension SwiftUICore::View {
    nonisolated public func onHingeChange(isEnabled: Swift::Bool = true, _ action: @escaping (_ oldContext: SwiftUICore::DeviceHingeContext, _ newContext: SwiftUICore::DeviceHingeContext) -> Swift::Void) -> some SwiftUICore::View
  }
  ```

  UIKit has `UIHinge` and `UIHingeInteraction` (27.1). "For layout, use the arrangement and region APIs" (**Apple
  talk**, [Leverage multiple displays][talk-scenes], 2:34). The map has this out of scope.
- The bar APIs (`toolbarVerticalEdge`, `toolbarVerticalBehavior(_:)`, `axisBehavior(_:)` and the rest) are #709's.

## `NavigationSplitView` on a Duo

**The SDK adds nothing.** `NavigationSplitView` is `@available(iOS 16.0, macOS 13.0, tvOS 16.0, watchOS 9.0, *)`, its
`preferredCompactColumn` initialisers are iOS 17, and no declaration touching it, `NavigationSplitViewVisibility`,
`NavigationSplitViewColumn`, `NavigationSplitViewStyle` or `navigationSplitViewColumnWidth` carries a 27.x
availability (**SDK**, `SwiftUI.swiftinterface:27122` and around). `UISplitViewController`'s newest members are
iOS 26. Its [reference page][doc-nsv] says nothing about the Duo. Everything below is behaviour the system gives an
existing split view, and rests on talks and engineers.

| Case | What it does | Rests on |
|---|---|---|
| Closed, outer display | "columns will collapse to single-stack navigation" | **Apple talk**, [Prepare][talk-prepare] 5:19; **Apple docs**, [HIG][hig-duo]: "collapses to a single pane on the outer display, the same way it adapts between regular and compact environments on other iPhone devices" |
| Open, inner display | "columns will appear both tiled and as overlays" | **Apple talk**, [Prepare][talk-prepare] 5:19 |
| Inner display, three columns | "you get two columns at a time ... there's a button in the top left that shows you ... the root column as an overlay" | **Apple engineer**, lab [285][lab-285] 35:11 |
| Inner display, portrait | Of Mail: "you'll see ... just the detail view. And if you press the button in the top left, you'll get the sidebar as an overlay and you can pull it in and actually see those two root views next to each other." "The same is true on a ... Pro Max in landscape." | **Apple engineer**, lab [285][lab-285] 35:11. Said of Mail, which has three columns |
| Partly folded like a book | "the system keeps both columns visible by adjusting their width and positioning them into an even 50/50 split" | **Apple talk**, [Strike a pose][talk-pose] 5:40; **Apple engineer**, lab [285][lab-285] 36:31; [thread 847879][t-847879]: it does so "by adjusting column widths and layout margins within the column" |
| Half the inner display in Split View multitasking | Not stated | **Unverified**. See below |

What the table leaves open:

- **A two-column split view on the inner display in portrait.** The only statement is about Mail's three columns.
  Whether a two-column `NavigationSplitView` with the default style tiles both columns at 669 points wide, or shows the
  detail alone with the sidebar as an overlay as an iPad does in portrait, is **Unverified**. It decides whether "the
  inner display shows the listing beside the open Session" holds in portrait without
  `.navigationSplitViewStyle(.balanced)` and a `columnVisibility` of `.all` or `.doubleColumn`, both iOS 16 API.
- **The moment of the even split.** A developer in [thread 847952][t-847952] reports that on folding "the system
  detects this is the final state after some delay and un-blurs the left side ... split view controller enlarges the
  sidebar to occupy the full width of the left side." No reply (**Developer report**).
- **Split View multitasking.** A web search's summary of the forums says an Apple DTS reply states that "on compact
  widths, such as when iPhone Duo is closed or opened flat in Split View, NavigationSplitView collapses into a single
  stack and ignores column visibility". I could not open the thread it came from (the unread four above), so this is
  **Unverified**: a paraphrase by a search engine of a post I have not read.

### Collapsing, expanding and a `NavigationStack` path

No source says what a `NavigationStack(path:)` inside a split view's detail column does when the phone closes and
opens. What the sources do say:

- **Apple docs** ([NavigationSplitView][doc-nsv]): at narrow sizes "the navigation split view collapses all of its
  columns into a stack, and shows the last column that displays useful information." "The split view ignores the
  visibility control when it collapses its columns into a stack." `preferredCompactColumn` chooses which column is on
  top when collapsed, and going back changes its binding.
- **Apple docs**, same page: "You can also embed a `NavigationStack` in a column. Tapping or clicking a
  `NavigationLink` that appears in an earlier column sets the view that the stack displays over its root view.
  Activating a link in the same column adds a view to the stack."
- **Apple talk** ([The SwiftUI cookbook for navigation][wwdc22-nav]): "One super cool thing about combining List
  selection and NavigationSplitView like this, is that SwiftUI can automatically adapt the split view to a single stack
  on iPhone or in Slide Over on iPad. Changes to selection automatically translate into the appropriate pushes and pops
  on iPhone." Its last recipe is a two-column split view whose detail column holds a `NavigationStack` with a path,
  which "also automatically adapts to narrow presentations".
- **Apple engineer**, lab [286][lab-286] 17:27: "It's not like your app is getting created on one display and then
  torn down and put on another display. It's really that you're getting resized."
- **Apple engineer**, lab [286][lab-286] 19:22, the one firm warning: "one thing that I see SwiftUI apps sometimes do
  is ... they will use one container on one side of an if statement, and then they'll use a different container on the
  other side ... oftentimes that if statement is guarded by size class. So if you were doing that, you will see state
  loss." Lab [287][lab-287] 39:06 repeats it: "a big if statement at the root of your app that is switching on: if
  compact, if regular, that is going to tear down all the state in your app. So switching to the system navigation
  containers ... can be the easiest way to preserve that state."
- **Apple engineer**, [thread 847818][t-847818] (DTS): "model the navigation path independently from how the flow is
  presented across the two displays." General assurance, no mechanism.
- **Apple engineer**, lab [287][lab-287] 39:50, on what a split view shows as it expands: "when it's closed, if you're
  in your messages list and you open it, which message do you show in the detail view? ... you do have to think that
  through".
- Scroll position: "it is just a resize of your view. And so as your view resizes, we do actually anchor it to the
  visible position" for `UICollectionView` and other scroll views (**Apple engineer**, lab [287][lab-287] 55:28).

**Inference** for the app. Today `ListingScreen` is a `NavigationStack(path:)` whose root is the listing and whose path
holds the pushed Sessions. To show the listing beside a Session, the listing becomes the split view's sidebar, the open
Session a selection, and deeper screens a `NavigationStack` path in the detail column. That one `NavigationSplitView`
must then be the container on every iPhone, collapsed on an ordinary one, since choosing between it and a
`NavigationStack` by size class is exactly the pattern that loses state. What the collapsed stack then holds (the
listing, the selected Session, then the detail's path) and whether the bound path survives a round trip is
**Unverified** and is the first thing to see on the simulator.

## Size classes in every case

| Case | Width | Height | Rests on |
|---|---|---|---|
| Outer display, portrait | compact | regular | **Apple talk**, [Prepare][talk-prepare] 3:07 |
| Outer display, landscape | compact | compact | **Apple talk**, [Prepare][talk-prepare] 3:07 |
| Inner display, open flat, either orientation | regular | regular | **Apple talk**, [Prepare][talk-prepare] 3:20; **Apple engineer**, lab [286][lab-286] 53:00: "It's regular, regular, which is the first time a phone has had that" |
| Inner display, partly folded like a book or a laptop | regular | regular | **Apple docs**, [HIG][hig-duo]: "A compact width layout for the outer display and a regular width layout for the inner display give you the fundamentals for every pose"; **Apple engineer**, [thread 847879][t-847879]: the division applies "no traits". No source states the folded case by name |
| Standing like a tent | as the outer display | as the outer display | **Inference**: the app is on the outer display. A **Developer report** ([thread 857225][t-857225]) has the system move a window to the outer display when a partly open phone is set down in that pose |
| Half the inner display, Split View multitasking | not stated | not stated | **Unverified** |
| Beside a pinned Picture in Picture video | not stated | not stated | **Unverified** |

On the two unstated rows:

- **Split View multitasking.** Apple says only that "all apps participate" ([Leverage multiple
  displays][talk-scenes], 2:59), that it is an even split of two apps ([Design][talk-design], 2:56), that the app is
  resized like any other change ("Your app handles both of these layouts the same", 3:17), and that each app's bars
  stand on its outer edge. A DTS Engineer, asked whether regular width is how to tell the displays apart
  ([thread 847903][t-847903]): "don't think in terms of the specific screen, just the available space ... consider the
  multi-tasking support where two apps can be open on the inner display". **Developer reports** from the simulator put
  the inner display at 669 by 951 points and the outer at 466 by 678 ([847933][t-847933], [847911][t-847911],
  [847883][t-847883]; the pixel sizes 2007 by 2853 and 1398 by 2034 at 3x agree, and one post's 382 by 644 for the
  closed phone is that display less an 84 point bar and a 34 point home indicator). **Inference**: half of the inner
  display in landscape is about 475 by 669 points, almost the outer display's size in portrait, so compact width and
  regular height is the likely answer, and a split view would collapse there. What the split is in portrait, side by
  side at about 334 points each or stacked, no source says.
- **Picture in Picture.** "if you're in Portrait Open, there's a new control that lets you pin the video to the top of
  the screen ... you can launch another app and use it at the bottom of the screen, and the video won't occlude the
  app" (**Apple engineer**, lab [287][lab-287] 59:22). "Your current app resizes vertically to fit the space that's
  left" (**Apple talk**, [Design][talk-design], 3:21). So the width stays the inner display's and the height shrinks.
  **Inference**: width stays regular. Whether the height becomes compact is not stated. [Raise the bar][talk-bar]
  (11:39) says bars may overflow "when using Picture in Picture in open portrait", though open portrait has
  horizontal bars, which I cannot reconcile without seeing it.

Other things said about size classes:

- The idiom is `.phone` in every pose, and there is no Duo trait (**Apple engineer**, [thread 847945][t-847945]:
  "its device idiom is still `.phone`"; summary thread: "Does iPhone Duo have its own UIUserInterfaceIdiom? No.").
- Size classes do not tell outer portrait from outer landscape by width. The summary thread suggests
  `toolbarVerticalEdge` as the signal for "bars are vertical" (**Apple engineer**); lab [286][lab-286] 58:09 says "on
  the outer display, the size classes are different in the two axes" (**Apple engineer**).
- Do not override them: "If an app overrides horizontalSizeClass directly, does that block the system's size-class
  updates as the device folds? ... Yes, it would very likely interfere" (**Apple engineer**, summary thread).
- Regular width on a phone is new for more than the split view. Developers report sheets standing inset and tab bars
  turning into sidebars on the inner display ([UIKit Q&A][qa-uikit]). The question whether an action sheet or share
  sheet needs a popover anchor at regular width on a phone ([thread 847957][t-847957]) has no reply. That is
  #719's ground.

## A portrait-only app on the inner display

The app declares `UISupportedInterfaceOrientations` as `UIInterfaceOrientationPortrait` alone and is built with the
27.2 SDK.

What the sources say, in the order of their weight:

1. **Apple talk**, [Prepare][talk-prepare] 3:20 to 3:57: "The interface orientation on the outer display behaves like
   any other iPhone ... Interface orientation on the inner display behaves differently. The inner display doesn't honor
   your supported interface orientations."
2. **Apple talk**, the same, 4:37 to 4:53: "iPhone Duo will continue to honor the UIRequiresFullScreen key, but your
   app will still resize when someone opens or closes their iPhone Duo. iPhone Duo respects your supported interface
   orientations, but your app will scale on the inner display, including in Split View multitasking."
3. **Apple engineer**, lab [285][lab-285] 26:46 to 28:38, asked exactly this ("suppose an app locks its supported
   orientations to portrait only. Does it still get the compact regular size class changes ... or does it block
   adaptive layout completely?"): "on the outer display, we will honor supported orientations, but on the inner
   display, we won't. Yeah, and so it will adapt. And the size class will be regular, regular on the, on the inside
   ... As soon as you go from a portrait aspect ratio to square ... where do you draw the line? ... when you support
   Resizability, we try to try our best to fill the screen".
4. **Apple engineer**, the summary thread, "Will portrait-only apps adapt well to all iPhone Duo poses?": "Partially
   ... If an app locks its supported orientations to portrait only, it still receives compact and regular size class
   changes as iPhone Duo folds and unfolds. On the outer display, the system honors supported orientations, but on the
   inner display, it adapts and provides regular-regular size classes, ignoring orientation restrictions."
5. The summary thread, "Is there a sanctioned way to lock the inner display to portrait only?": "No."
6. **Apple engineer**, a Frameworks Engineer in [thread 847836][t-847836], to an app wanting to stay half-width when
   built with the 27.1 SDK: "No you cannot. The expectation going forward is that all apps, including iPhone-only
   ones, will be fully adaptive and adjust to any available scene size at runtime."
7. **Apple engineer**, lab [285][lab-285] 45:20: "although we don't respect the supported orientations on the Inner
   display. We do provide the current orientation in the environment and as a trait."

Where they disagree. Statements 1 and 2 come thirty seconds apart in one talk and read as opposites, and the summary
thread carries both: "the inner display doesn't honor your supported interface orientations" in three answers, and
"Yes, iPhone Duo still honors UISupportedInterfaceOrientations, but the expected behavior for this device differs from
previous iPhone devices in certain hinge states and orientations" in another. The thread's own gloss of statement 2 is
"iPhone Duo respects your supported orientations, but your app still scales via size classes on the inner display".

How I read it (**Inference**): "respects" is true of the device as a whole because the outer display honours the
list; "scale" means resize, not a zoomed or letterboxed picture. Statements 3 to 7 leave little room for any other
reading. The letterboxed, centred, fixed-size presentation exists, but it is what an app built with the iOS 26 SDK or
earlier gets, not one built with 27.1 ([thread 847887][t-847887], summary thread).

So the app as it stands should get:

- Closed, portrait: the outer display, compact by regular, bar on the camera's side.
- Closed, turned to landscape: it stays portrait, as on any iPhone, since the outer display honours the list.
  **Unverified** how that looks on a display "wider and shorter" than an iPhone's.
- Open, portrait: the whole inner display, regular by regular, horizontal bars.
- Open, landscape: the whole inner display in landscape, regular by regular, vertical bars, although the app never
  asked for landscape. This is the part that changes the plan: the inner display's landscape layout is not optional.
- Partly folded, and in Split View multitasking: resized like any app.

All of that is the reading of talks and engineers, none of it a declaration. What a portrait-only build really shows
when the open phone is turned is **Unverified** and belongs to the simulator.

Related: Apple's advice is to stop being portrait-only. "Your app should support every orientation where possible"
(**Apple engineer**, [thread 847945][t-847945]); "If your app supports multitasking, the system requires it to support
all orientations" (summary thread); "a lot of phone apps that have been locked to portrait have only worried about top
and bottom safe areas" (lab [285][lab-285] 54:43).

## The deployment target

What needs iOS 27.1 at run time, and what does not:

| Wanted | Needs | Rests on |
|---|---|---|
| Filling the display, vertical bars | Building with the iOS 27.1 SDK or later. Not a deployment target | **Apple talk**, [Prepare][talk-prepare] 1:02 |
| Listing beside a Session, even split at the fold | `NavigationSplitView`, iOS 16 | **SDK** |
| Size classes, safe areas, layout margins | Nothing new | **SDK** |
| Reserved regions, `ArrangementView`, `onHingeChange`, `contentMargins(for:)`, the vertical bar modifiers | iOS 27.1 at run time | **SDK**: `@available(anyAppleOS 27.1, *)` |
| `description` of a `ReservedRegion` | iOS 27.2 | **SDK**, `Core:4913` |

- A Duo never runs iOS 27.0: "when iPhone Duo becomes available on October 23rd, it will run iOS 27.1" (Apple's host,
  opening each Group Lab). So `if #available(iOS 27.1, *)` is true on every Duo, and the `else` branch is reached only
  by an ordinary iPhone still on iOS 27.0.x (**Inference**).
- Apple's own advice for an app with older targets is the guard: "gate iOS 27.1 APIs (ArrangementView,
  ReservedRegion, onHingeChange, etc.) behind availability checks" (**Apple engineer**, summary thread). "Both
  ArrangementView and UIArrangementViewController are available on iOS 27.1 and later. These APIs are unavailable on
  iOS 27.0 or earlier." And for what stands in the other branch, lab [286][lab-286] 59:54: "For ArrangementViews
  specifically, if you're using an existing layout, then, you know, maintain using that existing layout. And then
  perhaps when ... ArrangementViews are available on those operating systems, switch to using that."
- In app code the guard is `if #available(iOS 27.1, *)` and `@available(iOS 27.1, *)`. `anyAppleOS` is how the SDK's
  own interface spells it, built with the experimental feature `AnyAppleOSAvailability` (**SDK**, the interface's
  `swift-module-flags` line). Whether app code may write `anyAppleOS` without that flag is **Unverified**.

**What guarding costs** (**Inference**, from how Swift availability works and how the app is built):

- Each use needs a branch. A type such as `ArrangementView` goes inside `if #available` in a view builder; a modifier
  such as `contentMargins(for:)` needs a small wrapper that applies it or not; a helper that takes or returns
  `ReservedRegion` must itself be marked `@available(iOS 27.1, *)`, so a region passed down to the conversation's list
  travels as plain `CGRect`s or behind such a mark.
- Every guard has an `else` that nothing tests. `ios/AGENTS.md` pins tests to iOS 27.2 simulators and CI selects
  Xcode 27.2, so no run ever takes a 27.0 branch. The branches would be code that looks dead, is not, and is unproven.
- The app has no availability checks today (`grep` finds none under `ios/`), so this would be a new habit, kept for
  one point release.

**What raising the target to 27.1 costs** (**Inference**):

- The app stops installing on an iPhone running iOS 27.0.x. It is signed for its author's own phones, and a Duo is
  27.1 or later by construction, so the cost is updating any other phone it runs on.
- Four places hold the number and move together: `IPHONEOS_DEPLOYMENT_TARGET` in
  `ios/Shaman.xcodeproj/project.xcproj`, `platforms: [.iOS("27.0")]` in `ios/ShamanUI/Package.swift`, and both the
  exported `IPHONEOS_DEPLOYMENT_TARGET` and the generated package's `platforms` in `scripts/build-ios-core.sh`.
- Nothing in tests or CI changes: the simulators are 27.2 and the SDK's default deployment target is 27.2.
- Raising it further, to 27.2, would also free `ReservedRegion`'s `description` and `toolbarTitleAlignment`, at the
  price of every phone needing 27.2.

The choice only arises if the spec uses a 27.1 API at all. A design made of a split view chosen by the system, size
classes and safe areas uses none, and the target can stay.

## What only the simulator can say

For #712 and the prototypes. None of these has an answer in any source I could read.

1. What the portrait-only build shows when the open phone is turned to landscape, and when the closed phone is.
2. The size classes at half the inner display in Split View multitasking, in each orientation, and whether the split is
   side by side or stacked in portrait.
3. The size classes, and the bars, beside a pinned Picture in Picture video in open portrait.
4. Which columns a two-column `NavigationSplitView` shows on the inner display in portrait and in landscape, with the
   default style and with `.balanced`.
5. What a `NavigationStack` path in the detail column, and the sidebar's selection, hold after closing and opening the
   phone; and what `preferredCompactColumn` reads through it.
6. Whether a `UICollectionView` inside a `UIViewRepresentable` gets the fold's region from
   `reservedRegions(kind: .division)`, in whose coordinates, and whether reading it in `layoutSubviews()` or a custom
   layout's `prepare()` is run again when the fold becomes active.
7. That the fold adds no safe area inset and changes no trait, as the Frameworks Engineer says.
8. The points: inner display 669 by 951, outer 466 by 678, a vertical bar's inset 84, all from developers' posts.

## Found along the way

Not this ticket's question; noted for the tickets they belong to.

- **Posing the simulator (#710).** `xcrun simctl list devicetypes` on this machine lists
  `iPhone Duo (com.apple.CoreSimulator.SimDeviceType.iPhone-Duo)`. `xcrun simctl help` lists nothing for fold, hinge or
  pose; `xcrun devicectl device` lists `appResize` ("Manage resizable app sessions") and `orientation`. Developers
  report no supported way to fold the simulator from a terminal or XCUITest ([847883][t-847883], no Apple reply), that
  `XCUIDevice.shared.orientation` does nothing on the Duo simulator, and that `devicectl device appResize` works as a
  stand-in with caveats, with XCUITest then reporting stale frames ([SwiftUI Q&A][qa-swiftui], threads 847892, 847884,
  847880; none answered). Lab [287][lab-287] 31:49, asked the same: "file Feedback".
- **A regression in 27.2 (#712).** A developer reports that built with Xcode 27.2, the top safe area on the open
  phone in landscape is 82 points although the bar stands vertically, where 27.1 gave none
  ([thread 858338][t-858338]). The app builds with 27.2.
- **Safe areas are late during a turn (#717).** In a transition's `alongsideTransition` block the view's frame is
  already the new one while `safeAreaInsets` are still the old, for about a tenth of a second
  ([thread 847911][t-847911], a **Developer report** with no reply). The conversation's list follows its foot through
  inset changes.
- **The keyboard (not yet specified on the map).** Nothing in the labs. Two forum questions, listed in the
  [UIKit Q&A][qa-uikit], whose threads I did not open: an `inputAccessoryView` collapsing vertical bar items it does
  not overlap (847973), and `keyboardLayoutGuide` against an inset sheet on the inner display (847946, no reply).

## Sources

Apple documentation and guidelines:

- [Preparing your app for iPhone Duo][prep]
- [ReservedRegion][doc-rr], [reservedRegions(kind:options:layoutDirectionBehavior:)][doc-rr-method],
  [UIView.ReservedRegion][doc-uirr], [reservedRegions(kind:options:)][doc-uirr-method]
- [ArrangementView][doc-av], [UIArrangementViewController][doc-uiavc]
- [NavigationSplitView][doc-nsv]
- HIG: [Designing for iPhone Duo][hig-duo], [Layout][hig-layout], [Split views][hig-split],
  [Multitasking][hig-multi]

Talks:

- [Prepare your app for iPhone Duo][talk-prepare]
- [Strike a pose with adaptive layouts on iPhone Duo][talk-pose]
- [Leverage multiple displays and scenes on iPhone Duo][talk-scenes]
- [Design for iPhone Duo][talk-design]
- [Raise the bar with iPhone Duo][talk-bar]
- [The SwiftUI cookbook for navigation][wwdc22-nav], WWDC22
- iPhone Duo Group Labs [285][lab-285], [286][lab-286], [287][lab-287]

Forums:

- [A Summary of the iPhone Duo Group Lab][summary]
- [iPhone Duo SwiftUI Q&A][qa-swiftui], [iPhone Duo UIKit Q&A][qa-uikit]
- Threads with an Apple reply used above: [847879][t-847879] (collection views and the fold),
  [847876][t-847876] (when regions change), [847800][t-847800] (an arrangement showing one view),
  [847836][t-847836] (staying narrow), [847945][t-847945] (iPhone only, regular width),
  [847903][t-847903] (telling the displays apart), [847818][t-847818] (navigation path),
  [847887][t-847887] (an app built with an older SDK), [847842][t-847842] (nothing moves list items)
- Threads without one: [847952][t-847952], [847787][t-847787], [847883][t-847883], [847963][t-847963],
  [857225][t-857225], [858338][t-858338]
- Not read, the forums having begun to refuse: [847795][t-847795], [848000][t-848000], [836088][t-836088],
  [850470][t-850470]

[prep]: https://developer.apple.com/documentation/technologyoverviews/preparing-your-app-for-iphone-duo
[doc-rr]: https://developer.apple.com/documentation/swiftui/reservedregion
[doc-rr-method]: https://developer.apple.com/documentation/swiftui/geometryproxy/reservedregions(kind:options:layoutdirectionbehavior:)
[doc-uirr]: https://developer.apple.com/documentation/uikit/uiview/reservedregion
[doc-uirr-method]: https://developer.apple.com/documentation/uikit/uiview/reservedregions(kind:options:)
[doc-av]: https://developer.apple.com/documentation/swiftui/arrangementview
[doc-uiavc]: https://developer.apple.com/documentation/uikit/uiarrangementviewcontroller
[doc-nsv]: https://developer.apple.com/documentation/swiftui/navigationsplitview
[hig-duo]: https://developer.apple.com/design/human-interface-guidelines/designing-for-iphone-duo
[hig-layout]: https://developer.apple.com/design/human-interface-guidelines/layout
[hig-split]: https://developer.apple.com/design/human-interface-guidelines/split-views
[hig-multi]: https://developer.apple.com/design/human-interface-guidelines/multitasking
[talk-prepare]: https://developer.apple.com/videos/play/tech-talks/111461/
[talk-bar]: https://developer.apple.com/videos/play/tech-talks/111462/
[talk-pose]: https://developer.apple.com/videos/play/tech-talks/111463/
[talk-scenes]: https://developer.apple.com/videos/play/tech-talks/111464/
[talk-design]: https://developer.apple.com/videos/play/tech-talks/111466/
[wwdc22-nav]: https://developer.apple.com/videos/play/wwdc2022/10054/
[lab-285]: https://developer.apple.com/videos/play/meet-with-apple/285/
[lab-286]: https://developer.apple.com/videos/play/meet-with-apple/286/
[lab-287]: https://developer.apple.com/videos/play/meet-with-apple/287/
[summary]: https://developer.apple.com/forums/thread/847644
[qa-swiftui]: https://developer.apple.com/forums/activities/1664080
[qa-uikit]: https://developer.apple.com/forums/activities/1670080
[t-847911]: https://developer.apple.com/forums/thread/847911
[t-847933]: https://developer.apple.com/forums/thread/847933
[t-847957]: https://developer.apple.com/forums/thread/847957
[t-847787]: https://developer.apple.com/forums/thread/847787
[t-847795]: https://developer.apple.com/forums/thread/847795
[t-847800]: https://developer.apple.com/forums/thread/847800
[t-847818]: https://developer.apple.com/forums/thread/847818
[t-847836]: https://developer.apple.com/forums/thread/847836
[t-847842]: https://developer.apple.com/forums/thread/847842
[t-847876]: https://developer.apple.com/forums/thread/847876
[t-847879]: https://developer.apple.com/forums/thread/847879
[t-847883]: https://developer.apple.com/forums/thread/847883
[t-847887]: https://developer.apple.com/forums/thread/847887
[t-847903]: https://developer.apple.com/forums/thread/847903
[t-847945]: https://developer.apple.com/forums/thread/847945
[t-847952]: https://developer.apple.com/forums/thread/847952
[t-847963]: https://developer.apple.com/forums/thread/847963
[t-848000]: https://developer.apple.com/forums/thread/848000
[t-836088]: https://developer.apple.com/forums/thread/836088
[t-850470]: https://developer.apple.com/forums/thread/850470
[t-857225]: https://developer.apple.com/forums/thread/857225
[t-858338]: https://developer.apple.com/forums/thread/858338
