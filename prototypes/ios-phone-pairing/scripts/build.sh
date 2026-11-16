#!/bin/sh
# Builds the spike: the core for the phone or the simulator, its Swift bindings, the Xcode project, then the app.
# Usage: build.sh sim|device [cargo profile, `release` by default; `size` is the shipping-shaped one]
set -eu
here="$(cd "$(dirname "$0")/.." && pwd)"
kind="${1:-sim}"
profile="${2:-release}"
team="${DEVELOPMENT_TEAM:-645L89QC7B}"
bundle="${BUNDLE_ID:-com.jaketucker.shaman-phone-spike}"
case "$kind" in
  sim) triple=aarch64-apple-ios-sim; sdk=iphonesimulator; destination="generic/platform=iOS Simulator" ;;
  device) triple=aarch64-apple-ios; sdk=iphoneos; destination="generic/platform=iOS" ;;
  *) echo "usage: build.sh sim|device [profile]" >&2; exit 2 ;;
esac

cd "$here/core"
export IPHONEOS_DEPLOYMENT_TARGET=18.0
cargo build --lib --target "$triple" --profile "$profile"
library="$here/core/target/$triple/$profile/libshaman_phone_spike.a"

# The generator reads the library built for the Mac; what it writes is the same for every target.
cargo build --lib
cargo run --quiet --features bindgen --bin uniffi-bindgen -- generate target/debug/libshaman_phone_spike.dylib \
  --language swift --no-format --out-dir "$here/app/ffi"
mkdir -p "$here/app/Spike/Generated"
mv "$here/app/ffi/shaman_phone_spike.swift" "$here/app/Spike/Generated/"
mv "$here/app/ffi/shaman_phone_spikeFFI.modulemap" "$here/app/ffi/module.modulemap"

cd "$here/app"
if [ ! -d Spike.xcodeproj ]; then
  xcrun xcodeproj init Spike.xcodeproj
  xcrun xcodeproj target add Spike --product-type application --project Spike.xcodeproj
  for phase in sources frameworks resources; do
    xcrun xcodeproj target phase add --target Spike "$phase" --project Spike.xcodeproj
  done
  xcrun xcodeproj group add Spike --filesystem-path Spike --project Spike.xcodeproj
  xcrun xcodeproj group include --group Spike --target Spike --project Spike.xcodeproj
  for setting in SDKROOT=iphoneos IPHONEOS_DEPLOYMENT_TARGET=18.0 SWIFT_VERSION=6.0 TARGETED_DEVICE_FAMILY=1 \
    INFOPLIST_KEY_UILaunchScreen_Generation=YES "PRODUCT_BUNDLE_IDENTIFIER=$bundle" \
    "INFOPLIST_KEY_NSLocalNetworkUsageDescription=Reaches the Shaman servers on your network that you pair with." \
    INFOPLIST_KEY_UIFileSharingEnabled=YES CODE_SIGN_STYLE=Automatic "DEVELOPMENT_TEAM=$team"; do
    xcrun xcodeproj setting set "${setting%%=*}" "${setting#*=}" --target Spike --project Spike.xcodeproj
  done
fi

mkdir -p "$here/build"
# The library is named by its path: the directory holds a dylib of the same name, which `-l` would prefer.
xcodebuild build -project Spike.xcodeproj -scheme Spike -configuration Release -sdk "$sdk" \
  -destination "$destination" -derivedDataPath "$here/build/$kind" -allowProvisioningUpdates \
  ARCHS=arm64 SWIFT_INCLUDE_PATHS="$here/app/ffi" \
  OTHER_LDFLAGS="$library -framework Security -framework CoreFoundation -liconv" \
  > "$here/build/$kind.log" 2>&1 || true
grep -E "error:|warning:|BUILD (SUCCEEDED|FAILED)" "$here/build/$kind.log" | sort -u | cut -c1-400 | head -40
ls -d "$here/build/$kind/Build/Products/"*/Spike.app
