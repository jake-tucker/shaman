// swift-tools-version: 6.2
// The app's views and everything Swift decides beneath them: the protocol the app reaches the core through, its live
// conformance, the fake, and the adapters around the core. The core itself is `ShamanCore`, which
// `scripts/build-ios-core.sh` writes beside this package, so nothing here builds until that script has run once.
import PackageDescription

let package = Package(
  name: "ShamanUI",
  defaultLocalization: "en",
  platforms: [.iOS("27.0")],
  products: [.library(name: "ShamanUI", targets: ["ShamanUI"])],
  dependencies: [.package(path: "../ShamanCore")],
  targets: [
    .target(
      name: "ShamanUI",
      dependencies: [.product(name: "ShamanCore", package: "ShamanCore")],
      swiftSettings: [.treatAllWarnings(as: .error)]
    ),
    .testTarget(
      name: "ShamanUIUnitTests",
      dependencies: ["ShamanUI"],
      swiftSettings: [.treatAllWarnings(as: .error)]
    ),
  ]
)
