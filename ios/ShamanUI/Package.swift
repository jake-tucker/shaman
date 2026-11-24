// swift-tools-version: 6.2
// The app's views and everything Swift decides beneath them: the protocol the app reaches the core through, its live
// conformance, the fake, and the adapters around the core. The core itself is `ShamanCore`, which
// `scripts/build-ios-core.sh` writes beside this package, so nothing here builds until that script has run once.
import PackageDescription

let package = Package(
  name: "ShamanUI",
  defaultLocalization: "en",
  platforms: [.iOS("27.1")],
  products: [.library(name: "ShamanUI", targets: ["ShamanUI"])],
  dependencies: [
    .package(path: "../ShamanCore"),
    // What draws an agent Message's Markdown: GitHub-flavoured tables and fenced code, each scrolling sideways, over
    // SwiftUI's own Text, so a streaming Message re-renders cheaply. Pinned exactly; move it deliberately.
    .package(url: "https://github.com/gonzalezreal/textual", exact: "0.5.0"),
  ],
  targets: [
    .target(
      name: "ShamanUI",
      dependencies: [
        .product(name: "ShamanCore", package: "ShamanCore"),
        .product(name: "Textual", package: "textual"),
      ],
      // The Symbols Nerd Font every Icon is drawn from (Nerd Fonts 3.5.1, whole), with the licence it ships under.
      // `crates/shaman-icons` reads the same file to see it draws every glyph the core resolves.
      resources: [.copy("Resources/Fonts")],
      swiftSettings: [.treatAllWarnings(as: .error)]
    ),
    .testTarget(
      name: "ShamanUIUnitTests",
      dependencies: ["ShamanUI"],
      swiftSettings: [.treatAllWarnings(as: .error)]
    ),
  ]
)
