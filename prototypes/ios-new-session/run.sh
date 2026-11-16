#!/bin/sh
# PROTOTYPE — throwaway. Builds the beginning-a-Session prototype and runs it on a simulator.
#   ./run.sh                  # variant A, on the Session listing
#   ./run.sh B                # variant A|B|C, or W for what won; the yellow pill flips them too
#   ./run.sh A -compose YES -draft "Trim the QR code's quiet zone" -send YES -start slow
# Anything after the variant is passed as launch arguments:
#   -compose YES              do what the compose button does
#   -draft "<text>"           the first Prompt, already written
#   -worktree YES             ask for a new Worktree
#   -picker YES               open the Workspace picker (A and B)
#   -send YES                 send the draft
#   -start quick|slow|refused how the toy Server answers
#   -scenario oneServer|threeServers|unreachable|empty   -outlook lab/relay-infra
set -eu
cd "$(dirname "$0")"

# A simulator's UDID; the first iPhone 18 Pro unless DEVICE names another.
DEVICE="${DEVICE:-$(xcrun simctl list devices available | sed -n 's/^ *iPhone 18 Pro (\([0-9A-F-]*\)).*/\1/p' | head -1)}"
BUNDLE=dev.shaman.prototype.newsession
BUILD="${TMPDIR:-/tmp}/shaman-new-session-prototype"
t=NewSessionPrototype

if [ ! -d $t.xcodeproj ]; then
  xcrun xcodeproj init $t.xcodeproj
  xcrun xcodeproj target add $t --product-type application
  for phase in sources frameworks resources; do
    xcrun xcodeproj target phase add --target $t $phase
  done
  xcrun xcodeproj group add $t --filesystem-path $t
  xcrun xcodeproj group include --group $t --target $t
  for kv in SDKROOT=iphoneos IPHONEOS_DEPLOYMENT_TARGET=26.0 PRODUCT_BUNDLE_IDENTIFIER=$BUNDLE \
    SWIFT_VERSION=5.0 TARGETED_DEVICE_FAMILY=1 INFOPLIST_KEY_UILaunchScreen_Generation=YES; do
    xcrun xcodeproj setting set "${kv%%=*}" "${kv#*=}" --target $t
  done
fi

xcrun simctl boot "$DEVICE" 2>/dev/null || true
# Xcode 27 shows simulators in DeviceHub; there is no Simulator.app.
[ -n "${QUIET:-}" ] || open -b com.apple.dt.Devices || true
xcodebuild build -quiet -project $t.xcodeproj -scheme $t \
  -destination "platform=iOS Simulator,id=$DEVICE" -derivedDataPath "$BUILD"
xcrun simctl install "$DEVICE" "$BUILD/Build/Products/Debug-iphonesimulator/$t.app"
xcrun simctl terminate "$DEVICE" $BUNDLE 2>/dev/null || true
variant="${1:-A}"
[ $# -ge 1 ] && shift
xcrun simctl launch "$DEVICE" $BUNDLE -variant "$variant" "$@"
