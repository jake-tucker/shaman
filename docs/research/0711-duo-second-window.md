# What a second window of the app takes on an iPhone Duo (#711)

Researched 2026-11-21 for the map in #706, against Xcode 27.2 (27B52) and its iPhoneOS 27.2 SDK, Apple's documentation
and talks as they stood that day, and the app at `63e373f8`.

Every platform claim carries one of four marks:

- **SDK**: declared in the installed SDK. The declaration, or the doc comment shipped beside it, is given verbatim.
- **Apple**: stated by Apple's documentation or a talk.
- **Engineer**: said by a person marked as Apple staff on the developer forums, or in the DTS summary of the Group Lab.
- **Unverified**: no source settles it. It needs to be seen on the iPhone Duo simulator or the phone.

A line marked **Inference** is mine, drawn from the sources it names, and no source states it.

Nothing was built or run for this note and no app code was changed. Two limits on the quotations: the SDK text and the
documentation pages named under "Read as JSON" in [Sources](#sources) were read directly, so those are exact; the talk's
transcript and the forum threads were read through a page-fetching tool that returns passages it was asked to quote, so
those quotations are as that tool gave them, typos included, and were not compared against the pages by eye.

This note does not say whether to offer two windows. That is #721's, with the user.

## Answer in brief

A second window on a Duo is iPad's multiple-scene model moved to an iPhone: the app declares
`UIApplicationSupportsMultipleScenes`, each window is its own scene with its own state and its own scene storage, and
the two scenes share one process. New windows can be made on the inner display only, so a request can be refused at any
moment and must be handled.

Three facts bear on the decision more than the rest:

1. **The app declares multiple scenes already.** `INFOPLIST_KEY_UIApplicationSceneManifest_Generation = YES` writes
   `UIApplicationSupportsMultipleScenes = true` into the built `Info.plist`. Whether to have a second window is
   therefore not a choice between doing nothing and adding something: doing nothing is, by declaration, the multi-window
   path, and a single-window app takes a change to the Info settings. Whether the system does in fact offer an
   iPhone-only app a second window on a Duo is unverified.
2. **The Rust core is opened once and is one for the whole app, which is right, but much of what it holds is one per
   core where two windows would want one per window**: the listing's scope and search, the settled shelf's length, the
   Workspace Picker's search, the Outlook, the Landing's draft, and each Session's composer draft.
3. **Closing the device puts the second window in the background while the first stays in front.** `RootScreen` tells
   the core the app left the front when its own scene does, so a closing Duo would have the core give up every relay
   login under way while a window of the app is still on screen.

## The platform half

### What a Duo does with windows

- **Apple**, [Leverage multiple displays and scenes on iPhone Duo][talk-scenes], chapter "Support multiple scenes"
  (3:38): "iPhone Duo is the first iPhone to support multiple instances of your app's UI. If your app supports this on
  iPad, it will on iPhone Duo as well. There is one thing to keep in mind. On iPad, new windows can be created at any
  time. On the outer display of iPhone Duo, new windows cannot be created. That behavior is reserved for the inner
  display. This dynamic availability is unique behavior to iPhone Duo. Make sure you handle errors when requesting new
  scenes. Use UIWindowSceneActivationAction, which automatically hides when new windows aren't available. If you're
  interested in adding support for multiple scenes for the first time, check out the documentation."
- **Apple**, same talk (2:59): "All apps participate in multitasking on iPhone Duo, where two apps can be placed in a
  side-by-side layout." This is Split View multitasking between apps, which no app opts in to or out of. It is a
  separate thing from a second window of one app.
- **Engineer**, [thread 847966][t-847966] (Frameworks Engineer): "All apps participate in iPhone Duo's multitasking, in
  that you cannot prevent multitasking with any sort of API in your app [...] This does _not_ mean there is a
  requirement for your app to support multiple scenes, as that can often be a larger task to reason about data models
  for separate UI. instances." And: "Just like on iPad, each user-level window is a scene with its own `SceneDelegate`.
  So yes, opening a second window would create a separate scene".
- **Engineer**, [Group Lab summary][lab-summary]: "Does iPhone Duo support multiple windows? Yes. It's the first iPhone
  to support multiple instances of your app's UI, following iPad's scene-based multi-window model — though new windows
  can only be created on the inner display, not the outer one." And: "Can I present multiple instances of my app in the
  inner display like Safari? Yes. iPhone Duo supports multiple simultaneous scene instances of your app's UI via the
  standard windowing APIs on the inner display, the same as iPad."
- **Engineer**, same summary: "Does iPhone Duo require apps to support multiple UIScenes for fold/unfold, and when is
  that recommended? It's not required universally. Multiple scenes matter specifically for Split View multitasking with
  multiple app instances and for scene accessories — hinge angle resizing is just a trait/geometry change to the
  existing scene."
- **Engineer**, same summary: "Is there an API to directly set or modify a window scene's size? No [...] Scene size on
  iPhone Duo is driven by device pose and Split View, aside from the multi-window activation flow on the larger inner
  screen." And "iPhone Duo does not support Stage Manager." So the second window stands in Split View's even split
  beside the first; an app chooses neither its size nor its side.
- **Engineer**, same summary: "Both UserDefaults and AppStorage are app-wide, not per-window, stores", and of audio:
  "Each instance is its own UIScene/window, the same multi-instance model as iPad."
- **SDK**: nothing in UIKit's headers or SwiftUI's interface says where on a hinged device a scene may be made. The
  iOS 27.1 additions around scenes are `UIHinge`, `UIHingeInteraction`, the camera capture scene accessory and its
  session role; none is a query for "may a window be made now". The only general query is the one that predates the
  Duo, `UIApplication.supportsMultipleScenes`, below.

### What an iPhone-only SwiftUI app declares

- **SDK**, `UIApplication.h`:

  ```objc
  // returns YES if the application both declares multiple scene support in its info.plist and the executing environment allows multiple scenes for at least one system type. NO otherwise.
  @property(nonatomic, readonly) BOOL supportsMultipleScenes API_AVAILABLE(ios(13.0)) API_UNAVAILABLE(watchos);
  ```

- **Apple**, [UIApplicationSupportsMultipleScenes][doc-key]: "If your app supports multiple scenes, set the value of
  this key to true. If you set the value to false, UIKit never creates more than one scene for your app. Setting this
  key to true has implications for your code. An app that supports multiple scenes must coordinate operations to
  prevent scenes from interfering with each other."
- **SDK**, the doc comment of SwiftUI's `WindowGroup`: "To enable an iPadOS app to simultaneously display multiple
  windows, be sure to include the `UIApplicationSupportsMultipleScenes` key with a value of `true` in the
  `UIApplicationSceneManifest` dictionary of your app's Information Property List." And: "Every window in the group
  maintains independent state. For example, the system allocates new storage for any `State` or `StateObject`
  variables instantiated by the scene's view hierarchy for each window that it creates."
- **SDK**, Xcode 27.2's build setting specification (`CoreBuildSystem.xcspec`),
  `INFOPLIST_KEY_UIApplicationSceneManifest_Generation`: "When `GENERATE_INFOPLIST_FILE` is enabled, sets the value of
  the UIApplicationSceneManifest key in the Info.plist file to an entry suitable for a multi-window application."
- **The app sets that to `YES`** (`ios/Shaman.xcodeproj/project.xcproj`, the `Shaman` target). An `Info.plist` built
  from this project with Xcode 27.2, read from a simulator's installed copy of the app and from a Release build for the
  phone in another checkout, holds:

  ```
  "UIApplicationSceneManifest" => { "UIApplicationSupportsMultipleScenes" => true, "UISceneConfigurations" => { } }
  "UIDeviceFamily" => [ 0 => 1 ]
  ```

  So the single `WindowGroup` in `ShamanApp` is, as declared, a group any number of windows may be made from.
  **Inference**, from the `WindowGroup` comment naming the key as the one thing to include: nothing more need be
  declared for a second window of the whole app, and SwiftUI's own `openWindow` needs no `NSUserActivityTypes`, scene
  configuration or scene delegate.
- **Engineer**, [thread 847945][t-847945] (Frameworks Engineer): "you are not required to support iPad to support the
  iPhone Duo as its device idiom is still `.phone`." And the Group Lab summary: "No, support for iPad is not required
  for building apps for iPhone Duo."
- **Unverified**: that an app with `TARGETED_DEVICE_FAMILY = 1` and the key set gets a second window on a Duo. Apple's
  sentence is "If your app supports this on iPad, it will on iPhone Duo as well", and an iPhone-only app supports
  nothing natively on iPad. Every other source reads as though the key alone decides it, and none says so outright.
  This is the first thing to see on the simulator.

**To have one window only**, the key must be `false` or absent. Turning the generation setting off drops the whole
manifest; giving the target an `Info.plist` of its own that holds the manifest with the key `false`, merged with the
generated keys, keeps the manifest. **Unverified** which of the two a SwiftUI app built with Xcode 27.2 wants; either is
a change to the Info settings and no code. Split View beside another app goes on either way.

### How a second window is asked for

By the person, through the system:

- **Unverified**: the gesture or control by which a person opens a second window of one app on a Duo. No source read
  describes it. The talk and the forum answers speak only of "the multi-window activation flow" and "Split View
  multitasking with multiple app instances". On the simulator: whether Shaman can be chosen as its own partner in Split
  View as built today.

By the app, in SwiftUI:

- **SDK**:

  ```swift
  @available(iOS 16.0, macOS 13.0, *)
  @available(tvOS, unavailable)
  @available(watchOS, unavailable)
  @preconcurrency @_Concurrency.MainActor public struct OpenWindowAction {
    @_Concurrency.MainActor @preconcurrency public func callAsFunction<D>(value: D) where D : Swift.Decodable, D : Swift.Encodable, D : Swift.Hashable
    @_Concurrency.MainActor @preconcurrency public func callAsFunction(id: Swift.String)
    @_Concurrency.MainActor @preconcurrency public func callAsFunction<D>(id: Swift.String, value: D) where D : Swift.Decodable, D : Swift.Encodable, D : Swift.Hashable
  }
  extension SwiftUICore.EnvironmentValues {
    public var openWindow: SwiftUI.OpenWindowAction { get }
  }
  ```

  The throwing, `async` forms that take a `sharingBehavior` are `@available(iOS, unavailable)`, so on iOS the action
  returns nothing and throws nothing.
- **SDK**, the doc comment of `WindowGroup`, on a group made with `WindowGroup(for:)`: "If a window with a binding to
  the same value that you pass to the `openWindow` action already appears in the user interface, the system brings the
  existing window to the front rather than opening a new window." And: "SwiftUI persists the value of the binding for
  the purposes of state restoration, and reapplies the same value when restoring the window."
- **SDK**, `dismissWindow` is `@available(iOS 17.0, macOS 14.0, visionOS 1.0, *)`; `pushWindow` and
  `defaultLaunchBehavior` are unavailable on iOS.
- **SDK**: SwiftUI's interface has nothing new in iOS 27.0 to 27.2 for asking for a window. Its only scene additions
  are scene accessories (`sceneAccessory`, `ExternalNonInteractiveAccessory`, `CameraCaptureAccessory`).

By the app, in UIKit, which a SwiftUI app may also call:

- **SDK**, `UIApplication.h`:

  ```objc
  /// Asks the system to activate an existing scene, or create a new scene and associate it with your app.
  /// - Parameters:
  ///   - request: The activation request.
  ///   - errorHandler: A handler to be called if the request fails.
  - (void)activateSceneSessionForRequest:(UISceneSessionActivationRequest *)request
                            errorHandler:(nullable void (^)(NSError * error))errorHandler API_AVAILABLE(ios(17.0), tvos(17.0)) API_UNAVAILABLE(watchos);
  ```

- **SDK**, `UIWindowSceneActivationAction.h`, the action the talk names. It is a `UIAction`, for a `UIMenu` or a
  UIKit control, and SwiftUI's `Menu` takes no `UIAction`:

  ```objc
  UIKIT_EXTERN API_AVAILABLE(ios(15.0)) API_UNAVAILABLE(watchos, tvos)
  /// An action that facilitates activating a @c UIWindowScene when performed.
  @interface UIWindowSceneActivationAction: UIAction
  /// @param alternateAction If running on a platform that doesn't support multiple windows, the alternate action is used. Otherwise, the alternates handler is called if the scene activation request fails.
  /// @param configurationProvider Called when the action has been triggered. It should return a @c UIWindowSceneActivationConfiguration whose user activity will be used to request scene activation.
  ```

  Its configuration is made from an `NSUserActivity` (`UIWindowSceneActivationConfiguration.initWithUserActivity:`).
  **Inference**, from Apple's [article for iPad][doc-ipad], which says of a window made from an activity that its
  "`activityType` must be included in the app's `Info.plist` under the `NSUserActivityTypes` array": this path needs an
  activity type the app declares and a scene that takes it, which in SwiftUI is `handlesExternalEvents(matching:)` on
  the scene (iOS 14).

### What a refusal looks like

- **SDK**, `UISceneDefinitions.h`, what `errorHandler` is handed:

  ```objc
  typedef NS_ERROR_ENUM(UISceneErrorDomain, UISceneErrorCode) {
      UISceneErrorCodeMultipleScenesNotSupported,
      UISceneErrorCodeRequestDenied,
      ...
  } API_AVAILABLE(ios(13.0)) API_UNAVAILABLE(watchos);
  ```

- **SDK**, the doc comment of `EnvironmentValues.supportsMultipleWindows` (iOS 16):

  > The reported value depends on both the platform and how you configure your app:
  > * In macOS, this property returns `true` for any app that uses the SwiftUI app lifecycle.
  > * In iPadOS, this property returns `true` for any app that uses the SwiftUI app lifecycle and has the Information
  >   Property List key `UIApplicationSupportsMultipleScenes` set to `true`.
  > * For all other platforms and configurations, the value returns `false`.
  >
  > If the value is false and you try to open a window, SwiftUI ignores the action and logs a runtime error.

  The comment in the 27.2 SDK, and the page on the web, name iPadOS alone and do not mention the Duo.
- **Apple**, the talk: `UIWindowSceneActivationAction` "automatically hides when new windows aren't available". The
  header's own words are that the alternate action is used, or its handler called where the request fails.
- **Unverified**, each to be seen on the simulator:
  - what `supportsMultipleWindows` reads on a Duo, on each display, and whether a view reading it is drawn again as
    the device opens and closes;
  - what `openWindow` does on the outer display: by the comment above, nothing but a line in the log, which the app
    cannot see, so a SwiftUI button has no refusal to show unless `supportsMultipleWindows` follows the display;
  - which `UISceneError` code a request made on the outer display fails with;
  - what a request does when the inner display already holds two windows, of this app or of this app and another.

### What becomes of a second window when the device closes, and opens again

- **Engineer**, [thread 847838][t-847838] (DTS Engineer), answering whether `scenePhase` changes as a Duo closes and
  opens: "if your app is in full-screen using the entire open flat inner display, the scene is unaffected by the hinge
  state change—the system scales and adjusts your window scene based on size class and trait changes. Otherwise, if
  someone is using multi-window on the inner display, the most recently active scene window would be remain active
  when the device closes—the secondary window scene would transition to an inactive state, then backgrounded."
- **Engineer**, Group Lab summary, of one window: "Treat display transitions as size-class and trait changes, not a
  scene disconnect or app termination; your process stays alive." And: "the app undergoes a dynamic resizing and
  transitions from the inner display to the compact outer display, maintaining the existing scene and application
  state."
- **Inference** from those two: on closing, the window last touched moves to the outer display at compact width, and
  the other goes to the background as an app sent away does. Its scene stays connected while the system leaves it so,
  its views and their state stay in memory, and the process runs on because the first scene is in front. In SwiftUI the
  second window's `scenePhase` reads `.background` while the first reads `.active`.
- **Unverified**:
  - whether opening the device again brings the pair back side by side, or the inner display shows the surviving
    window alone and the other waits in the app switcher;
  - whether a scene put in the background this way is later disconnected, as iPadOS does to scenes in the switcher to
    win back memory, in which case its views are made again from scene storage alone;
  - whether the same holds when the partner in Split View is another app, that is, whether Shaman is sent to the
    background by a closing device when it was the window touched less recently. That one matters to the app whatever
    #721 decides, since every app takes part in Split View.

### What each scene keeps, and a cold start

- **SDK**, the doc comment of `SceneStorage` (iOS 14):

  > You use `SceneStorage` when you need automatic state restoration of the value. `SceneStorage` works very similar to
  > `State`, except its initial value is restored by the system if it was previously saved, and the value is shared with
  > other `SceneStorage` variables in the same scene.
  >
  > The system manages the saving and restoring of `SceneStorage` on your behalf. [...] The system makes no guarantees
  > as to when and how often the data will be persisted.
  >
  > Each `Scene` has its own notion of `SceneStorage`, so data is not shared between scenes.
  >
  > If the `Scene` is explicitly destroyed (e.g. the switcher snapshot is destroyed on iPadOS or the window is closed on
  > macOS), the data is also destroyed.

- **SDK**, `UIApplication.h`: `openSessions` is "All of the representations that currently have connected UIScene
  instances or had their sessions persisted by the system (ex: visible in iOS' switcher)". A window is a
  `UISceneSession` with a `persistentIdentifier`, and it outlives the process until it is discarded.
- **SDK**, `Scene.restorationBehavior(_:)` (iOS 18): the default is `.automatic`, "On macOS, this behavior is governed
  by a system setting [...] On all other platforms, it is enabled by default." `.disabled` is unavailable on iOS, so a
  window group cannot be told to forget its windows.
- **Apple**, [Restoring your app's state with SwiftUI][doc-restore]: "On iOS, state restoration is especially important
  at the window or scene level, because windows come and go frequently. It's necessary to save and restore state
  associated with each one. On the iPad, it's especially important because an app in the switcher is not necessarily
  running."
- **Engineer**, Group Lab summary: "For scenes that actually disconnect and reconnect, implement
  stateRestorationActivity(for:) to save an NSUserActivity". That is UIKit's way; `@SceneStorage` is SwiftUI's, and
  the app has it.
- So, by declaration: each window has a `KeptScene` of its own under the one key `"listing"`, and a window opened anew
  starts with none. A window made by `WindowGroup(for:)` also has its presented value kept and handed back.
- **Unverified**: which scenes a cold start connects on a Duo. **Inference**, from `openSessions` above and Apple's
  "an app in the switcher is not necessarily running": a launch connects the scenes that are to be shown and leaves the
  rest as sessions until they are brought forward. On a Duo launched closed, at most one window can show, so the
  second would connect later or not at all in that run, and so read its scene storage later. Whether
  the pair is remembered as a pair, and whether swiping one window away in the switcher destroys its storage on the
  phone (`ios/AGENTS.md` records that the 27.2 simulator keeps it), are both to be seen.

### Scene accessories, for completeness

Scene accessories are the other way an app has two scenes on a Duo, and they are not a second window. **SDK**:
`UISceneAccessory` (iOS 27.0) has `externalNonInteractive(sceneConfiguration:)` and, from iOS 27.1,
`cameraCapture(sceneConfiguration:)`; SwiftUI has `sceneAccessory { CameraCaptureAccessory { ... } }` (iOS 27.1).
**Engineer**, Group Lab summary: "Can non-camera apps display content on the outer display while unfolded? Only via the
system-controlled scene accessory mechanism". The map has the camera capture accessory out of scope, and nothing here
changes that.

## The app half: what assumes one scene

`ShamanApp` declares one `WindowGroup` whose content is `RootScreen`. With a second window SwiftUI makes a second
`RootScreen` with its own `@State`, its own `@SceneStorage` and its own tasks, over the same core.

### What already holds

- **The core is opened once for the process, not once for each window.** `ShamanApp` holds it as
  `@State private var opened = Opened.choose()` on the `App` (`ios/Shaman/ShamanApp.swift`), outside the `WindowGroup`,
  so every window is handed the same `Core`. `ThinClient` is one object over one directory with one runtime
  (`crates/shaman-thin-client/src/lib.rs`), and nothing would open it twice.
- **A listener is one subscription for each caller.** `listened` (`Listened.swift`) makes a stream whose ending cancels
  the subscription the core gave it, and every `watch_*` of the core hands out a subscription of its own over a shared
  `watch` channel. Two windows following the listing, the Outlook or the notices each get every value, and one window
  going cancels only its own.
- **A Session may be open twice.** `session.rs` opens "one for each screen of a Session pushed": each `OpenSession` is
  a reader of its own, and closing one ends that reading alone. Two windows on one Session are two readings of it over
  its Remote's Pairing, twice the traffic and otherwise sound.
- **`KeptScene` is the scene's by construction**, in `@SceneStorage("listing")` in `RootScreen`. It is the only scene
  storage key in the app. Nothing is in `UserDefaults` or `@AppStorage`, which CI checks.
- **The Server-side truth is shared as it should be**: the conversation, a Turn's work, an Approval answered, a
  Session settled, a Remote renamed or removed, a pairing made. A change made in one window reaches the other by the
  model.

### What is held once for the core and would be shared by both windows

Each of these is a single value in the core that every window reads and any window writes. In one window that is
invisible. In two, an act in one window changes what the other shows, with no word of why.

| What | Where | What two windows would see |
|---|---|---|
| The listing's scope | `narrow_listing`, `restore_listing` (`lib.rs`), `Listings::narrow`, `restore` (`listing.rs`) | Narrowing one window's listing to a Remote or a Workspace narrows the other's. Each window then keeps that one scope in its own `KeptScene`. |
| The listing's search | `search_listing`; `ListingScreen` holds the field's text as `@State private var query` | A search typed in one window fills the other's listing with found rows while the other's field stands empty. |
| The settled shelf's length | `show_more_settled` | "Show more" in one window lengthens the other's shelf. |
| The Workspace Picker's search | `workspace_query` in `Inner`; `watch_workspaces` empties it as a picker opens | Opening the picker in one window clears a search under way in the other's. |
| The Outlook | `Remotes`; turned by `open_session`, `narrow_listing`, `choose_workspace` | Opening any Session in one window turns the Outlook, so the other window's "New sessions begin in" changes, and a new-session screen standing in the other window now begins on a different Remote and Workspace from the one it showed as its Prompt was being written. |
| The Landing's draft | one key, `beginning::LANDING_DRAFT`, written "for the Remote the Outlook stands at" (`set_draft`) | Two new-session screens share one draft, each writing over the other. |
| A Session's composer draft | `Drafts` keyed by Session; `set_for` (`drafts.rs`) tells no listener of a change Swift made ("Swift's own change: it holds it already") | Two windows on one Session each hold their own text in `SessionModel.text` and each hand the core every keystroke over the same version. Neither field hears of the other. The core keeps whichever was typed last, and that is what both show after a cold start. Text the core gives back (a send that failed, a Prompt a stop withdrew) is told to both and lands in both fields. |
| A notice | `Notices`: one shown at a time until `notice_seen` | Both windows' banners show it; tapping it away in one takes it from both, which is as wanted. Each window's `NoticeModel` times it separately. |

### What follows the scene where it should follow the app

- **The front.** `RootScreen` reads `@Environment(\.scenePhase)` and calls `core.cameToFront()` on `.active` and
  `core.leftFront()` on `.background`. Inside a view that is the phase of that one scene (**SDK**, the doc comment of
  `ScenePhase`: "If you read the phase from inside a `View` instance, you obtain a value that reflects the phase of the
  scene that contains the view"). `left_front` writes every draft, which is harmless, and "Every login under way is
  given up" (`lib.rs`). `came_to_front` checks every Remote, and "every stream resumes by a fresh snapshot". By the
  DTS answer above, a closing Duo sends the second window to the background while the first stays active: the core is
  told the app left the front while a window of it is on screen, and a relay login standing in the surviving window is
  given up. Opening the device again tells the core the app came to the front though it never left, and every Remote
  is checked afresh. The same doc comment gives the app-wide reading: "If you read the phase from within an `App`
  instance, you obtain an aggregate value that reflects the phases of all the scenes in your app. The app reports a
  value of `ScenePhase/active` if any scene is active".
- **Background time.** `RootScreen` holds `@State private var backgroundTime = BackgroundTime(tasks:
  UIApplication.shared)`, commented "kept for as long as the app is", and follows `core.actsUnderWay()`. Two windows
  are two of them, each beginning a background task for the same act. "Never twice" holds within one only. Harmless,
  and not what the comment says.
- **The network path.** `RootScreen` runs `NetworkPath.changes()`, an `NWPathMonitor` of its own, and tells the core.
  Two windows tell it of each change twice, and it checks every Remote each time.
- **Listeners of a window in the background.** SwiftUI's `.task` runs for as long as its view is in the hierarchy,
  whatever the scene's phase. A second window sent to the background by a closing device keeps every Session on its
  stack open in the core and read live, for as long as the first window keeps the process in front. Nothing is wrong
  in that; it is traffic and battery for a window nobody can see.

### Launch arguments and what a scene is handed

`Launch` in `ShamanApp.swift` reads the process's arguments and environment, and the `WindowGroup`'s content hands them
to every `RootScreen` it makes. All of it is debug-only. In a debug build, a second window would:

- push the Session `--open` names, or the new-session screen for `--compose`, over its own listing as well;
- present the pairing confirmation for `SHAMAN_INVITE` a second time (`RootScreen` holds the Invite as `@State`, which
  each window gets afresh);
- forget its scene storage under `--forget-scene`, and show the sheet alone under `--servers-and-device`.

`RootScreen`'s `restoring`, "whether what the scene kept is still to be handed to a listing: to the first one a launch
shows", is true once in each window, which is right for a window restored at a cold start. But each window that hands
its kept scope back calls `core.restoreListing`, and the scope is one for the core, so the window connected last
decides the scope of both. `KeptScene`'s own comment, "A launch into a new scene, whose storage holds nothing, starts
at the listing under Everywhere", stops being true: a second window opened while the first is narrowed starts at the
first's scope.

### Interventions as sheets in more than one window

`SessionScreen` presents the Session's own Intervention from `model.presented`, which is `frame.intervention` of that
window's own `OpenSession`.

- A Session open in both windows presents the same Approval or Questionnaire as a sheet in each. Answered in one, the
  core takes it from both frames once the Server shows it answered, and the other sheet goes by itself; the binding's
  setter guards against counting that as a swipe. Until then both sheets take a tap; what the Server answers a second
  Decision sent in that moment was not read for this note.
- Swiping the sheet away calls `opened.dismiss()`, which is "for as long as this screen stays open" and held in that
  `OpenSession` alone (`Local.dismissed` in `session.rs`). The other window's sheet stays up.
- What the reader has half given a Questionnaire is `SessionModel.questionnaireDrafts`, in Swift and of that screen
  alone, so the two sheets hold different half-answers and the one sent wins.
- A parent Session in one window and its Subagent's in the other: the parent's bar says the Subagent waits, and its
  "Review" pushes the Subagent's Session in the parent's window, though it is already open, with its sheet up, in the
  other.
- The arming moment (`interventionArming`) is each sheet's own, which is right.

### Smaller things

- **The relay login's anchor.** `WebLoginBrowser.presentationAnchor` (`RelayLoginSheet.swift`) takes the key window
  among all connected scenes' windows, or the first window, or `scenes[0]`. With two windows the system's
  authentication sheet can rise over the window that did not ask.
- **The sheet of Servers, Relays and This device, and pairing**, can be up in both windows at once. Their acts go to
  the one core, which holds one pairing at a time (`pairing: AsyncMutex<()>`), so two are made in turn.
- **The scanner.** Two windows could each stand a scanner at the top of its stack, two readers of one camera.
- **Tests.** `ShamanTests/ConversationListTests.swift` takes the first connected window scene, which is sound while a
  test launches one. `ColdStartTests`, as `ios/AGENTS.md` describes it, is of one scene's storage; it was not read.
- **The fake.** `FakeCore` is one object for the app as the live core is, so the fake stands in faithfully for whatever
  is decided; what it shares between windows was not read line by line for this note.

## What each path would take

Laid out for #721; no path is recommended here.

**One window only.** Set `UIApplicationSupportsMultipleScenes` to `false`, or drop it, in the Info settings, and see on
the simulator that the system then offers no second window. No code changes, and every assumption above stays true.
The app still shares the inner display with another app in Split View, at compact width, which no app can refuse; the
unverified question of what a closing device does to the app when it was the window touched less recently stays open
on this path too.

**A second window by the system's own means, with nothing added to the app.** This is what today's build declares. If
the simulator shows that a person can open one, then everything under "held once for the core" and "follows the scene"
is live behaviour on a Duo today, and this path is the work of making two whole copies of the app sound side by side:

- the front, background time and the network path told to the core once for the app, from the `App`'s own aggregate
  `scenePhase` or the like, in place of once for each `RootScreen`;
- a decision, for each value in the table, between one for each window and shared on purpose. One for each window
  means the core's listing takes a scope, a search and a shelf length for each follower, and likewise the picker; by
  the rule in `ios/AGENTS.md` that is a change to the core, to `Core`, to `LiveCore` and to `FakeCore` together. The
  Outlook is the hard one, since "where the next Session begins" is one fact of the phone that two windows would turn
  two ways;
- a composer draft that two fields can hold at once: the core telling the other listeners of a change, or the app
  never showing one Session's composer twice;
- the relay login anchored to the window that asked;
- launch arguments handed to the first window alone, if UI tests are ever to open two;
- tests on a Duo simulator that open two windows, close the device and open it again, which waits on #710.

**A second window the app offers, to stand one Session beside another.** All of the above, and:

- a way to ask. In SwiftUI, `openWindow` with either the same group (a second whole app, listing first) or a second
  `WindowGroup(for:)` presenting one Session by its reference, which the system would bring forward where that Session
  already has a window and would keep and restore by its value. Or UIKit's `UIWindowSceneActivationAction`, the one
  Apple names, from a UIKit menu, with an `NSUserActivity` type declared and a scene that takes it;
- a control that is there only when a window can be made. Apple asks that it hide on the outer display. In SwiftUI
  that rests on `supportsMultipleWindows`, whose behaviour on a Duo is unverified, and `openWindow` reports no failure
  to the app. In UIKit `activateSceneSession(for:errorHandler:)` does report one;
- what the second window is when the device closes: by the DTS answer it goes to the background and the window last
  touched stays, so which Session a person is left with on the outer display follows their last touch, and the app
  does not choose;
- what a window of one Session alone keeps and restores, since `KeptScene` is the shape of a listing with a stack
  over it;
- the words for it, which the map already lists under the glossary.

**Two Sessions in one window, as a layout.** Not a second scene, so none of the above applies and every one-scene
assumption holds: one `RootScreen`, one `KeptScene`, one front. The core already reads several Sessions at once, one
`OpenSession` for each screen pushed. What it takes is #714's and #721's to say: one stack would become two columns of
Sessions, `KeptScene` would keep both, two composers and two sets of Intervention sheets would stand in one window, and
the app, not the system, would decide what the outer display shows when the device closes. Apple's guidance in #707
(one more level of hierarchy on the inner display, and never a function tied to a pose) is what such a layout would be
measured against.

## Could not verify

Everything marked **Unverified** above, gathered:

1. Whether an iPhone-only app with the key set is offered a second window on a Duo at all.
2. How a person asks for one.
3. What `supportsMultipleWindows` and `UIApplication.supportsMultipleScenes` read on each display, and whether they
   change as the device opens and closes.
4. What `openWindow` does on the outer display, and which `UISceneError` a UIKit request fails with there.
5. What a request does when the inner display is full.
6. Whether the pair comes back side by side when the device opens again, and whether the scene sent to the background
   is ever disconnected.
7. Whether a closing device sends Shaman to the background when it shares the inner display with another app and was
   touched less recently.
8. Which scenes a cold start connects, on each display, and whether a window swiped away loses its scene storage.
9. Which form of the Info settings gives a SwiftUI app one window only.

Not read: the three Group Lab videos themselves ([285][lab-285], [286][lab-286], [287][lab-287]), whose pages carry no
transcript. The DTS engineer's written [summary][lab-summary] stands in for them; its last 23 questions were read for
their headings and for the one answer on Split View only. The `FakeCore`'s state was not audited for two windows.

The simulator has what the checks need: the `iPhone Duo` device type is installed with the iOS 27.2 runtime
(`xcrun simctl list devicetypes`). Posing it from a terminal is #710's question; `simctl` lists nothing for folding,
and `devicectl device` has `appResize` and `orientation` (**SDK**, the tools' own help). Forum threads asking Apple for
a scripted fold ([847883][t-847883], [847938][t-847938]) have no answer from Apple.

## Questions this raised that no ticket on the map holds

1. **Does today's build already open a second window on a Duo?** It declares the key. #712 walks today's build on the
   simulator, Split View included, and does not list a second window of Shaman itself among what it looks at. If one
   opens, the shared state and the per-scene front above are behaviour to record there, and "one window only" becomes a
   change to make, not the state of things.
2. **What a closing device does to Shaman when it shares the inner display with another app.** If the app touched less
   recently is sent to the background, then `leftFront`, the relay login given up, and the check on return all happen
   on a fold, with one window and whatever #721 decides. #715 asks what survives a fold and does not name this.
3. **Whether the scanner's camera runs in Split View.** **SDK**, `AVCaptureSession.h`:
   `AVCaptureSessionInterruptionReasonVideoDeviceNotAvailableWithMultipleForegroundApps` is "An interruption caused
   when the app is running in a multi-app layout [...] the session may only be run if your app occupies the full
   screen", unless `multitaskingCameraAccessEnabled` is set, which is the default only with an entitlement. Whether
   VisionKit's data scanner sets it, and whether a Duo's Split View counts as a multi-app layout, are unverified. This
   belongs with "Pairing and the scanner on two displays", which the map holds as not yet specified.

## Sources

Installed SDK, `iPhoneOS27.2.sdk` in Xcode 27.2 (27B52):

- `System/Library/Frameworks/UIKit.framework/Headers/`: `UIApplication.h`, `UIWindowSceneActivationAction.h`,
  `UIWindowSceneActivationConfiguration.h`, `UISceneSessionActivationRequest.h`, `UISceneDefinitions.h`,
  `UISceneSession.h`, `UIScene.h`, `UIWindowScene.h`, `UISceneAccessory.h`, `UIHinge.h`; and
  `AVFoundation.framework/Headers/AVCaptureSession.h`
- `System/Library/Frameworks/SwiftUI.framework/Modules/SwiftUI.swiftmodule/arm64e-apple-ios.swiftinterface` for
  declarations and availability, and `arm64e-apple-ios.swiftdoc` beside it for the doc comments
- `Xcode.app/Contents/SharedFrameworks/SwiftBuild.framework/.../SWBCore.framework/Versions/A/Resources/CoreBuildSystem.xcspec`
  for the build setting

Apple's documentation. Read as JSON, exactly: [UIWindowScene.ActivationAction][doc-action],
[supportsMultipleScenes][doc-supports], [activateSceneSession(for:errorHandler:)][doc-activate], [Scenes][doc-scenes],
[Multitasking on iPad, Mac, and Apple Vision Pro][doc-multitasking]. Read through the fetching tool:
[Preparing your app for iPhone Duo][doc-prepare] (which says nothing of several scenes beyond linking the talk),
[UIApplicationSupportsMultipleScenes][doc-key], [supportsMultipleWindows][doc-smw], [openWindow][doc-open],
[SceneStorage][doc-storage], [Supporting multiple windows on iPad][doc-ipad],
[Restoring your app's state with SwiftUI][doc-restore], and the Human Interface Guidelines'
[Designing for iPhone Duo][hig-duo] and [Multitasking][hig-multi], neither of which speaks of two windows of one app on
a Duo.

Talks and forums, read through the fetching tool: [Leverage multiple displays and scenes on iPhone Duo][talk-scenes];
[A Summary of the iPhone Duo Group Lab][lab-summary]; the [SwiftUI Q&A][qa-swiftui] and [UIKit Q&A][qa-uikit], every
page of each, and of their threads [847838][t-847838], [847966][t-847966], [847945][t-847945], [847836][t-847836],
[847773][t-847773], [847887][t-847887]. Threads that ask this note's open questions and have no answer from Apple:
[847971][t-847971] (a state every window must show), [847968][t-847968] (which window a notification opens in),
[847959][t-847959] (scene callbacks on a fold), [847897][t-847897] and [847927][t-847927] (one scene or a new one on a
fold), [847967][t-847967] (which display a launch connects for).

The app, at `63e373f8`: `ios/Shaman/ShamanApp.swift`, `ios/Shaman.xcodeproj/project.xcproj`, and in
`ios/ShamanUI/Sources/ShamanUI/`: `RootScreen.swift`, `KeptScene.swift`, `Core.swift`, `LiveCore.swift`,
`Listened.swift`, `ListingScreen.swift`, `SessionScreen.swift`, `NoticeBanner.swift`, `BackgroundTime.swift`,
`NetworkPath.swift`, `RelayLoginSheet.swift`. The core: `crates/shaman-thin-client/src/lib.rs`, `drafts.rs`,
`session.rs`, `listing.rs`, `notices.rs`.

[talk-scenes]: https://developer.apple.com/videos/play/tech-talks/111464/
[lab-summary]: https://developer.apple.com/forums/thread/847644
[lab-285]: https://developer.apple.com/videos/play/meet-with-apple/285/
[lab-286]: https://developer.apple.com/videos/play/meet-with-apple/286/
[lab-287]: https://developer.apple.com/videos/play/meet-with-apple/287/
[qa-swiftui]: https://developer.apple.com/forums/activities/1664080
[qa-uikit]: https://developer.apple.com/forums/activities/1670080
[t-847838]: https://developer.apple.com/forums/thread/847838
[t-847966]: https://developer.apple.com/forums/thread/847966
[t-847945]: https://developer.apple.com/forums/thread/847945
[t-847836]: https://developer.apple.com/forums/thread/847836
[t-847773]: https://developer.apple.com/forums/thread/847773
[t-847887]: https://developer.apple.com/forums/thread/847887
[t-847971]: https://developer.apple.com/forums/thread/847971
[t-847968]: https://developer.apple.com/forums/thread/847968
[t-847959]: https://developer.apple.com/forums/thread/847959
[t-847897]: https://developer.apple.com/forums/thread/847897
[t-847927]: https://developer.apple.com/forums/thread/847927
[t-847967]: https://developer.apple.com/forums/thread/847967
[t-847883]: https://developer.apple.com/forums/thread/847883
[t-847938]: https://developer.apple.com/forums/thread/847938
[doc-prepare]: https://developer.apple.com/documentation/technologyoverviews/preparing-your-app-for-iphone-duo
[doc-key]: https://developer.apple.com/documentation/bundleresources/information-property-list/uiapplicationscenemanifest/uiapplicationsupportsmultiplescenes
[doc-action]: https://developer.apple.com/documentation/uikit/uiwindowscene/activationaction
[doc-supports]: https://developer.apple.com/documentation/uikit/uiapplication/supportsmultiplescenes
[doc-activate]: https://developer.apple.com/documentation/uikit/uiapplication/activatescenesession(for:errorhandler:)
[doc-scenes]: https://developer.apple.com/documentation/uikit/scenes
[doc-multitasking]: https://developer.apple.com/documentation/uikit/multitasking-on-ipad-mac-and-apple-vision-pro
[doc-smw]: https://developer.apple.com/documentation/swiftui/environmentvalues/supportsmultiplewindows
[doc-open]: https://developer.apple.com/documentation/swiftui/environmentvalues/openwindow
[doc-storage]: https://developer.apple.com/documentation/swiftui/scenestorage
[doc-ipad]: https://developer.apple.com/documentation/uikit/supporting-multiple-windows-on-ipad
[doc-restore]: https://developer.apple.com/documentation/swiftui/restoring-your-app-s-state-with-swiftui
[hig-duo]: https://developer.apple.com/design/human-interface-guidelines/designing-for-iphone-duo
[hig-multi]: https://developer.apple.com/design/human-interface-guidelines/multitasking
