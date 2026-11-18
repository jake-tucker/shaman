#!/usr/bin/env bash
# Builds the thin Client's core for the iOS app and assembles it as the Swift package `ios/ShamanCore/`: the core
# (`crates/shaman-thin-client`) as a static library for the phone, the simulator or both, the Swift UniFFI generates
# for it, and a manifest over the two. Run it before `xcodebuild`, and again after any change to the core; no build
# phase runs it. Nothing it writes is committed.
#
#   scripts/build-ios-core.sh [sim|device|both] [cargo profile]
#
# `sim` is the default. The profile is the one each slice is built with: unoptimised (`dev`) for the simulator and
# size-optimised (`size`) for the device unless given. There is no Intel slice.

set -euo pipefail

kind=${1:-sim}
profile=${2:-}
case "$kind" in
  sim) triples=(aarch64-apple-ios-sim) ;;
  device) triples=(aarch64-apple-ios) ;;
  both) triples=(aarch64-apple-ios aarch64-apple-ios-sim) ;;
  *)
    echo "usage: $0 [sim|device|both] [cargo profile]" >&2
    exit 2
    ;;
esac

root=$(cd "$(dirname "$0")/.." && pwd)
package="$root/ios/ShamanCore"
work="$root/target/ios-core"
cd "$root"

# Matches the app's deployment target, so the library is built for the iOS the app runs on.
export IPHONEOS_DEPLOYMENT_TARGET=27.0

installed=$(rustup target list --installed)
for triple in "${triples[@]}"; do
  grep -qx "$triple" <<< "$installed" || rustup target add "$triple"
done

# The profile's output directory: `dev` writes to `debug`, every other profile to its own name.
profile_for() {
  if [ -n "$profile" ]; then
    echo "$profile"
  elif [ "$1" = aarch64-apple-ios ]; then
    echo size
  else
    echo dev
  fi
}
directory_of() {
  if [ "$1" = dev ]; then echo debug; else echo "$1"; fi
}

libraries=()
for triple in "${triples[@]}"; do
  built=$(profile_for "$triple")
  echo "Building the core for $triple ($built)" >&2
  # A static library here alone: everywhere else the crate is an ordinary library.
  cargo rustc --locked -p shaman-thin-client --lib --crate-type staticlib --target "$triple" --profile "$built"
  libraries+=("$root/target/$triple/$(directory_of "$built")/libshaman_thin_client.a")
done

rm -rf "$work"
mkdir -p "$work/generated" "$work/headers"
echo "Generating the Swift" >&2
# Read from the library just built, so the Swift matches it whatever profile it was built with.
cargo run --locked --quiet -p shaman-thin-client --features bindgen --bin uniffi-bindgen-swift -- \
  "${libraries[0]}" "$work/generated" --swift-sources --headers --modulemap \
  --module-name ShamanCoreFFI --modulemap-filename module.modulemap
mv "$work/generated/ShamanCoreFFI.h" "$work/generated/module.modulemap" "$work/headers/"

rm -rf "$package"
mkdir -p "$package/Sources/ShamanCore"
framework=()
for library in "${libraries[@]}"; do
  framework+=(-library "$library" -headers "$work/headers")
done
xcodebuild -create-xcframework "${framework[@]}" -output "$package/ShamanCoreFFI.xcframework" > /dev/null
mv "$work/generated/ShamanCore.swift" "$package/Sources/ShamanCore/"

cat > "$package/Package.swift" << 'EOF'
// swift-tools-version: 6.2
// Written by scripts/build-ios-core.sh, and replaced whole each time it runs; not committed.
import PackageDescription

let package = Package(
  name: "ShamanCore",
  platforms: [.iOS("27.0")],
  products: [.library(name: "ShamanCore", targets: ["ShamanCore"])],
  targets: [
    .binaryTarget(name: "ShamanCoreFFI", path: "ShamanCoreFFI.xcframework"),
    .target(
      name: "ShamanCore",
      dependencies: ["ShamanCoreFFI"],
      // What the core's dependencies call in the system: the trust store and the proxy settings.
      linkerSettings: [
        .linkedFramework("Security"),
        .linkedFramework("SystemConfiguration"),
        .linkedFramework("CoreFoundation"),
        .linkedLibrary("iconv"),
      ]
    ),
  ]
)
EOF
echo "Assembled $package" >&2
