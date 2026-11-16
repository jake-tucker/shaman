#!/bin/sh
# PROTOTYPE — throwaway. Builds the Session listing prototype and runs it on a simulator.
#   ./run.sh                      # variant A, three servers
#   ./run.sh B unreachable        # variant A|B|C, then a scenario from Model.swift
set -eu
cd "$(dirname "$0")"

# A simulator's UDID; the first iPhone 18 Pro unless DEVICE names another.
DEVICE="${DEVICE:-$(xcrun simctl list devices available | sed -n 's/^ *iPhone 18 Pro (\([0-9A-F-]*\)).*/\1/p' | head -1)}"
BUNDLE=dev.shaman.prototype.sessionlist
BUILD="${TMPDIR:-/tmp}/shaman-session-list-prototype"

if [ ! -d SessionListPrototype.xcodeproj ]; then
  t=SessionListPrototype
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
open -b com.apple.dt.Devices || true
xcodebuild build -quiet -project SessionListPrototype.xcodeproj -scheme SessionListPrototype \
  -destination "platform=iOS Simulator,id=$DEVICE" -derivedDataPath "$BUILD"
xcrun simctl install "$DEVICE" "$BUILD/Build/Products/Debug-iphonesimulator/SessionListPrototype.app"
xcrun simctl terminate "$DEVICE" $BUNDLE 2>/dev/null || true
xcrun simctl launch "$DEVICE" $BUNDLE -variant "${1:-A}" -scenario "${2:-threeServers}"
