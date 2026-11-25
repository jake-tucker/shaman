# What Rune offers and lacks for a coding-agent Client

Research for [#769](https://github.com/jake-tucker/shaman/issues/769), part of the map
[#767](https://github.com/jake-tucker/shaman/issues/767).

- **Source.** [anomalyco/rune](https://github.com/anomalyco/rune) at `983e33b916b0da5719442d94ae04a00867c952a3`
  (branch `dev`, committed 2026-11-25 09:28 UTC). Every path below is relative to that repository at that
  revision. `README` alone means `crates/rune/README.md`; `kit README` means `crates/rune-kit/README.md`.
- **Method.** Read the source, the two READMEs, `AGENTS.md`, `ARCHITECTURE.md`, `CHANGELOG.md`,
  `changelog/unreleased/`, the examples index and the open issues (`gh issue list -R anomalyco/rune`, read
  2026-11-25). Nothing of Rune's was built or run, so every "works" below means "the source and docs say so and
  a test or example exists", not "seen working". Anything inferred or not determined is marked.
- **Reading note.** Rune is written for OpenCode Desktop, a coding-agent app, and its docs use that app as the
  worked case (`README:440`, `demo/opencode/*.png`). The app's own source is not in the repository: there is
  no end-to-end transcript example to copy, only the parts.

## Missing or risky

Ordered by how much each should weigh on the spec.

1. **The pin will be expensive to move, and Rune moves about a hundred commits a day.** `dev` held 3,320
   commits 34 days after the repository was created (GitHub API, 2026-11-25); 291 landed on 2026-11-23 and 239
   on 2026-11-24, and `dev` had already moved past the pinned revision while this was written.
   `changelog/unreleased/` holds 1,461 fragments, each "a change that can move an app's output or break its
   build" (`AGENTS.md`, "Changes"), 618 of them dated 2026-11-23 to 25. The `[patch.crates-io]` list a consumer
   must copy grew from one crate (`wgpu-hal`) to eight between 2026-11-12 and 2026-11-24
   (`changelog/unreleased/2026-11-12-winit-taskbar.md`, `2026-11-12-parley-l1.md`,
   `2026-11-12-a11y-uia-headings-landmarks.md`, `2026-11-15-macos-headings.md`,
   `2026-11-18-wayland-csd-corners.md`, `2026-11-24-flat-tree-item-set-size.md`,
   `2026-11-24-vendor-patched-naga.md`), and since 2026-11-12 a build without the `winit` patch does not
   compile on macOS (open issue #850). Rune's CI tests macOS only, every six hours and never per change
   (`.github/workflows/macos-tests.yml`), and never runs clippy (#641), so Windows and Linux regressions are
   found by consumers. Rune takes no pull requests; several open issues end with its bot saying it "made a
   change for this but could not open a pull request (rejected by review)" (#669, #1093, #1407).
2. **On macOS the app has to be a signed `.app` bundle, which Shaman does not ship today.** Notifications post
   only from "a bundled, signed app" (`crates/rune-notify/src/mac.rs:1-2`); `rune::single_instance` does
   nothing on macOS and relies on Launch Services, which only covers bundles
   (`crates/rune/src/open_url/instance.rs:27,40`); an icon and URL schemes come from the bundle too
   (`README:1354-1355`). Rune's own bundler makes a development bundle that "cannot be notarized"
   (`README:1378-1380`). A bare binary in a `tar.gz`, as `shaman` ships today, gets no notifications, and
   nothing stops a second copy of it starting on a Mac (the second half inferred from `single_instance`
   returning true there).
3. **No menu bar on Windows or Linux, and no native context menu on Linux.** Native menus return
   `MenuError::Unsupported` off macOS (`crates/rune/src/native_menu/mod.rs:318`,
   `crates/rune/src/shortcuts/README.md`, "Native application and Dock menus"). `tx.popup_menu` is native on
   macOS and Windows only (`crates/rune/src/popup_menu/mod.rs:49,65`, `macos.rs:21`, `win32.rs:35`); elsewhere
   the app draws the kit's in-window `Menu`. No open issue asks for a Windows or Linux menu bar.
4. **Key bindings are single chords, and a popup cannot take keys from a focused field.** A `Chord` is one key
   with modifiers (`crates/rune/src/input/chord.rs:65-80`); the TUI's `Ctrl+X` leader
   (`src/tui/commands.rs:1135-1145` in Shaman) is a two-stroke sequence the app must run itself, and on Windows
   and Linux `Ctrl+X` is Cut in a focused field. Completions beside the caret must be driven from the
   composer's own key hook, because nothing delivers keys to a popup while another element keeps focus (#1365,
   open).
5. **Selection copies plain text, and one selection across several Messages is not proven.**
   `El::selection_copy` returns `Option<String>` (`crates/rune/src/el/text.rs:952`): a rich copy of read-only
   text is the app's to build. The kit's `Markdown::selectable` makes each Markdown its own selection group
   (`crates/rune-kit/src/markdown.rs:670`) and a text belongs to its nearest group
   (`crates/rune/src/runtime/selection/selectable.rs:280-287`), so as shipped a drag stops at the end of one
   agent Message. Whether clearing the group on the built element joins it to a Transcript-wide group is
   inferred, not shown anywhere.
6. **A sticky header sticks only inside its own virtual row** (#1379, open; `README:519-520`). A header that
   stays up for a whole Turn needs the Turn in one row, or nested `virtual_rows`, which lose the fast path.
7. **Rune has no notion of a Nerd Font Icon.** Its `Icon` is a static SVG body
   (`crates/rune-text/src/icon.rs:20-27`). Private-use codepoints are never looked up in system fonts: "an
   app's icon font has them or nothing does" (`crates/rune-text/src/fonts/system_fonts.rs:255-262`). The app
   must bundle a Nerd Font and name its family; no example or test draws one.
8. **Every headless command-line run needs a GPU adapter, `--tree` included**
   (`crates/rune/src/tooling/headless.rs:226`), and the flags exist in debug builds or with the `tooling`
   feature only (`README:3520-3524`). `App::headless_runtime` in a `cargo test` does not build a renderer
   (`crates/rune/src/tooling.rs:148-170`). Nothing headless reaches the native side: real menus, OS
   notifications, an input method, a real drag, activation, single instance.
9. **Smaller gaps.** The diff view's split mode always wraps (#1407). The text caret never blinks (#669). A
   panic in the view ends the process (#1003). There is no native message box (#637). Accessibility has not
   been audited (#111) and a virtual list exposes only its mounted rows (`README:3444-3445`). Rune's docs lag
   its source in at least two places found here: the README says Wayland takes no file drops and that
   `drag_out` is macOS only, while `crates/rune/src/platform/wayland_drop.rs` and
   `crates/rune/src/drag_out/mod.rs:33` say otherwise.
10. **Linux is the weakest platform.** A custom title bar doubles up on GNOME (#591), wheel scrolling is rough
    (#898), `drag_out` does nothing (#526), and there is no reopen or open-URL hook (#22). Linux is not a
    primary target for the first cut, but its build must still pass.

## The table

One row per line of the ticket. "Complete" is a judgement from reading, not from running.

| # | Line | What exists, and where | How complete it looks | What the app builds for itself |
|---|---|---|---|---|
| 1 | Long Transcript | `El::virtual_rows(count, estimate, key, build)` on a growth stack inside a `stick_bottom` scroll (`crates/rune/src/el/virtual_rows.rs:85-108`, `README:472-493`); rows may differ and change height, "measured corrections preserve the visible key and the bottom stays exact". Streaming: `.growth(GrowthSpec)`, `.streaming(true)`, `.reveal(word_fade)` (`README:615-633`). Keeping place: a `virtual_rows` list holds its first visible row on its own, `El::scroll_anchor()` for other content (`README:1226-1234`), `on_at_bottom` and `on_far_from_bottom` (`crates/rune/src/el/scroll.rs:86-110`). Sticky: `.sticky_top`, `.stuck_shadow`, `tx.keep_on_screen` (`README:506-525`). `memo(id, version, build)` per row. | The most finished area: this is what Rune was built for. Vertical only (#1384). Sticky headers do not cross rows (#1379). No example assembles a whole transcript; the nearest are the doc example at `virtual_rows.rs:85-97`, `crates/rune/examples/scroll_anchor.rs`, `sticky.rs` and `crates/rune-kit/examples/scroll_bench.rs`. | The row model (what one row is: an entry, or a Turn with its Turn Fold), stable keys and a revision per row for `memo`, the Working Indicator, the latest-position affordance, and how a Turn Fold's marker stays put (one row per Turn, or accept that it scrolls away). |
| 2 | Selecting across rows, copying plain and rich | `text(..).selectable(true)`, `El::selection_group(true)`, selection across unbuilt and scrolled-out virtual rows with pinned ends (`README:2213-2323`, `CHANGELOG.md:600-608`); edge autoscroll; `tx.text_selection()`, `tx.select_text`; `El::selection_copy`; `El::copy_breaks`. `Markdown::selectable(true)` and `copy_source(true)` (`crates/rune-kit/src/markdown.rs:416-438`). Rich clipboard for writing: `tx.copy_rich(ClipboardContent::new(text).with_html(..).with_rtf(..))` (`README:1998-2070`). | Plain copy is complete, capped at 10,000 unbuilt rows or 4 MiB with `truncated()` saying so. Rich copy of a selection does not exist: `selection_copy` returns a `String`. One selection across several kit Markdowns is not shown to work (see risk 5). RTF is written on macOS only. | Rich copy: take Cmd/Ctrl+C in `on_key` (it sees the key before the selection does, `README:356-358`), read `tx.text_selection()`, and write HTML with `tx.copy_rich`. A probe of selection across Messages. What a copy of an Activity or a Turn Fold yields. |
| 3 | Markdown, code, tables, links, an Activity's renderer | Kit `Markdown` (feature `markdown`, pulldown-cmark): CommonMark, GFM tables, task lists, strikethrough, images, autolinks, `diff` fences, optional Mermaid and LaTeX; "built for streaming: re-render with the longer source each time, and earlier blocks stay mounted" (`kit README:458-549`, `crates/rune-kit/src/markdown.rs:1-62`). Syntax colour: `rune_kit::syntax`, 80 lexers (`crates/rune-highlights`), usable outside Markdown as `Span`s. Links: `Markdown::on_link`, `on_link_secondary`, `on_link_hover` (`crates/rune-kit/src/markdown/links.rs`); opening is `Tx::open_url` (http, https, mailto only) or `Tx::open_path`, `Tx::reveal_path` (`README:2502-2516`). | Complete for an agent's Messages. A fence colours its first 128 KiB or 32,768 tokens and draws the rest plain (`crates/rune-kit/src/syntax.rs:131-133`). Raw HTML blocks draw as literal text (`crates/rune-kit/src/markdown/parser.rs:25`). Images need the app to supply the picture (`Markdown::images`). | There is no Activity or tool-call component. A renderer for one kind of Activity is composed from `row`/`col`/`text`/`rich`/`icon`, the kit's `Collapsible`, `Accordion`, `Card`, `Badge`, `Spinner`, `Markdown` (with `code_frame`, `diff_frame` for a header and copy button), `syntax::highlight` and `DiffView`. The app owns the typed mapping from Activity kind to renderer, and the placeholder for a kind it does not know. Link policy (what a file path or a non-web scheme does) is the app's. |
| 4 | Diff view and inline images | `rune_kit::diff`: `diff::parse(patch)` for `git diff` output, `FileDiff::from_texts`, `DiffState`, `DiffView`; unified or split, word-level changes, syntax colour, folds, sticky file header, annotations, line selection, virtualised (`kit README:551-618`, `crates/rune-kit/src/diff/`). Images: `ImageCache::load` and `load_scaled` decode PNG, APNG, JPEG, GIF, WebP, BMP, ICO, SVG, and AVIF behind a feature, off the UI thread; `El::pixels`, `pixels_fit`; kit `image_zoom::Preview` for a zoomable view (`README:2697-2828`, `crates/rune-kit/src/image_zoom/mod.rs`). | Diff: stated to hold at two 50,000-line texts (diff about 40 ms, first frame about 20 ms; the test is `crates/rune-kit/tests/integration/diff.rs:773`, ignored by default, not run here). Split mode always wraps (#1407). A parsed patch's folds cannot open, since it holds no whole file. Word diff skips lines over 1,000 bytes (`crates/rune-kit/src/diff/words.rs:12`). Images: at most 8,192 px a side and 256 MiB decoded (`crates/rune-image/src/lib.rs:63-67`); atlas keeps 64 MiB of unshown pixels. A pasted image is dropped over 32 MiB as PNG. | Fetching: Rune has no HTTP client, so an Attachment's bytes come from Shaman's Server and go to `cache.load`. Sizing rules for an inline image, the open-larger preview, and what a too-large or failed image shows. Whether a diff comes as a patch or as two texts decides whether folds open. |
| 5 | The editor | `text_input(&edit, style)` over `TextEdit` (`crates/rune-edit`), kit `TextArea` with `auto_grow`, `max_height`, `.plain()` for a composer's own chrome (`kit README:105-128`). Multiline with wrap; undo and redo grouped by typing, 128 edits or 1 MiB (`README:1913-1918`). IME: preedit through winit, plus Rune's own clause and reconversion handling on macOS and Windows (`crates/rune/src/platform/ime/macos.rs`, `windows.rs`). Paste: text, sanitised HTML, RTF, and images as PNG through `on_input_paste` and `content.image()` (`README:2011-2042`). Pills: `Mark::Pill(id)`, `edit.insert_pill`, `replace_with_pill`, `pill_style`, `on_pill_press`, `pill_copy_text` (`README:2123-2152`). Caret rect: `Cx::caret(key)` (`crates/rune/src/runtime/cx.rs:504-509`). Popup: kit `PopupList`, written for "a chat composer's `@`-mention and `/`-command popovers" (`crates/rune-kit/src/overlays/popup_list.rs:1-3`). Example: `crates/rune-kit/examples/rich_editor.rs`. | Strong. Pills cover mentions and numbered labels such as `[Image 1]` and `[Paste 1: 12 lines]` as atomic runs. Linux IME has no module of Rune's own and rests on winit (inferred from `crates/rune/src/platform/ime.rs:8-13`). The caret does not blink (#669). `PopupList` anchors to an element, not to the caret. Spellcheck exists per platform and is off unless asked. | Completion logic end to end: detecting `/` and `@` before the caret, filtering, routing Up, Down, Return, Tab and Escape from the field's `on_input_key` to the list (#1365), placing the list from `Cx::caret`, and swapping the typed query for a pill in one undo step. Which keys send, steer and queue (`on_submit` is Return; Shift- or Option-Return types a newline). Turning a pasted image or a large paste into an Attachment or a label. |
| 6 | Focus and keys | Key routing in eleven ordered steps (`README:332-364`). `El::on_key_preview` runs on a focused field's ancestors before the field, so app-wide bindings work while the composer has focus. `Chord::parse("mod+shift+t")`, `Keybind` with per-platform alternatives, labels for display (`README:392-414`, `crates/rune/src/input/chord.rs`). Typed text arrives as `Key::Text`, a command as `Key::Command(c)`; `tx.physical_key()` and `tx.key_base()` separate the key from the layout (`README:380-390`). Global (system-wide) shortcuts on all three platforms (`crates/rune/src/shortcuts/README.md`). Focus: `focusable`, `tab_index`, `focus_group`, `modal` with trap and restore (`README:306-331`). | Thorough for single chords. No key sequences. Reserved or pre-empted: on macOS the standard menus take ⌘Q, ⌘H, ⌘W, ⌘M and route ⌘Z, ⌘X, ⌘C, ⌘V, ⌘A back to Rune (`README:895-907`); a field keeps the plain edit chords, and Ctrl+Y and Shift+Insert off macOS (`README:416-418`); Left Ctrl + Left Alt is AltGr on Windows and types; Tab, Escape, Return, Space and the paging keys have built-in meanings; Alt+F4 and Alt+Space are the system's. Windows and Linux never report Fn. | The binding table from semantic command to chord per platform, a sequence state machine if the `Ctrl+X` leader survives, the choice of what `Ctrl+X` means in the composer off macOS, and which TUI bindings a window cannot have. |
| 7 | Menus | macOS: `App::menu_item(MenuItem::new(title, action).menu("File").key_equivalent(..))`, `handle.set_menu`, submenus, Dock menu, `standard_menus()` (`crates/rune/src/shortcuts/README.md`, `crates/rune/src/native_menu/`). Context menus: `tx.popup_menu(menu, anchor, on_pick)`, native on macOS and Windows (`README:2518-2584`); kit `Menu` and `context_menu(..)` drawn in-window everywhere (`kit README:66-72, 192-259`). Palette parts: kit `Dialog`, `TextInput`, `PopupList::build_list`, `Combobox` (its own fuzzy ranking), `Keybind` keycaps, `rune_kit::search::Fold` (`crates/rune-kit/src/search.rs`). | macOS menu bar: complete. Windows and Linux: none. Native context menu: macOS and Windows; Linux falls back (`popup_menu_supported()`). There is no command palette component. | A command palette from the parts, fed by the same semantic commands as the menus. A stand-in for the menu bar on Windows and Linux (the palette, or an in-window menu from kit `Menu`). One command registry that fills the macOS menu bar, the palette and the key bindings. |
| 8 | Notifications | `crates/rune-notify`, a separate crate: `register(&Identity)`, `available()`, `post(&Notification::new(id, title).body(..).info(k, v))`, `remove(ids)`, `on_click(f)`, `app_focused()`, `bring_to_front()` (`crates/rune-notify/README.md`, `src/lib.rs:36-365`). macOS `UNUserNotificationCenter`, Windows toasts, Linux `org.freedesktop.Notifications`. A click hands back `Click { id, info }`: the notification's id and the string pairs posted with it, plus `action()`, `input(id)` and on Linux `activation_token()` (`src/lib.rs:95-125`). | Present on all three, each with a condition. macOS: bundled and signed, and the first post asks permission. Windows: `register` writes an `AppUserModelId` under `HKCU` and must run before a window opens; a click after the app quit reaches it only through an activation scheme plus `rune::single_instance`. Linux: needs a notification server; the first call can wait 2.5 s, so make it after the first frame. Notifications are silent. The crate is not in Rune's default members nor in its CI test list, and has no headless stand-in. | Which events post one, and suppressing them while the window is in front (`app_focused()`). Carrying the Session's id in `info` and opening that Session on a click. A seam of its own to test against, since Rune's headless runtime never sees a notification. The bundle on macOS and the identity on Windows. |
| 9 | Files dragged in, file dialogs | Drops: `El::on_drop_files`, `accepts_extensions`, `accepts_drag`, `on_drop_hover`, `tx.drop_point()` (`README:2434-2500`); macOS, Windows, X11, and Wayland since 2026-11-24 (`crates/rune/src/platform/wayland_drop.rs`, `changelog/unreleased/2026-11-24-wayland-file-drop.md`; the README still says Wayland has none). Dialogs: feature `native-file-dialog`; `tx.open_file`, `open_files`, `save_file`, `open_folder` with a callback; rfd on macOS and Windows, the XDG portal on Linux (`README:2354-2406`). Example: `examples/drop-files`. | Complete. Paths only: a drop carries file paths, not bytes or other payloads (#357). One dialog at a time. Both are scriptable headlessly (`drop_files`, `file-dialog`). | Reading the dropped file off the UI thread and making it an Attachment; which kinds are accepted and what a refused drop says; the drop highlight over the composer. |
| 10 | One instance, several windows | `rune::single_instance(id)`: a named pipe per user on Windows, a Unix socket in `$XDG_RUNTIME_DIR` on Linux; a second launch hands its arguments over, brings the first window forward and returns false (`crates/rune/src/open_url/instance.rs:1-61`, `README:998-1011`). macOS: always true, Launch Services reopens the running bundle and `App::on_reopen` hears it (`README:908-922`). Windows: `run_windows`, `App::window`, `rune::open_window(spec)` at run time; each window has its own state type, runtime, focus and clock, sharing one GPU device (`README:2586-2695`, `crates/rune/src/window/spec.rs:1038`). `rune::run` quits when its window closes. | Matches the map's decisions: closing the last window quits, a second launch focuses the first. On macOS it holds only for a bundle. Each window owning its own state fits "each later window is a Client of its own". | Calling `single_instance` first thing in `main` with an id per Channel or build, so side-by-side builds do not hand launches to each other. Nothing for several windows in the first cut beyond not sharing state through globals. |
| 11 | Theme tokens | Core: `rune::tok::theme(dark)`, 16 colours, plus six accents, spacing, radius, type and motion (`crates/rune-core/src/tok.rs:151-169`). Kit: `Kit { Palette, Elevation, Type, Radius, Motion, Glyphs }`, 64 colour roles in `Palette` (`crates/rune-kit/src/tokens/kit.rs:23-231`); `Kit::neutral(dark)` is the plain light and dark (`tokens/neutral.rs:17`); `Kit::from_palette(PaletteSeed { bg, fg, accent })` derives a whole kit from three colours (`tokens/palette.rs:10,110`). `MarkdownStyle::neutral(dark)`, `SyntaxTheme::from_kit`, the diff's colours from the kit. Themes as data: `Theme::from_json(src, dark)` (`crates/rune-core/src/theme.rs`); `themes/opencode` is a worked theme crate. `cx.system_dark()`; `ColorScheme` for window chrome. | Complete, and light and dark are one boolean. Colour changes animate. | A function from the app's theme to a `Kit`, and the discipline that every colour is read from the kit. A later theme then costs one more such function or one JSON document; three colours is the cheapest form. Any role the kit lacks (an Agent's colour, a Marker) is the app's own token beside it. |
| 12 | Fonts | Inter 4.1 is embedded; `AppSpec::fonts` embeds more faces and `AppSpec::families = (ui, mono)` names the slots, each a comma list (`README:454-455, 1731-1766`). The mono slot defaults to the system's (SF Mono, Cascadia Mono then Consolas, fontconfig's `monospace`). `rune::text::add_font(bytes)` at run time, up to 64 MiB (`README:1776-1784`). Fallback: a named family, then the sans slot, then symbol and script fallbacks, system CJK on demand, an emoji family (`README:1733-1735, 1786-1800`). Per-platform rasterisers (`README:139-193`). | Complete for text. For Shaman's Icons: private-use codepoints skip system fallback (`crates/rune-text/src/fonts/system_fonts.rs:255-262`), so a Nerd Font glyph draws only from a bundled face named with `Family::Named(..)` or in a slot's list. That it then draws is inferred from the font path being ordinary; nothing in Rune tests it. A span cannot yet name its own family (#1093). | Bundling a Nerd Font (the symbols-only face is the small one) and drawing an Icon as its own text element beside the Title, which is also what the glossary asks of an Icon. A probe that snaps one Icon Catalog glyph on each platform. The fallback when an Icon is absent. |
| 13 | Accessibility | AccessKit on all three: UI Automation, NSAccessibility, AT-SPI; default feature `accessibility` (`README:3400-3456`, `crates/rune/src/a11y/`). Built only when an assistive client asks. Roles, names, values, states, actions, text fields with caret and selection, live regions, modal scoping. Kit components report theirs, with goldens (`crates/rune-kit/tests/a11y`). `--tree` and `Runtime::accessibility_json()` expose the tree; `Runtime::accessibility_action` drives it. | Real, and testable from a headless test. Three AccessKit crates are patched, and VoiceOver misses headings without the patch (`README:3469`). Not audited against WCAG (#111). A virtual list exposes mounted rows only, with the count and each row's index. Read-only text takes keyboard focus only with `caret_browsing(true)`. | Labels for icon-only controls, roles and names for Transcript rows and Activities, a live region for what arrives while streaming, and a decision on what the first cut promises a screen reader. Hands-on checks with VoiceOver and Narrator: the tree can be asserted headlessly, the reading cannot. |
| 14 | State and threads | One app state `S`; `fn view(&S, &mut Cx<S>) -> El`; handlers get `&mut S` between frames, then the view re-runs and the tree is diffed (`README:10-14`). Everything runs on one UI thread; there is no render thread (`ARCHITECTURE.md:59-63`). From a tokio task: `AppHandle<S>` is `Clone + Send + Sync`, and `handle.update(|s, tx| ..)` runs on the UI thread in call order and commits on the next frame (`crates/rune/src/app.rs:1381-1383`, `README:802-817`); `update_if` skips the commit when it returns false. Or a channel drained in `poll`, woken through the `Waker` from `AppSpec::setup` (`README:648-656`). `rune::Latest` for latest-wins requests. Rune bundles no async runtime. | Simple and sufficient. What re-runs on a change: the whole view, except `memo(id, version, ..)` subtrees whose version is unchanged and virtual rows outside the mounted window. What never re-runs it: hover, press, scroll, `Live`, `LiveText` and `Pixels` updates (`README:13-14, 1206-1208, 1858-1871`). A panic in the view ends the process (#1003). | The bridge from Shaman's client events to `handle.update`, batching a stream's deltas per frame, a revision counter per row for `memo`, and tokio on its own threads. `AGENTS.md` ("Performance") forbids, in the frame loop: any file I/O on the UI thread ("never read, stat, write, sync or rename"), a `poll` that returns true when nothing visible changed, a wake loop with no end, reading whole rects where a size would do, building unmounted rows, unbounded caches, and logs rewritten on a timer. The docs add: no socket, HTTP, database or process call in `view`, a handler, `poll` or `init`; debug builds warn past 8 ms (`site/src/FastApps.tsx`). |
| 15 | Headless tooling | Flags in debug builds, or with the `tooling` feature: `--snap` (PNG at a scene, time, size, scale, theme), `--tree` (elements with rects, colours, text styles, plus the accessibility tree), `--script` (timed pointer, key, type, paste with HTML or PNG, scroll, wheel, `drag_files`, `drop_files`, `menu-pick`, `file-dialog`, `shortcut`, `reopen`, `open_url`, close request, resize), `--list-scenes`, `--trace`, `--video` (`README:3518-3719`). In tests: `App::new(spec).headless_runtime(size, env)`, then `rt.find`, pointer and key input, `rt.frame(dt)`, `rt.elements()`, `rt.accessibility_json()`, `rt.popup_menu()`, `rt.file_dialog()`, `rt.selected_text()`, `clipboard::memory()`, `rt.take_path_requests()` (`README:3721-3779`). Fixed virtual clock, so runs are deterministic. | Can assert: layout, text, colours, focus, the accessibility tree, selection, clipboard contents, scripted drops, menus and dialogs as data, pixels. Cannot assert: a real menu bar or native popup, an OS notification, a real input method, a real drag from another app, window activation or single instance, system materials, presentation timing, or anything `on_ready` starts (it never runs under `headless_runtime`). Pixels come from the host's GPU and font rasteriser, so a golden is per platform (inferred from `README:139-193`). CLI runs need an adapter; Linux without one needs a software Vulkan driver (`README:3504-3516`). | Scenes (`AppSpec::scenes`) for each state worth a picture, a fake Server feeding the app, the choice between asserting trees and comparing pictures, and a GPU adapter on each runner for anything that snaps. |
| 16 | Churn | No releases, every crate `0.0.1`; "pin a rev of `dev`; `dev` is never rewritten" (`README.md`, `AGENTS.md`). `CHANGELOG.md` is one `## dev` section of 477 entries, four headed "Breaking" (`CHANGELOG.md:200, 881, 2207, 2486`); 1,461 more wait in `changelog/unreleased/`. Rules meant to protect consumers: a changelog fragment for anything that can move output or break a build, and twelve literal-built types that only gain builder methods (`AGENTS.md`, "Changes"). | A pinned consumer never breaks; a consumer that moves its pin should expect to. Most entries are additions or output changes (pixels, layout, defaults) rather than compile breaks, but output changes break goldens, and the patch list and toolchain move too. No entry mentions the Rust toolchain; Rune pins 1.99.1 (`rust-toolchain.toml`) against Shaman's 1.98.1. | A policy for the pin: who moves it, how often, and what is run before it moves. Goldens that tolerate a bump. Building Rune's types through constructors and builders wherever one exists, since only the twelve listed types are promised to keep their fields. |

## Open issues that touch a line

Read 2026-11-25; 84 open, 630 closed.

| Line | Issue | What it says |
|---|---|---|
| 1 | #1379 | Sticky headers do not cross `virtual_rows`; the maintainers' agent looked and made no change. |
| 1 | #1384 | Virtual rows are vertical only. Not needed by a Transcript. |
| 2 | #542 | Selectable text over glyphs the app places itself. Not needed. |
| 3, 5 | #350, #967 | No inline image inside a text run; no block widget inside a text field. |
| 4 | #1407 | Split diff cannot leave lines unwrapped. |
| 5 | #669 | The caret never blinks; a fix was "rejected by review". |
| 5 | #1080, #127 | No replacement text or concealed range in a text field. Pills cover Shaman's labels. |
| 5, 6 | #1365 | No key layer for a popup while a field keeps focus. |
| 6 | #2042 | A key sent from a timer poll can wait a frame. |
| 7 | #313, #637 | No window menu on X11; no native message dialog. |
| 9 | #171, #526 | Linux drag and drop. #171's Wayland drop half landed on 2026-11-24 and the issue is still open. |
| 9 | #357, #532 | No drop targets inside the app for payloads other than files; nothing says which action a drop gets. |
| 10 | #22, #174 | No reopen or open-URL hook on Linux; `on_close` cannot cancel (`on_close_request` can). |
| 12 | #1093 | A span's own font family, and fonts per script. |
| 13 | #111, #1680 | No WCAG audit; native child views invisible to VoiceOver. |
| 14 | #1003 | No recovery from a panic in a view. |
| 15 | #296 | A software adapter costs 0.67 CPU-seconds a frame on a tall panel: relevant to runners without a GPU. |
| 16 | #850, #124, #641 | The `winit` patch is a hard build requirement; build time and dependency count; CI runs no clippy. |
| Linux | #591, #2224, #898, #1859, #523 | Double title bar on GNOME, title-bar double-clicks on Wayland, wheel feel, primary selection, clipboard types. |

## Questions this raises

- **How the app is packaged on macOS and identified on Windows.** New, and not on the map. Notifications and
  one instance both need a signed `.app` bundle on macOS, and Windows notifications need an `AppUserModelId`
  registered for the user plus an activation scheme if a click is to work after the app quit. That touches
  where the `shaman` executable the app starts as its Server lives relative to the bundle, what a Keeper is
  registered against, and what the release job signs and notarizes.
- **Two probes for the proof of Rune on three platforms**, each a headless snap or test: one selection dragged
  across two agent Messages in different virtual rows, and one Icon Catalog glyph drawn from a bundled Nerd
  Font.
- **For the key bindings ticket already in the map's fog:** Rune has no key sequences, so the `Ctrl+X` leader
  is either rebuilt by the app or replaced, and `Ctrl+X` is Cut in the composer on Windows and Linux.

## Inferred or not determined

- **Selection across several Markdown Messages.** Not determined. The built element is an `El`, so
  `.selection_group(false)` on it may release its texts to an outer group; nothing in Rune does this.
- **A Nerd Font glyph drawing from a bundled face.** Inferred. The private-use rule only removes system
  fallback; whether a registered face that no family list names is still tried was not determined
  (`Fallbacks::own_families`, `crates/rune-text/src/fonts/system_fonts.rs:206-241`).
- **`App::headless_runtime` needing no GPU.** Inferred from `runtime_for` building a `Runtime` with a
  `TextSystem` and no renderer, and from #669 calling `Runtime::frame` "CPU only".
- **Updates from several `handle.update` calls landing in one commit.** Inferred from "commits on the next
  frame"; not traced through the event loop.
- **IME on Linux.** Rune has platform modules for macOS and Windows only; Linux is winit's. Not examined on a
  desktop.
- **Golden pictures differing per platform.** Inferred from the per-platform rasterisers; Rune's own kit
  comparisons were made on lavapipe (`kit README:639`).
- **Every number about speed or size** is Rune's own claim, read and not measured.
- **How many unreleased fragments break a compile.** Not counted. Thirteen contain the word "breaking"; of the
  six read, two were compile or script breaks and four were output changes or said "without breaking".

## Sources

- `README.md`, `AGENTS.md`, `ARCHITECTURE.md`, `CHANGELOG.md`, `changelog/unreleased/`, `Cargo.toml`,
  `rust-toolchain.toml`, `.github/workflows/macos-tests.yml`
- `crates/rune/README.md`, `crates/rune/src/shortcuts/README.md`, `crates/rune-kit/README.md`,
  `crates/rune-notify/README.md`, `examples/README.md`, `site/src/FastApps.tsx`
- Source files as cited in each row
- Issues of anomalyco/rune by number, and the GitHub API for commit and issue counts, 2026-11-25
