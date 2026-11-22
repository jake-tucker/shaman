# What a build can ask to learn that the home indicator is not kept on screen

Researched 2026-11-22 for [What a build can ask to learn that the home indicator is not kept on screen][t728], a
ticket of [Map: iPhone Duo support in the iOS app][map], against Xcode 27.2 (27B52) and its iOS 27.2 SDK, with the
app at `91317a49`. The question comes from [How the conversation and composer stand on a Duo][t717], which stands the
composer's foot on the display's bottom edge on a Duo and keeps the band beneath it on an ordinary iPhone.

Every claim names what it rests on:

- **SDK**: declared in the installed SDK, or written in the doc comment the SDK ships beside the declaration.
- **Doc**: stated by Apple's documentation or Human Interface Guidelines.
- **Talk**: said in a tech talk, with the time.
- **Xcode skill**: stated by the `app-resizability` skill Xcode 27.2 exports.
- **Lab**: said by an Apple engineer or designer in an iPhone Duo Group Lab, with the lab and the time.
- **Forum**: written by Apple staff on the developer forums.
- **Developer report**: something a developer posted on the forums. Not Apple's word.
- **Simulator**: measured on the simulators by an earlier ticket of this map, and read from its findings. Not run for
  this note.
- **Phone**: what the user saw on their own iPhone Duo, as the earlier tickets record it.
- **Inference**: my reading of the above, which no source states.
- **Unverified**: no source answers it. Where it matters it is listed under
  [What I could not verify](#what-i-could-not-verify-and-what-to-run).

Nothing was built or run for this note, no simulator was booted or posed, and no app code was changed.

## Answer in brief

**Nothing in the SDK says so.** No trait, no environment value, no property of a scene, a window or a view, and no
notification reports whether the home indicator is drawn, hidden, or kept. Every public name that speaks of the
indicator is a preference the app tells the system: `prefersHomeIndicatorAutoHidden` in UIKit and
`persistentSystemOverlays(_:)` in SwiftUI. Neither can be read back as the system's answer, and the three releases
that brought the Duo, iOS 27.0, 27.1 and 27.2, added nothing on the subject.

**Apple's guidance does not name the case either.** No document, guideline, talk, lab or forum answer says that a
Duo does not keep its home indicator, and none speaks of content in the bottom safe area of a Duo. The one mention
of the indicator on a Duo treats it as present: "Drag your app using the home indicator at the bottom". Apple's
staff do say, in so many words, that "there is no API to detect whether your app is running on iPhone Duo" and that
an app should "not introduce behavior that is specific to the device".

**The most honest signal left is the vertical bar's edge**, `toolbarVerticalEdge` in the environment (iOS 27.1): the
foot stands on the bottom edge where it is not `nil`, and keeps its band where it is. It is the system's own word
on where it has stood its chrome in this window, it is what Apple's staff point custom bars at, it names no device,
and where it is wrong it is wrong toward today's layout. Its limits are plain and are set out below: it says
nothing about the indicator, the link between the two is one phone's behaviour, and it is `nil` on the inner display
in portrait and in the laptop pose, so the band stays there.

## How each source was read

| Source | How |
|---|---|
| SwiftUI and UIKit declarations | `grep` over `SwiftUI`, `SwiftUICore` and `UIKit`'s `arm64-apple-ios-simulator.swiftinterface` and every header in `UIKit.framework/Headers`, under `$(xcrun --sdk iphonesimulator --show-sdk-path)/System/Library/Frameworks`. Then every public framework in the SDK for "home indicator" and "home affordance" |
| SwiftUI doc comments | `strings` over the `.swiftdoc` beside each interface file |
| [Preparing your app for iPhone Duo][prep], the API reference and the guidelines | The JSON behind each page, at `developer.apple.com/tutorials/data/...` |
| The six tech talks | Each page's transcript |
| Group Labs [285][lab285], [286][lab286], [287][lab287] | The pages carry no transcript. Read from each video's English subtitle track, searched for the indicator, the bottom edge and detection |
| [A Summary of the iPhone Duo Group Lab][labsummary], the [SwiftUI Q&A][qa-swiftui] and [UIKit Q&A][qa-uikit] | Through a page-fetching tool, because the forums refuse a plain client. Quotes are as that tool relayed them, asked for verbatim. Three pages of each listing; the UIKit one has a fourth I did not read |
| Xcode's agent skills | `xcrun agent skills export`, then `app-resizability/references/safe-area-task.md` |
| The iPhone User Guide, [Learn gestures to navigate iPhone][guide] | The same fetching tool |
| The map's own measurements | [How today's build behaves on the Duo simulator][t712]'s [findings][f712] and the probe beside them; the contact sheets of that ticket and of [How the conversation and composer stand on a Duo][t717] |

## What the SDK says of the home indicator

All **SDK**. This is every public declaration in the iOS 27.2 SDK that names the indicator.

```objc
// UIKit.framework/Headers/UIViewController.h:681
@interface UIViewController (UIHomeIndicatorAutoHidden)

// Override to return a child view controller or nil. If non-nil, that view controller's home indicator auto-hiding will be used. If nil, self is used. Whenever the return value changes, -setNeedsHomeIndicatorAutoHiddenUpdate should be called.
@property (nonatomic, readonly, nullable) UIViewController *childViewControllerForHomeIndicatorAutoHidden API_AVAILABLE(ios(11.0)) API_UNAVAILABLE(tvos) API_UNAVAILABLE(watchos);

// Controls the application's preferred home indicator auto-hiding when this view controller is shown.
@property (nonatomic, readonly) BOOL prefersHomeIndicatorAutoHidden API_AVAILABLE(ios(11.0)) API_UNAVAILABLE(tvos) API_UNAVAILABLE(watchos);

// This should be called whenever the return values for the view controller's home indicator auto-hiding have changed.
- (void)setNeedsUpdateOfHomeIndicatorAutoHidden API_AVAILABLE(ios(11.0)) API_UNAVAILABLE(tvos) API_UNAVAILABLE(watchos);

@end
```

```swift
// SwiftUI.swiftinterface:28195, UIHostingController, which overrides the two properties
@_Concurrency.MainActor @preconcurrency @objc override dynamic open var prefersHomeIndicatorAutoHidden: Swift.Bool {
  @objc get
}
@_Concurrency.MainActor @preconcurrency @objc override dynamic open var childForHomeIndicatorAutoHidden: UIKit.UIViewController? {
  @objc get
}

// SwiftUI.swiftinterface:12267
extension SwiftUICore.View {
  @available(iOS 16.0, macOS 13.0, tvOS 16.0, watchOS 9.0, *)
  nonisolated public func persistentSystemOverlays(_ visibility: SwiftUICore.Visibility) -> some SwiftUICore.View
}
extension SwiftUI.Scene {
  @available(macOS 15.0, iOS 18.0, tvOS 18.0, watchOS 11.0, visionOS 2.0, *)
  nonisolated public func persistentSystemOverlays(_ preferredVisibility: SwiftUICore.Visibility) -> some SwiftUI.Scene
}
```

A third form of `persistentSystemOverlays(_:)`, on `CompositorContent`, is unavailable on iOS.

What is not there, each checked against the same files (**SDK**):

- **No trait.** `UITraitCollection` gained three properties across the Duo's releases:
  `systemPrefersReducedResourceUsage` (`ios(27.0)`), `verticalBarEdge` (`ios(27.1)`) and `navigationTitleAlignment`
  (`ios(27.2)`). None of the twenty-odd trait definitions in the headers concerns the indicator or system overlays.
- **No environment value.** The environment values added in 27.x are `isTabViewSidebarAvailable` (`anyAppleOS 27.0`),
  `toolbarVerticalEdge`, `splitArrangementAxis` and `overlayArrangementZIndex` (`anyAppleOS 27.1`), and
  `toolbarTitleAlignment` (`anyAppleOS 27.2`). `persistentSystemOverlays` has a modifier and no environment key.
- **Nothing on the scene or the window.** `UIWindowScene` gained `displayLink(target:selector:)`,
  `closureConfirmation`, a delegate method for supported orientations (all `ios(27.0)`) and a session role for the
  camera capture accessory (`ios(27.1)`). Its geometry, windowing behaviours and status bar manager say nothing of the
  indicator.
- **No safe area region for it.** `SafeAreaRegions` is `container`, `keyboard` and `all`, as it has been since iOS
  14. The indicator's band is part of the container's bottom inset with nothing to tell it from the rest.
- **No layout region for it.** `UIView.LayoutRegion` has `safeArea`, `margins`, `readableContent` and, from 27.1, a
  bar on a given edge. None is the indicator's.
- **Nothing in any other public framework.** Outside UIKit and SwiftUI the words appear only in the lists of exported
  symbols, as private names (`_UIHomeAffordanceHostSceneComponent` and the like). UIKit knows of the indicator inside;
  it tells an app nothing, and a private name is not something a build may ask.

The whole of what iOS 27.1 and 27.2 added to the three frameworks is short enough to list, and it is the fold, the
hinge, arrangement views, reserved regions, content margins, the vertical bar and the title's alignment. Both earlier
notes carry the declarations: [0708][f708] and [0709][f709].

## What `prefersHomeIndicatorAutoHidden` and `persistentSystemOverlays` do

Both are a way to tell and not to ask.

- `prefersHomeIndicatorAutoHidden` is a read-only property a view controller overrides. It returns "`true` if your
  view controller lets the system determine when to hide the indicator, or `false` if you want the indicator to show
  at all times. The default implementation of this method returns `false`." And: "The system takes your preference
  into account, but returning `true` is no guarantee that the indicator will be hidden." (**Doc**,
  [prefersHomeIndicatorAutoHidden][d-prefers])
- Reading it gives back what the app said. On a `UIHostingController` it is "A Boolean value that indicates whether
  the view controller prefers the home indicator to be hidden or shown" (**SDK**), which is the app's own
  `persistentSystemOverlays` preference handed to UIKit (**Inference**). Nothing reports what the system then did.
- `persistentSystemOverlays(_:)` "Sets the preferred visibility of the non-transient system views overlaying the
  app." "The Home indicator doesn't appear without specific user intent when you set visibility to `hidden`." "You can
  indicate a preference with this modifier, but the system might or might not be able to honor that preference."
  (**SDK**, **Doc**, [persistentSystemOverlays(_:)][d-overlays])
- **On a Duo, no source says what either does.** The app sets neither today, so by the documentation it asks for the
  indicator "to show at all times", and on the user's phone it is not kept all the same (**Phone**). So a Duo already
  goes beyond what the documentation describes, and Apple has written nothing about it (**Inference**).
- **Telling instead of asking does not fit.** The app could ask every iPhone to hide the indicator and stand the foot
  on the edge everywhere. On an ordinary iPhone that changes what is drawn, which the map rules out, and the
  indicator comes back on "specific user intent", which by the same documentation is a touch near it: where the
  composer is (**Inference**).

## What Apple's guidance says of the bottom of a Duo

It does not name this case. What it does say:

- **The indicator is there on the inner display.** "Using Device Hub, preview your app on the inner display. Drag your
  app using the home indicator at the bottom to one side of the screen." (**Talk**, [Prepare your app][talk-prepare]
  7:50) Of Split View multitasking: "It's built on the iPhone home gesture ... Just drag an app to the side to create
  a split view." (**Talk**, [Design][talk-design] 2:47) So on a Duo the bottom edge carries the swipe home and the
  drag that makes a Split View, and Apple speaks of an indicator to drag. That squares with the phone, where it
  "does show up briefly sometimes" (**Phone**), and says nothing of when.
- **Nothing lists the indicator among what moves to the side.** "Controls on the side include both system and app
  elements: the Dynamic Island, the status bar, the toolbar (including navigation buttons), and the tab bar."
  (**Doc**, [Designing for iPhone Duo][hig-duo]) So no source supports the idea that the indicator is drawn in the
  strip.
- **Controls belong inside the safe area.** "When positioning your UI, make sure to place foreground elements like
  interactive controls within the safe area", while "background content may extend past the safe area" (**Talk**,
  [Prepare your app][talk-prepare] 6:37, 8:59). The guideline's page says the same in general terms and has no
  sentence on the bottom edge (**Doc**). The composer on the display's edge is the app's own choice, made on the
  phone, and not something Apple offers.
- **The one sanctioned way out of the safe area is reserved regions.** "In iOS 27.1, we're introducing a new API that
  lets your custom UI use as much of the available screen space as possible without colliding with system-provided
  UI ... use ReservedRegion to safely position UI elements outside the safe area while maximizing usable space ...
  This is a great option for building your own custom bars or when implementing edge-to-edge UI." (**Talk**,
  [Prepare your app][talk-prepare] 8:08) See the next section for why it does not answer here.
- **An app is not to know it is on a Duo.** "There is no API to detect whether your app is running on iPhone Duo ...
  An iPhone Duo app is still an iPhone app, and other than avoiding placing interactive UI on reserved regions, the
  recommendation would be to not introduce behavior that is specific to the device." (Technology Evangelist,
  [thread 847775][f-detect], **Forum**) The lab summary: "developers should rely on standard adaptive layout
  principles rather than hardcoding device-specific checks" (**Forum**). Xcode's skill lists "`UIDevice` model checks
  gating layout decisions" and "`safeAreaInsets.bottom > 0` used as a proxy for display geometry" among what to
  remove (**Xcode skill**).
- **The bottom edge is no longer the app's alone.** "On other phones, I think developers can expect that they own the
  whole bottom edge, or they own the whole top edge, and that's their place to put custom controls. That doesn't
  translate super well to the Duo because the side controls share the space with the status bar" (**Lab** 287,
  29:14). Said of custom bars against the strip, not of the indicator.
- **The labs, the lab summary and the forum listings never mention the home indicator** as something a Duo shows,
  hides or moves. One developer's unanswered question speaks of "the status bar and home-indicator insets" changing
  mid-fold ([thread 847934][f-midfold], **Developer report**). The iPhone User Guide's gestures page gives the Duo the
  same swipe "up from the bottom edge of the screen" as every other iPhone and says nothing of a bar there
  ([guide][guide], **Doc**).

## Reserved regions do not answer it, as far as can be told

An occlusion region is "an area where an element, such as the Dynamic Island, a camera, or window controls, covers
content" (**Doc**, [ReservedRegion][d-rr]); the overview narrows that to "a hardware element, such as the camera"
(**Doc**, [prep]). The indicator is in neither list. If the system did give the indicator as an occlusion region
while it draws it, that would be the legitimate ask: a foot outside the safe area would move for a region that is
there and stand on the edge where there is none.

Nothing suggests it does, and nothing rules it out:

- The baseline's probe asked for occlusion regions, inactive ones included, in every pose of the Duo simulator. It was
  answered the inner camera's region on the inner display, an active region at the head of the strip for a frame or
  two during a fold, and nothing at the bottom (**Simulator**, [findings][f712]).
- That probe stood inside the safe area, and a query "returns only the regions that overlap your view" (**Doc**). A
  region lying wholly in the bottom 34 points would not have been answered to it (**Inference**). It was also not
  read on the iPhone 18 Pro simulator, where an indicator would be the case to find.

So this stays **Unverified**, and it is the first thing worth running.

## The signals that are left

None of these says anything about the home indicator. Each says something else, which on the user's phone happens to
go with it.

| Signal | Availability | What it truly says | Where it is set on a Duo | On an ordinary iPhone |
|---|---|---|---|---|
| `toolbarVerticalEdge != nil`, or UIKit's `traitCollection.verticalBarEdge != .unspecified` | `anyAppleOS 27.1`; `ios(27.1)` | The system has a preferred side edge for bars in this context, "regardless of whether a vertical bar is currently visible" (**SDK**) | Outer display in both orientations, inner display in landscape and in the book pose, and the left half of a Split View, the one half measured. `nil` on the inner display in portrait and in the laptop pose (**Simulator**) | `nil`: "hardware without a vertical bar" (**SDK**). Not measured |
| `onHingeChange`'s `DeviceHingeContext.hinge != nil`, or `UIHingeInteraction`'s update | `anyAppleOS 27.1`; `ios(27.1)` | The view is in "a hierarchy that provides hinge updates" (**SDK**): the hardware folds | Every pose, closed included (**Simulator**) | `nil` by the doc comment's "Hinge Unavailable" example (**Inference**). Not measured |
| A division region answered with `.includeInactive` | `anyAppleOS 27.1` | A fold crosses this view | The inner display alone; none on the outer (**Simulator**) | None (**Inference**) |
| A leading or trailing safe area inset | Any | Something stands at the side | Where the bar does, but not on the window in Split View (**Simulator**) | In landscape, by Xcode's skill, which says not to test an inset against a threshold (**Xcode skill**) |
| The bottom safe area inset | Any | Nothing: 34 points on both (**Simulator**) | | |
| The device's model | Any | Its name | | Ruled out by the ticket and by Apple |

### The vertical bar's edge is the most honest

- **It is the system speaking of this window, not of the hardware.** It follows the context: `leading` in the left
  half of a Split View, `nil` in portrait on the inner display, `nil` on "hardware without a vertical bar", and so
  `nil` for an iPhone app resized on an iPad or a Mac (**SDK**, **Simulator**; the last is **Inference**). A new
  device that stands its bars at the side would be taken up without a change, and one that folds but keeps them
  horizontal would not.
- **It is what Apple points custom bars at.** "If a custom bar must remain, use toolbarVerticalEdge / verticalBarEdge
  and ReservedRegion APIs to manually reposition it", and twice more to the same effect in the lab summary; "the edge
  of the vertical bar can be read from the environment's toolbarVerticalEdge in SwiftUI" (Technology Evangelist,
  [thread 847835][f-custombar]) (**Forum**). The composer's foot is the app's one custom bar.
- **It fails toward today.** Where it is `nil` the band stays, which is what every iPhone has now. On iOS 27.0, where
  the name does not exist, the other branch of the availability check keeps the band too, and no Duo runs 27.0
  ([0708][f708]). So no ordinary iPhone can be made worse by it (**Inference**).
- **It costs nothing to read.** A `View` is evaluated again when the environment changes (**Xcode skill**), so the
  foot follows a turn or a fold by itself. In UIKit a read outside a layout pass needs
  `registerForTraitChanges(UITraitCollection.systemTraitsAffectingVerticalBarEdge)` (**SDK**, **Xcode skill**).

What it must not be taken for:

- **It is not the indicator.** The reasoning is "where the system has moved its own chrome off the top and bottom
  edges, the foot may take the bottom edge". That the indicator is not kept there is seen on one phone on iOS 27.x
  and written down by nobody at Apple. A setting, an update or another Duo could keep it, and the foot would then
  stand under a drawn indicator with nothing to tell the build (**Inference**).
- **It leaves the band on the inner display in portrait and in the laptop pose**, where it is `nil` (**Simulator**).
  [How the conversation and composer stand on a Duo][t717] decided no band "for both open and closed"; whether the
  user looked at the open display in portrait, and whether the indicator is kept there, is not recorded
  (**Unverified**). If the band is to go there too, this signal cannot do it.
- **It is ambiguous at `nil`.** "`unspecified` is ambiguous: it covers both a configuration where no bar is possible
  and one where a bar is allowed but the edge is not resolved." (**Xcode skill**) Keeping the band on `nil` is the
  safe reading of both.
- **It does not say a bar is showing.** A screen that turned its vertical bar off still reads an edge (**SDK**). For
  this use that is right: the indicator does not follow one screen's choice of bars (**Inference**).
- **Where it is read may matter.** The baseline read it at the app's root. Whether a column of a split view that keeps
  horizontal bars, or a sheet, reads the same value is **Unverified**; the foot stands in the detail column, whose
  bar is the vertical one ([0709][f709]).

### Why not the hinge

The hinge is set in every pose, the inner display in portrait included, so it alone would take the band away
everywhere on a Duo. It is the less honest of the two:

- It says "this hardware folds", which is the device's name in other words. It is what Apple's staff ask an app not
  to do.
- Apple gives the hinge "for interactions and effects", and layout to size classes, reserved regions and arrangements
  ([What Apple's design guidance asks of an app on a Duo][t707]).
- It is handed to an action and is not in the environment: `onHingeChange` calls back with the context, where the
  edge is there as the view is first evaluated (**SDK**). In the baseline's log some probes had written their size,
  insets and bar edge with the hinge still unread (**Simulator**). So a foot placed by it may move once after it
  first appears (**Inference**).
- It would also be set in whatever a later device with a hinge does with its indicator.

### If neither will do

The build cannot know; the user can. A choice in the app, off until asked for, is honest in a way no proxy is, and
puts on the user what the system should have said. Whether that is worth a setting is the spec's to decide and not
this note's.

## What the map takes on trust

Two things the question rests on that no source backs:

- **That a Duo does not keep its indicator.** Seen on the user's phone, in the app, closed and open (**Phone**). No
  Apple source describes it, and Apple's own talk drags an app by the indicator on the inner display.
- **That an ordinary iPhone draws it all the time.** That is what `prefersHomeIndicatorAutoHidden`'s documentation
  describes for an app that returns `false`. It has not been looked at on this map: an ordinary iPhone in the hand
  is under "Not seen" in [How the conversation and composer stand on a Duo][t717]. The simulator's pictures do not
  settle it. Of the eight iPhone 18 Pro frames in the two contact sheets I looked at, one shows an indicator
  (`pro-session-settled` in [the control sheet][shot-pro]) and seven do not; of the eighteen Duo frames of the app I
  looked at, none does. What decides whether a simulator's screenshot carries the indicator I did not find out, so a
  screenshot is weak evidence either way (**Simulator**, **Inference**).

## What I could not verify, and what to run

1. **Whether the indicator is ever an occlusion region.** Take the baseline's
   [`DuoProbe.swift`][probe], put `.ignoresSafeArea()` on its `GeometryReader` so the view reaches the display's
   bottom edge, and run the app on the iPhone 18 Pro simulator and on the Duo simulator closed, open in landscape and
   open in portrait. Read `occlusion=` in what it writes. Look for a region whose frame lies in the bottom 34 points,
   and whether it is `active=true` on the iPhone and absent or `active=false` on the Duo. If so, reserved regions are
   the answer and this note's fallback is not needed. A simulator may not draw or report the indicator as a phone
   does, so the same probe on the two phones is the last word.
2. **What an ordinary iPhone reads.** The same run on the iPhone 18 Pro simulator: `barEdge=` should be `nil` and
   `hinge=` should be `none`. Both are the SDK's words and unmeasured.
3. **The inner display in portrait and the laptop pose, on the phone.** Open the Duo, turn it to portrait, open a
   Session and leave it alone for ten seconds: is the indicator drawn at the bottom, and does it stay? The same seated
   as a laptop. This decides whether the band the vertical bar's edge leaves there is right or wrong.
4. **An ordinary iPhone on iOS 27.x, in the hand.** Open a Session and leave it alone for ten seconds: is the
   indicator still drawn? If it is not kept there either, the question this ticket answers changes.
5. **Whether it is a setting.** On the Duo, look through Settings for anything that decides when the indicator shows.
   If there is one, the build's proxy is wrong on a phone set the other way.
6. **Split View on the phone.** The edge is `leading` in the left half (**Simulator**), so the foot would stand on the
   bottom edge there, where the drag that moves an app between halves begins. Not seen on the phone.
7. **The value inside a column.** Read `toolbarVerticalEdge` in the detail column, in the listing's column and in a
   sheet, on the inner display in landscape, and see that the detail column's is not `nil`.
8. **What `persistentSystemOverlays(.hidden)` does on a Duo.** Only if the spec comes to want it.
9. **Forum text.** Each quote is as a page-fetching tool relayed it. The fourth page of the UIKit Q&A's listing and
   the threads without an Apple reply were not opened, beyond what the listings show of them.

## Sources

- The iOS 27.2 SDK in Xcode 27.2 (27B52), and the skills `xcrun agent skills export` writes
- [Preparing your app for iPhone Duo][prep]; [ReservedRegion][d-rr]; [prefersHomeIndicatorAutoHidden][d-prefers];
  [persistentSystemOverlays(_:)][d-overlays]; [toolbarVerticalEdge][d-edge]; [verticalBarEdge][d-uiedge]
- Human Interface Guidelines: [Designing for iPhone Duo][hig-duo], [Layout][hig-layout]
- [Get ready for iPhone Duo][duo], which lists every resource
- Tech talks: [Prepare your app for iPhone Duo][talk-prepare], [Raise the bar with iPhone Duo][talk-bar],
  [Strike a pose with adaptive layouts on iPhone Duo][talk-pose],
  [Leverage multiple displays and scenes on iPhone Duo][talk-scenes],
  [Build a great camera experience for iPhone Duo][talk-camera], [Design for iPhone Duo][talk-design]
- iPhone Duo Group Lab: [285][lab285], [286][lab286], [287][lab287]
- Forums: [A Summary of the iPhone Duo Group Lab][labsummary], [iPhone Duo SwiftUI Q&A][qa-swiftui],
  [iPhone Duo UIKit Q&A][qa-uikit]; with an Apple reply, [Detecting the iPhone Duo][f-detect],
  [How to get a place for my custom vertical bar?][f-custombar], [Custom floating bottom right button][f-floating];
  without one, [Safe area and status-bar height during the fold animation][f-midfold],
  [Custom controls on the side][f-side]
- iPhone User Guide: [Learn gestures to navigate iPhone][guide]
- This map: [0708][f708], [0709][f709], [0712][f712] and its [probe][probe]

[t728]: https://github.com/jake-tucker/shaman/issues/728
[map]: https://github.com/jake-tucker/shaman/issues/706
[t707]: https://github.com/jake-tucker/shaman/issues/707
[t712]: https://github.com/jake-tucker/shaman/issues/712
[t717]: https://github.com/jake-tucker/shaman/issues/717
[f708]: https://github.com/jake-tucker/shaman/blob/research/duo-layout-apis/docs/research/0708-duo-layout-apis.md
[f709]: https://github.com/jake-tucker/shaman/blob/research/duo-vertical-bars/docs/research/0709-duo-vertical-bars.md
[f712]: https://github.com/jake-tucker/shaman/blob/research/duo-baseline/docs/research/0712-duo-baseline.md
[probe]: https://github.com/jake-tucker/shaman/blob/research/duo-baseline/docs/research/0712-duo-baseline/scratch/DuoProbe.swift
[shot-pro]: https://github.com/jake-tucker/shaman/blob/research/duo-baseline/docs/research/0712-duo-baseline/sheets/iphone-18-pro-control.jpg
[duo]: https://developer.apple.com/iphone-duo/
[prep]: https://developer.apple.com/documentation/technologyoverviews/preparing-your-app-for-iphone-duo
[d-rr]: https://developer.apple.com/documentation/swiftui/reservedregion
[d-prefers]: https://developer.apple.com/documentation/uikit/uiviewcontroller/prefershomeindicatorautohidden
[d-overlays]: https://developer.apple.com/documentation/swiftui/view/persistentsystemoverlays(_:)
[d-edge]: https://developer.apple.com/documentation/swiftui/environmentvalues/toolbarverticaledge
[d-uiedge]: https://developer.apple.com/documentation/uikit/uitraitcollection/verticalbaredge
[hig-duo]: https://developer.apple.com/design/human-interface-guidelines/designing-for-iphone-duo
[hig-layout]: https://developer.apple.com/design/human-interface-guidelines/layout
[talk-prepare]: https://developer.apple.com/videos/play/tech-talks/111461/
[talk-bar]: https://developer.apple.com/videos/play/tech-talks/111462/
[talk-pose]: https://developer.apple.com/videos/play/tech-talks/111463/
[talk-scenes]: https://developer.apple.com/videos/play/tech-talks/111464/
[talk-camera]: https://developer.apple.com/videos/play/tech-talks/111465/
[talk-design]: https://developer.apple.com/videos/play/tech-talks/111466/
[lab285]: https://developer.apple.com/videos/play/meet-with-apple/285/
[lab286]: https://developer.apple.com/videos/play/meet-with-apple/286/
[lab287]: https://developer.apple.com/videos/play/meet-with-apple/287/
[labsummary]: https://developer.apple.com/forums/thread/847644
[qa-swiftui]: https://developer.apple.com/forums/activities/1664080
[qa-uikit]: https://developer.apple.com/forums/activities/1670080
[f-detect]: https://developer.apple.com/forums/thread/847775
[f-custombar]: https://developer.apple.com/forums/thread/847835
[f-floating]: https://developer.apple.com/forums/thread/847858
[f-midfold]: https://developer.apple.com/forums/thread/847934
[f-side]: https://developer.apple.com/forums/thread/847921
[guide]: https://support.apple.com/guide/iphone/learn-gestures-for-iphone-models-with-face-id-iphfdf164cac/ios
