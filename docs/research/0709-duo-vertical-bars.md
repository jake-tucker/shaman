# Which vertical bar APIs a Duo app has (#709)

Researched 2026-11-21 for the map in #706, against Xcode 27.2 (27B52) and its iOS 27.2 SDK, with the app at
`63e373f8`. What Apple's design guidance asks is digested in #707 and is not repeated here.

Every claim names what it rests on:

- **SDK**: declared in the installed SDK, or written in the doc comment the SDK ships beside the declaration.
- **Doc**: stated by Apple's documentation.
- **Talk**: stated in "Raise the bar with iPhone Duo", with the time it is said.
- **Xcode skill**: stated by the `app-resizability` or `swiftui-whats-new-27` skill Xcode 27.2 exports.
- **Lab**: said by an Apple engineer or designer in an iPhone Duo Group Lab, with the lab and the time.
- **Forum**: written by Apple staff on the developer forums.
- **Inference**: my reading of the above, which no source states.
- **Simulator**: no source answers it, and it has to be seen running. Nothing was built or run for this note.

## Answer in brief

Every name the talk gives exists in the SDK, spelled as the talk's code spells it and not always as its transcript
says it. The SDK adds one the talk leaves out, `ContentMarginGuide`, and renames one WWDC26 showed,
`toolbarMinimizationBehavior(_:for:)`.

The system decides an item's axis from what the item is. An item with a symbol goes into the vertical bar. An item
that is text alone, or a custom view, stays in a horizontal bar. That rule sorts the app's bars cleanly: its three
buttons with symbols go vertical, and its principal items, its status item and its two text-only sheet buttons do
not. What a horizontal bar holding only those leftovers looks like beside a vertical bar, and what a `.searchable`
drawer, an inline navigation title and a `safeAreaInset(edge: .top)` banner do there, no source says.

The vertical bar reaches content as a safe area inset on one horizontal edge, with nothing on the other. Layout
margins become asymmetric too, and SwiftUI has a new API to read them. Five of the names need iOS 27.1 and the app
targets 27.0.

## How each source was read

| Source | How |
|---|---|
| SwiftUI declarations | `SwiftUI.framework/Modules/SwiftUI.swiftmodule/arm64e-apple-ios.swiftinterface` and the same file under `SwiftUICore.framework`, in `$(xcrun --sdk iphoneos --show-sdk-path)/System/Library/Frameworks` |
| SwiftUI doc comments | `strings` over the `arm64e-apple-ios.swiftdoc` beside each interface file; the interface files carry none |
| UIKit declarations | `UIKit.framework/Headers`: `UIBarButtonItem.h`, `UINavigationItem.h`, `UIViewController.h`, `UIVerticalBarEdge.h`, `UIViewLayoutRegion.h`, `UIBarMinimization.h` |
| [Preparing your app for iPhone Duo][prep] and the API reference | The JSON behind each page, at `developer.apple.com/tutorials/data/documentation/<path>.json`. The reference pages repeat the SDK's doc comments word for word |
| [Raise the bar with iPhone Duo][talk] | The page's code tab and transcript |
| [What's new in SwiftUI][wwdc269] (WWDC26) | The page's code tab and transcript. It predates the Duo and never names it |
| Group Labs [285][lab285], [286][lab286], [287][lab287] | The pages carry no transcript. Read from each video's English subtitle track |
| [iPhone Duo SwiftUI Q&A][qa] and [A Summary of the iPhone Duo Group Lab][labsummary] | Through a page-fetching tool, because the forums answer a plain HTTP client with a human check. Quotes are as that tool relayed them |
| Xcode's agent skills | `xcrun agent skills export --output-dir <dir>`, then `app-resizability/references/safe-area-task.md` and `swiftui-whats-new-27/references/toolbar.md` |

## The names

### As the SDK declares them

All **SDK**. Each declaration and the `@available` line above it is verbatim; the trailing comments are mine and
stand for a member's own `@available` lines. `anyAppleOS 27.1` means iOS 27.1 and its siblings. SwiftUI's file
writes `SwiftUI.Name` and SwiftUICore's `SwiftUICore::Name`.

```swift
// SwiftUI.ToolbarItemPlacement
public static let cancellationAction: SwiftUI.ToolbarItemPlacement          // the struct's own iOS 14.0
@available(iOS 27.0, visionOS 27.0, *)
public static let topBarPinnedTrailing: SwiftUI.ToolbarItemPlacement

@available(anyAppleOS 27.1, *)
extension SwiftUI.ToolbarContent {
  nonisolated public func axisBehavior(_ behavior: SwiftUI.ToolbarItemAxisBehavior) -> some SwiftUI.ToolbarContent
}
@available(anyAppleOS 27.1, *)
public struct ToolbarItemAxisBehavior : Swift.Hashable, Swift.Sendable {
  public static let automatic: SwiftUI.ToolbarItemAxisBehavior
  public static let horizontalOnly: SwiftUI.ToolbarItemAxisBehavior       // iOS only
  public static let verticalPreferred: SwiftUI.ToolbarItemAxisBehavior    // iOS only
}

@available(iOS 27.0, macOS 26.1, watchOS 27.0, tvOS 27.0, visionOS 27.0, *)
extension SwiftUI.ToolbarContent {
  @_Concurrency.MainActor @preconcurrency public func visibilityPriority(_ priority: SwiftUI.ToolbarItemVisibilityPriority) -> some SwiftUI.ToolbarContent
}
@available(iOS 27.0, macOS 26.1, watchOS 27.0, tvOS 27.0, visionOS 27.0, *)
public struct ToolbarItemVisibilityPriority : Swift.Hashable, Swift.Sendable {
  public static let automatic: SwiftUI.ToolbarItemVisibilityPriority
  public static let low: SwiftUI.ToolbarItemVisibilityPriority            // iOS 27.0, macOS 26.1
  public static let high: SwiftUI.ToolbarItemVisibilityPriority           // iOS 27.0, macOS 26.1
  public init(lowerThan other: SwiftUI.ToolbarItemVisibilityPriority)     // iOS 27.0, macOS 27.0
  public init(higherThan other: SwiftUI.ToolbarItemVisibilityPriority)    // iOS 27.0, macOS 27.0
}

@available(anyAppleOS 27.1, *)
extension SwiftUICore.View {
  nonisolated public func toolbarVerticalCompressionBehavior(_ behavior: SwiftUI.ToolbarVerticalCompressionBehavior) -> some SwiftUICore.View
}
@available(anyAppleOS 27.1, *)
public struct ToolbarVerticalCompressionBehavior : Swift.Hashable, Swift.Sendable {
  public static let automatic: SwiftUI.ToolbarVerticalCompressionBehavior
  public static let prefersToolbarItems: SwiftUI.ToolbarVerticalCompressionBehavior   // iOS 27.1 only
  public static let prefersTabBar: SwiftUI.ToolbarVerticalCompressionBehavior         // iOS 27.1 only
}

@available(iOS 27.0, visionOS 27.0, *)
nonisolated public struct ToolbarOverflowMenu<Content> : SwiftUI.ToolbarContent, SwiftUI.CustomizableToolbarContent where Content : SwiftUICore.View {
  nonisolated public init(@SwiftUICore.ContentBuilder content: () -> Content)
}
extension SwiftUICore.View {
  @available(iOS 27.0, visionOS 27.0, *)
  nonisolated public func toolbarOverflowMenu<C>(@SwiftUICore.ContentBuilder content: () -> C) -> some SwiftUICore.View where C : SwiftUICore.View
}

@available(anyAppleOS 27.1, *)
extension SwiftUICore.View {
  nonisolated public func toolbarVerticalBehavior(_ behavior: SwiftUI.ToolbarVerticalBehavior) -> some SwiftUICore.View
}
@available(anyAppleOS 27.1, *)
public struct ToolbarVerticalBehavior : Swift.Hashable, Swift.Sendable {
  public static let automatic: SwiftUI.ToolbarVerticalBehavior
  public static let disabled: SwiftUI.ToolbarVerticalBehavior
}

// SwiftUICore
@available(anyAppleOS 27.1, *)
extension SwiftUICore::EnvironmentValues {
  public var toolbarVerticalEdge: SwiftUICore::HorizontalEdge? {
    get
  }
}
```

Beside the names the ticket lists, the same SDK declares these, which bear on a vertical bar:

```swift
// Layout margins, which are asymmetric beside a vertical bar. SwiftUICore.
@available(anyAppleOS 27.1, *)
public struct ContentMarginGuide { }
@available(anyAppleOS 27.1, *)
extension SwiftUICore::ContentMarginGuide {
  public static var container: SwiftUICore::ContentMarginGuide { get }
}
@available(anyAppleOS 27.1, *)
extension SwiftUICore::View {
  nonisolated public func contentMargins(for guide: SwiftUICore::ContentMarginGuide, edges: SwiftUICore::Edge.SwiftUICore::Set = .all, alignment: SwiftUICore::Alignment? = nil) -> some SwiftUICore::View
}
@available(anyAppleOS 27.1, *)
extension SwiftUICore::GeometryProxy {
  public func contentMargins(for guide: SwiftUICore::ContentMarginGuide, edges: SwiftUICore::Edge.SwiftUICore::Set = .all) -> SwiftUICore::EdgeInsets
}

// Where a sheet stands, which decides whether its bar is vertical on the inner display. SwiftUI.
@available(anyAppleOS 27.0, *)
extension SwiftUICore.View {
  nonisolated public func presentationPlacement(_ placement: SwiftUI.PresentationPlacement) -> some SwiftUICore.View
}
@available(anyAppleOS 27.0, *)
public struct PresentationPlacement : Swift.Hashable, Swift.Sendable {
  public static let automatic: SwiftUI.PresentationPlacement
  public static let leading: SwiftUI.PresentationPlacement     // iOS only
  public static let center: SwiftUI.PresentationPlacement      // iOS only
  public static let trailing: SwiftUI.PresentationPlacement    // iOS only
}

// A navigation bar that scrolls away. SwiftUI.
@available(anyAppleOS 27.0, *)
extension SwiftUICore.View {
  nonisolated public func toolbarMinimizationBehavior(_ behavior: SwiftUI.ToolbarMinimizationBehavior, for bars: SwiftUI.ToolbarPlacement...) -> some SwiftUICore.View
  nonisolated public func toolbarMinimizationSafeAreaAdjustment(_ adjustment: SwiftUI.ToolbarMinimizationSafeAreaAdjustment, for bars: SwiftUI.ToolbarPlacement...) -> some SwiftUICore.View
  nonisolated public func toolbarMinimizationRestoration(_ restoration: SwiftUI.ToolbarMinimizationRestoration, for bars: SwiftUI.ToolbarPlacement...) -> some SwiftUICore.View
}
// ToolbarMinimizationBehavior: automatic, onScrollDown, onScrollUp, never

// A toolbar item drawn without its margins. SwiftUI.
@available(anyAppleOS 27.0, *)
extension SwiftUI.ToolbarContent {
  nonisolated public func contentMarginsRemoved(_ removed: Swift.Bool = true) -> some SwiftUI.ToolbarContent
}
```

The UIKit side, for reading a talk or a forum answer that names it (**SDK**, all `API_AVAILABLE(ios(27.1))` unless
marked):

| UIKit | SwiftUI counterpart |
|---|---|
| `UIBarButtonItem.axisBehavior`, `UIBarButtonItem.AxisBehavior` `.automatic` `.horizontalOnly` `.verticalPreferred` | `axisBehavior(_:)` |
| `UIBarButtonItem.visibilityPriority` (27.0), `UIBarButtonItemVisibilityPriority` `.standard` `.low` `.high` | `visibilityPriority(_:)`, where `.standard` is `.automatic` |
| `UINavigationItem.verticalBarCompressionBehavior`, `UIVerticalBarCompressionBehavior` `.automatic` `.prefersBarItems` `.prefersTabBar` | `toolbarVerticalCompressionBehavior(_:)`, where `.prefersBarItems` is `.prefersToolbarItems` |
| `UIViewController.preferredVerticalBarBehavior`, `UIVerticalBarBehavior` `.automatic` `.disabled`, `childForPreferredVerticalBarBehavior`, `setNeedsUpdateOfVerticalBarConfiguration()` | `toolbarVerticalBehavior(_:)` |
| `UITraitCollection.verticalBarEdge`, `UIVerticalBarEdge` `.unspecified` `.leading` `.trailing`, `UITraitCollection.systemTraitsAffectingVerticalBarEdge` | `toolbarVerticalEdge`, where `.unspecified` is `nil` |
| `UINavigationItem.pinnedTrailingGroup` (16.0) | `.topBarPinnedTrailing` |
| `UINavigationItem.additionalOverflowItems` (16.0) | `ToolbarOverflowMenu` |
| `UISheetPresentationController.preferredPlacement` (27.0) | `presentationPlacement(_:)` |
| `UINavigationItem.navigationBarMinimization`, `UIBarMinimization` (27.0) | `toolbarMinimizationBehavior(_:for:)` |
| `UIView.LayoutRegion.bar(onEdge:extent:)` (27.1) and `.bar(onEdge:)` (27.2) | None. The SwiftUI interface declares no bar region; `ReservedRegion.Kind` has `occlusion` and `division` alone |

### Where the talk, the transcripts and the SDK disagree

| Said | Where | The SDK's spelling |
|---|---|---|
| "the toolbarCompressionBehavior API" | Talk transcript, 12:14 | `toolbarVerticalCompressionBehavior(_:)`, as the talk's code has it |
| "leftItemSupplementsBackButton" | Talk transcript, 5:05 | `leftItemsSupplementBackButton`, as the talk's code has it |
| "the preferredPlacement API" for sheets | Talk transcript, 4:09 | That is UIKit's. SwiftUI's is `presentationPlacement(_:)` |
| "a new AxisBehavior API", "horizontal-only", "vertical-preferred" | Talk transcript, 8:00 to 8:52 | `axisBehavior(_:)` with `ToolbarItemAxisBehavior.horizontalOnly` and `.verticalPreferred` |
| `.prefersBarItems` | Talk code, 12:23, UIKit half | SwiftUI's case is `.prefersToolbarItems` |
| `toolbarMinimizeBehavior(.onScrollDown, for: .navigationBar)` | WWDC26 "What's new in SwiftUI" code, 7:37, and Xcode 27.2's own `swiftui-whats-new-27` skill | `toolbarMinimizationBehavior(_:for:)`. The older spelling appears nowhere in the 27.2 interface. A Frameworks Engineer on the forums gives the new one ([thread 847863][f-minimize]) |
| A pinned trailing item "never moves to the overflow menu" | Xcode's `swiftui-whats-new-27` skill | The doc comment says "Pinned items only move to the overflow menu when search is active and there isn't enough room" |

### What each one does

- **`axisBehavior(_:)`.** `.automatic`: "The system infers the supported axes based on the content type of the
  item. Image items support both axes; text and custom view items support only the horizontal axis." `.horizontalOnly`:
  "If an item only supports horizontal bars and no horizontal bars are present, the item is not shown."
  `.verticalPreferred`: the item "prefers a vertical placement when both horizontal and vertical bars are present."
  (**SDK**) Setting `.verticalPreferred` does nothing on a phone with no vertical bar, so it needs no condition
  (**Lab** 287, 31:19).
- **`visibilityPriority(_:)`.** "When toolbar space is limited, items with a lower priority move into the overflow
  menu before items with a higher priority." (**SDK**) With none set, "items overflow from bottom to top" (**Talk**
  13:23). It also takes a `ToolbarItemGroup` (**Xcode skill**).
- **`ToolbarOverflowMenu`.** Holds "actions that are always placed in the toolbar's overflow menu"; on iOS "this
  content is placed into the overflow menu in the navigation bar." (**SDK**) The ellipsis is kept for that menu (**Talk** 12:50).
  A confirmation dialog cannot be anchored to an item in it from SwiftUI; an Apple evangelist answers only that the
  dialog is driven by shared state ([thread 847865][f-overflow], **Forum**).
- **`.topBarPinnedTrailing`.** For "prominent actions" such as Done, which stand just under Back or Close
  (**Talk** 5:23, **Doc**).
- **`.cancellationAction`.** For "custom back or close buttons", which stand at the top of the bar (**Talk** 4:55,
  **Doc**).
- **`toolbarVerticalCompressionBehavior(_:)`.** "When the tab bar and navigation/toolbar items are both rendered
  together in the vertical bar, this property controls which items compress first." (**SDK**, UIKit's header, of its
  counterpart) The app has no tab bar, so it has no use for it (**Inference**).
- **`toolbarVerticalBehavior(.disabled)`.** "Bar content falls back to the standard horizontal top and bottom bars,
  and the vertical status bar reverts to the horizontal axis." It is "a stable choice: avoid changing it frequently as
  the user navigates, and don't toggle it for a single view as a function of that view's state." Where it is read
  from: "The behavior is resolved by the window or presentation if placed within a sheet or form", and a
  `NavigationStack` "uses the top most view of its stack of views", a `NavigationSplitView` "the view in the
  trailing-most column". (**SDK**) So one pushed screen can turn the bar off for the whole window while it is on top
  (**Inference**).
- **`toolbarVerticalEdge`.** It "reflects the system's preferred edge for the vertical bar in the current context,
  regardless of whether a vertical bar is currently visible", and is `nil` "on devices and in contexts where the
  system never places a vertical bar — for example hardware without a vertical bar, or a size class or orientation in
  which no vertical bar is used." (**SDK**) It does not say a bar is showing: a screen that disabled the bar still reads an edge
  (**Inference** from that sentence).

## Which bars go vertical

**Only bars a navigation container owns.** `NavigationStack`, `NavigationSplitView` and `TabView` in SwiftUI
(**Doc**, **Talk** 2:28, **Lab** 287 28:29). Every bar in the app is one of these: the app declares no custom bar.

**Which display and orientation.** The outer display, in portrait and landscape, and the inner display in landscape.
The inner display in portrait keeps horizontal bars (**Talk** 0:57 to 1:15). The app's Info settings allow portrait
alone, and the inner display does not honour that (#707), so today's build has a vertical bar on the outer display
and on the inner display turned to landscape, and horizontal bars on the inner display in portrait (**Inference**).

**Which side.** The side the camera is on, so it can be the left: "the vertical bar always aligns with where the
camera is, so it can be on the left", and in Split View multitasking "the left-hand app will have a left-hand bar"
(**Lab** 285, 13:10 to 13:44). "Because the bar is aligned with the hardware, it stays on the same side of the device
in right-to-left languages" (**Talk** 4:18). `toolbarVerticalEdge` is a leading or trailing edge, which follows
layout direction (**Xcode skill**), so the same physical side reads `.trailing` in English and `.leading` in Arabic
(**Inference**; it squares the talk with the doc comment's "the system determines the edge based on locale and
device").

| Container | Outer display | Inner display, landscape | Inner display, portrait | Rests on |
|---|---|---|---|---|
| A `NavigationStack` alone | Vertical | Vertical | Horizontal | **Talk** 0:57 to 1:15 |
| A `NavigationSplitView` showing several columns | Does not arise: it collapses to one column (#707) | Vertical for the detail column, horizontal for the sidebar and content columns | Horizontal | **Doc**: "the system shows bars horizontally for the sidebar or content view, and vertically for the detail view". **Talk** 3:36 |
| A `NavigationSplitView` collapsed to one column | **Simulator**. The doc's rule is for "a split view displaying multiple views", which suggests a collapsed one behaves as a stack | n/a | n/a | **Inference** |
| An inspector | Presents as a sheet | Horizontal: it gets no vertical bar of its own | Horizontal | **Doc**, **Talk** 3:47; the sheet is from the lab summary, **Forum** |
| A sheet with a toolbar | Vertical "by default" | Horizontal when centred, which is the default, or leading. Vertical when trailing | Horizontal | **Doc**, **Talk** 3:58 to 4:16 |
| A sheet with a toolbar and `toolbarVerticalBehavior(.disabled)` | Horizontal bars, and the sheet stops short of the camera (#707) | Horizontal, as it already was | Horizontal | **Doc**, **SDK**; the inner display rows are **Inference** |
| A sheet with no toolbar | **Simulator**. The talk says only "if your sheets already have a toolbar, it's displayed vertically" | Horizontal | Horizontal | **Talk** 3:58 |

A sheet has an inset of its own: "A sheet carries its own inset for a vertical bar. Its content is inset even where
the presenting view is not" (**Xcode skill**). A sheet's bar stays as it is when the outer display turns: "whether
your sheet has or does not have controls laid out to the side, the controls kind of stay with the sheet" (**Lab**
286, 46:54).

The bar's height changes often: turning the outer display, a pinned Picture in Picture video, Split View
multitasking and the keyboard all shorten it, and items overflow when they do (**Talk** 11:39, **Lab** 286 50:49).

## What becomes of an item

1. An item with a symbol goes into the vertical bar and shows the symbol. An item with a title and no symbol is not
   presented vertically. An item that is a custom view is not presented vertically. (**Doc**, **SDK**, **Talk** 6:52
   to 7:13)
2. An item's title is still wanted when it has a symbol: an overflow menu shows both (**Doc**, **Talk** 7:13).
3. A custom view may ask for the vertical bar with `.axisBehavior(.verticalPreferred)`, and must then fit "the
   bar's fixed width" (**Talk** 8:52, 10:06). The system holds a custom view back "because it doesn't know how wide
   it needs to be" (**Lab** 287, 30:43).
4. An item that shows a symbol and a count inline is a custom view. The talk's answer is a badge: "rather than
   keeping the count inline, I can use a badge on the inbox to turn an item that displays both text and a symbol into
   a symbol-only item" (**Talk** 8:59), with `.badge(_:)` on the item's content (**Talk** code, 9:27).
5. Order in the bar, from the top: Back or Close, prominent actions, then the rest in their groups "with a vertical
   spacer visually separating top and bottom placements" (**Talk** 4:36 to 5:31). The stack's own Back button is
   placed for the app (**Doc**).
6. Flexible spacers have no size in a vertical bar and fixed spacers keep theirs (**Talk** 11:07).
7. A keyboard's accessory bar stays with the keyboard (**Talk** 10:00).

**What no source says** (**Simulator**, each of them):

- Where the horizontal bar holding the left-behind items stands beside a vertical bar, how tall it is, and whether
  it is there at all when nothing is left for it.
- Whether an item the system infers to be horizontal is dropped, as a `.horizontalOnly` item is, when no horizontal
  bar is present.
- What becomes of an inline `navigationTitle` with no principal item.
- What a `.searchable` field in the navigation bar drawer does. No talk, document, lab or forum answer mentions a
  search field beside a vertical bar. The nearest fact is the pinned placement's doc comment, that pinned items
  overflow only "when search is active and there isn't enough room" (**SDK**), which says search takes room from the
  items.
- Whether a horizontal navigation bar scrolls away. An engineer says "on iPhone Duo, by default, the bars do actually
  scroll away when, I believe, you're closed and also maybe in landscape", and that
  `toolbarMinimizationBehavior(_:for:)` turns it off (**Lab** 287, 57:26, hedged as quoted). The SDK's doc comment
  for `.automatic` names only one case: "navigation bars on iOS will minimize when the view has a searchable using
  the `SearchFieldPlacement/toolbarPrincipal` placement." The same engineers add that a view anchored to its foot may
  want the bar to leave in the other direction, as Messages does (**Lab** 287, 58:01), which is the conversation's
  case.

## The app's own bars, screen by screen

Line numbers are in `ios/ShamanUI/Sources/ShamanUI/` at `63e373f8`. "Horizontal" means the item stays in a
horizontal bar under the automatic axis behaviour, by rule 1 above; what that bar looks like is the first open
question above.

### Screens on the listing's stack

| Screen | Item | What it is | What the sources say becomes of it | Rests on |
|---|---|---|---|---|
| Listing, `ListingScreen.swift:88` | `.principal` scope menu | A `Menu` whose label is text and a chevron: a custom view | Horizontal | **SDK**, **Doc** |
| Listing, `:73` | `.searchable`, `.navigationBarDrawer(displayMode: .always)` | A search field under the title | Not said | **Simulator** |
| Listing, `:247` | `.bottomBar` "Servers & device" | A `Button` with a `Label`: title and `server.rack` | Vertical, as its symbol, in the lower group under the spacer | **Doc**, **Talk** 5:31 |
| Listing, `:255` | `.status` "Begins in" | Two lines of text in a `VStack`: a custom view | Horizontal. The other two bottom items leave for the vertical bar, so it is the only thing left for a bottom bar. Whether that bar stands, or the item is dropped | **SDK** for the axis; **Simulator** for the rest |
| Listing, `:269` | `.bottomBar` "New session" | A `Button` with a `Label`: title and `square.and.pencil` | Vertical, as its symbol, last in the bar. Items overflow "from bottom to top", so it is the first to go unless given a priority, and the talk names Compose among "the last to move into overflow" | **Doc**, **Talk** 13:23 and 13:54 |
| Listing, `:78` and `NoticeBanner.swift:94` | `safeAreaInset(edge: .top)` failure and notice banners | Content inset under the top bar | See "Content inset against a bar" | **Simulator** |
| Session, `SessionScreen.swift:72` | `.principal` Session header | Title over place, text in a `VStack`: a custom view. It has no menu at this commit | Horizontal | **SDK**, **Doc** |
| Session, `:76` | `.topBarTrailing` tree button | A `Button` whose label is an `HStack` of a symbol and, while Subagents work, a count: a custom view, with an accessibility label and no title | The count makes it the talk's symbol-and-text case, to be a symbol with a badge. Whether today's label is taken for an image item when the count is absent | **Talk** 8:59 for the remedy; **Simulator** for today's behaviour |
| Session | Back | The stack's own | Top of the vertical bar | **Doc**, **Talk** 4:55 |
| Session, `:30` | `safeAreaInset(edge: .bottom)` composer and foot | Content inset over the bottom edge | See below | **Simulator** |
| Session, `:39` | `safeAreaInset(edge: .top)` banner, and the notice banner | Content inset under the top bar | See below | **Simulator** |
| Session, `:144` | `ignoresSafeArea(.container, edges: .top)` on the conversation | Names one edge | Xcode's skill faults only a bare `ignoresSafeArea()`, which "extends content under every edge, including one that holds a vertical bar" | **Xcode skill** |
| New session, `NewSessionScreen.swift:60` | `.principal` title over place | A custom view | Horizontal | **SDK**, **Doc** |
| New session, `:43`, `:48` | Bottom composer inset, top banner inset | As the Session's | See below | **Simulator** |
| Workspace Picker, `WorkspacePickerScreen.swift:60`, `:62` | Inline `navigationTitle` and `.searchable` drawer, always shown | A title and a search field, no items of its own | Not said | **Simulator** |
| Workspace Picker, `:52` | `safeAreaInset(edge: .top)` banner | | See below | **Simulator** |
| Turn's work and Activity detail, `WorkScreen.swift:54`, `:179` | Inline `navigationTitle` alone | A title and the stack's Back | Back goes to the top of the vertical bar. The title: not said | **Doc**; **Simulator** |

### Sheets

On the outer display a sheet's bar stands vertically by default, and on the inner display a sheet is centred with
horizontal bars (**Doc**, **Talk**). Each row below is the outer display's case.

| Sheet | Item | What it is | What the sources say becomes of it | Rests on |
|---|---|---|---|---|
| Relay login, `RelayLoginSheet.swift:110` | `.cancellationAction` "Cancel" | A text-only button | Horizontal: "If your item has a title and doesn't have an icon, the system doesn't present it vertically." With a symbol it would stand at the top of the sheet's vertical bar. A sheet "with only one item, like the close button" is the talk's case for turning the vertical bar off | **Doc**, **Talk** 4:55 and 14:37 |
| Questionnaire, `QuestionnaireSheet.swift:54` | `.cancellationAction` decline, `role: .destructive` | A text-only button whose words the core writes | Horizontal, as above | **Doc** |
| Questionnaire, `:51` | Inline `navigationTitle`, the Question's position | A title | Not said | **Simulator** |
| Tree, `TreeSheet.swift:102` | `.principal` title over a count | A custom view. The sheet has no buttons | Horizontal. Whether a sheet whose bar holds nothing vertical still gives up a side strip | **SDK**; **Simulator** |
| Tree, `:90` | `safeAreaInset(edge: .top)` banner | | See below | **Simulator** |
| Servers & device, `DeviceScreen.swift:25`, `:57` | A `NavigationStack` with a title and no items; pushes Pairing, Scan and Pair | Titles, and Back once pushed | Back goes to the top of a vertical bar. The rest: not said | **Doc**; **Simulator** |
| Servers & device, `:58`, and Pairing, `PairingScreen.swift:139` | `safeAreaInset(edge: .top)` banners | | See below | **Simulator** |
| Scan, `CodeScanner.swift:160` | `ignoresSafeArea(edges: .bottom)` | Names one edge | As the conversation's | **Xcode skill** |
| Approval, `SessionScreen.swift:157` | No `NavigationStack`, no toolbar; detents medium and large | A sheet with no bar | Not said for a sheet with no toolbar | **Simulator** |
| Unreachable, `ListingScreen.swift:438`, `:448` | No `NavigationStack`; a top banner inset; medium detent | A sheet with no bar | Not said | **Simulator** |

Pairing also stands as the root screen of a phone holding no Remote (`PairingScreen.swift:21`), where it is a
`NavigationStack` alone and takes the first table's rules.

No source says how a sheet at a medium detent stands on the outer display. The lab summary's answer to that
question restates the defaults and no more ([summary][labsummary], **Forum**).

## Content inset against a bar

**The bar is a safe area inset on one horizontal edge.** Turning the bar off animates "the leading or trailing safe
area inset for the vertical bar" away (**SDK**). "Where a vertical bar is present, one horizontal edge usually
carries the whole inset and the other carries zero", and it "is also much larger than the horizontal insets of
earlier devices" (**Xcode skill**). It is there in portrait: "It is not a landscape condition" (**Xcode skill**). No
source gives its width.

**Scrolling content runs under the bar and is reachable.** "The right side of your view will have a safe area inset
which will automatically become a content inset on the scroll view", and content should not be clipped by hand
(Frameworks Engineer, [thread 847821][f-scroll], **Forum**). The bar has no scroll edge effect by default and gains a
background under Reduce Transparency (**Talk** 10:52), "a little bit narrower than the safe area for the vertical
bar" (**Lab** 285, 30:04).

**`safeAreaInset(edge: .top)` and `safeAreaInset(edge: .bottom)`.** No source says what their content does beside a
vertical bar. What can be said:

- The banner and the composer are laid out inside the safe area, and the bar is a safe area inset on the side, so
  they should stand beside the bar and not under it (**Inference**). Whether a background drawn by the foot reaches
  under the bar is **Simulator**.
- The top inset may have nothing above it. A developer reports that "on the Duo, when folded or opened in landscape
  mode, the top safe area is zero" ([thread 847792][f-top], unanswered, not Apple). The status bar and Dynamic Island
  are in the strip (#707), so that is plausible where no horizontal bar is left. On the app's screens a principal
  item or a title may keep a horizontal bar above the banner; whether it does is the open question of the last
  section. **Simulator.**
- Xcode's skill prefers `safeAreaBar(edge:)` "for bar content" and `safeAreaInset(edge:)` "for other content beside
  the inset region". Of `safeAreaInset` it says: "It places the content the same way, and it gives up two things: the
  bar appearance, and the extended scroll edge effect on any scroll view the inset affects." (**Xcode skill**)
  `safeAreaBar` is iOS 26.0, and has a `HorizontalEdge` form as well as the `VerticalEdge` one (**SDK**). The
  composer is the app's one piece of bar content (**Inference**).
- Inside an `ArrangementView`, a `safeAreaBar` belongs on each of the two views (DTS Engineer,
  [thread 847790][f-safeareabar], **Forum**).

**Layout margins differ on the bar's side.** `ContentMarginGuide.container` "vends the system layout margins, which
range from 16 to 20 points on iPhone", and "On iPhone Duo, the margin can also be asymmetric, applying different
values on each side of the device when next to a vertical bar. Apply or read the guide instead of writing these
values into your layout." (**SDK**) It is applied with `.contentMargins(for: .container, edges: .horizontal)` or read
from a `GeometryProxy`, and is iOS 27.1. An engineer calls it new: "there is a new API for that ... a modifier ...
that'll apply the margins for you ... SwiftUI hadn't had that before" (**Lab** 286, 39:57 to 40:31). No source gives the
values on either side. The app writes its horizontal padding as numbers (`.padding(.horizontal, 14)` on a
conversation row, `.padding(.horizontal)` on a banner), which keeps clear of the bar, since the safe area does that,
and does not follow the margin (**Inference**).

**In SwiftUI the insets are directional.** "`EdgeInsets` is directional. `leading` means leading and flips with
layout direction", a `GeometryProxy`'s `size` "is already inset", and an inset should not be read through a
`GeometryReader` to decide layout (**Xcode skill**).

## What needs iOS 27.1

`axisBehavior(_:)`, `toolbarVerticalBehavior(_:)`, `toolbarVerticalCompressionBehavior(_:)`, `toolbarVerticalEdge`
and `ContentMarginGuide` are `anyAppleOS 27.1`. `visibilityPriority(_:)`, `ToolbarOverflowMenu`,
`.topBarPinnedTrailing`, `presentationPlacement(_:)` and `toolbarMinimizationBehavior(_:for:)` are 27.0 (**SDK**).
The app and its package target 27.0, so the first five need `if #available(iOS 27.1, *)` or a raised target. A
toolbar builder takes `if #available` (**Xcode skill**). A Duo runs 27.1 or later, so the 27.0 branch only ever runs
on a phone with no vertical bar (**Inference**).

## What I could not verify

- Everything marked **Simulator** above. The largest are the horizontal bar left for principal items and titles, the
  `.status` item, the `.searchable` drawer, the top banner inset, and the sheets.
- The width of the bar's inset and the margin values. Apple publishes neither.
- Forum text. The forums refuse a plain HTTP client, so each quote is as a page-fetching tool relayed it. The lab
  summary thread is long, and two of its answers disagree on right-to-left: one says the bar is on "the trailing edge
  (or leading depending on language direction)", another that it stays "on the corresponding side". The talk says the
  second.
- Whether a pinned Picture in Picture video gives the inner display in portrait a vertical bar. The talk's "when
  using Picture in Picture in open portrait" (11:39) suggests the bar is there to be shortened; nothing says so
  outright.
- Whether bars scroll away by default on a Duo. One engineer says so with "I believe" and "maybe"; the SDK's doc
  comment does not.

## Questions this raised that are not tickets

- **Should the navigation bar scroll away on a Duo, and in which direction for a conversation anchored to its foot?**
  The lab says it does by default on a closed Duo and that Messages reverses the direction.
  `toolbarMinimizationBehavior(_:for:)` decides it. #717 covers the conversation and does not name this.
- **Should the horizontal padding the app writes as numbers give way to `contentMargins(for: .container)`?** The
  margins are asymmetric beside the bar and the app's are not.
- **Should the composer's foot be a `safeAreaBar`?** Xcode's skill says bar content belongs in one, for the scroll
  edge effect.

## Sources

- [Preparing your app for iPhone Duo][prep], Apple documentation
- SwiftUI reference: [axisBehavior(_:)][d-axis], [ToolbarItemAxisBehavior][d-axisbehavior],
  [visibilityPriority(_:)][d-priority], [ToolbarOverflowMenu][d-overflow],
  [toolbarVerticalBehavior(_:)][d-vertical], [toolbarVerticalCompressionBehavior(_:)][d-compression],
  [toolbarVerticalEdge][d-edge], [topBarPinnedTrailing][d-pinned], [presentationPlacement(_:)][d-placement],
  [ContentMarginGuide.container][d-margins]
- [Raise the bar with iPhone Duo][talk], tech talk
- [What's new in SwiftUI][wwdc269], WWDC26
- iPhone Duo Group Lab: [285][lab285], [286][lab286], [287][lab287]
- [iPhone Duo SwiftUI Q&A][qa], three pages of threads, and [A Summary of the iPhone Duo Group Lab][labsummary]
- Forum threads: [Horizontal scroll below tab bar][f-scroll], [UINavigationBar hiding on scroll][f-minimize],
  [Confirmation dialog anchor for a ToolbarOverflowMenu item][f-overflow], [safeAreaBar and the Fold][f-safeareabar],
  [Custom navigation buttons][f-custom], [Top alignment][f-top]
- The iOS 27.2 SDK in Xcode 27.2 (27B52), and the skills `xcrun agent skills export` writes

[prep]: https://developer.apple.com/documentation/technologyoverviews/preparing-your-app-for-iphone-duo
[talk]: https://developer.apple.com/videos/play/tech-talks/111462/
[wwdc269]: https://developer.apple.com/videos/play/wwdc2026/269/
[lab285]: https://developer.apple.com/videos/play/meet-with-apple/285/
[lab286]: https://developer.apple.com/videos/play/meet-with-apple/286/
[lab287]: https://developer.apple.com/videos/play/meet-with-apple/287/
[qa]: https://developer.apple.com/forums/activities/1664080
[labsummary]: https://developer.apple.com/forums/thread/847644
[f-scroll]: https://developer.apple.com/forums/thread/847821
[f-minimize]: https://developer.apple.com/forums/thread/847863
[f-overflow]: https://developer.apple.com/forums/thread/847865
[f-safeareabar]: https://developer.apple.com/forums/thread/847790
[f-custom]: https://developer.apple.com/forums/thread/847798
[f-top]: https://developer.apple.com/forums/thread/847792
[d-axis]: https://developer.apple.com/documentation/swiftui/toolbarcontent/axisbehavior(_:)
[d-axisbehavior]: https://developer.apple.com/documentation/swiftui/toolbaritemaxisbehavior
[d-priority]: https://developer.apple.com/documentation/swiftui/toolbarcontent/visibilitypriority(_:)
[d-overflow]: https://developer.apple.com/documentation/swiftui/toolbaroverflowmenu
[d-vertical]: https://developer.apple.com/documentation/swiftui/view/toolbarverticalbehavior(_:)
[d-compression]: https://developer.apple.com/documentation/swiftui/view/toolbarverticalcompressionbehavior(_:)
[d-edge]: https://developer.apple.com/documentation/swiftui/environmentvalues/toolbarverticaledge
[d-pinned]: https://developer.apple.com/documentation/swiftui/toolbaritemplacement/topbarpinnedtrailing
[d-placement]: https://developer.apple.com/documentation/swiftui/view/presentationplacement(_:)
[d-margins]: https://developer.apple.com/documentation/swiftui/contentmarginguide/container
