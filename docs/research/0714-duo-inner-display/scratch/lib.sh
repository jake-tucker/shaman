shopt -s nullglob 2>/dev/null || setopt nullglob
UDID=C2F3038F-36AD-45F7-B847-F37B000FBE21
NAME="Shaman shaman duo"
APP=/Users/jake/Projects/shaman/.worktrees/prototype-ios-duo-inner-display/ios/build/xcode/Build/Products/Debug-iphonesimulator/Shaman.app
BUNDLE=com.jaketucker.shaman
SHOTS=/tmp/duo714/shots
pose() { osascript /tmp/duo714/pose.applescript "$UDID" "$NAME" "$1" && sleep "${2:-4}"; }
active_display() { xcrun devicectl device info displays --device "$UDID" -j - 2>/dev/null | jq -r '.result.displays[] | select(.active) | .displayId'; }
orientation() { xcrun devicectl device orientation get --device "$UDID" 2>/dev/null | tail -1; }
state() { echo "display=$(active_display) $(orientation)"; }
snap() { if [ -n "$PLAIN" ]; then xcrun simctl io "$UDID" screenshot "$SHOTS/$1.png" >/dev/null 2>&1; sips -Z 1000 "$SHOTS/$1.png" --out "$SHOTS/$1.small.png" >/dev/null 2>&1; return; fi; local d; d=$(active_display); xcrun simctl io "$UDID" screenshot --display="$d" "$SHOTS/$1.png" >/dev/null 2>&1; sips -Z 1300 "$SHOTS/$1.png" --out "$SHOTS/$1.small.png" >/dev/null 2>&1; }
launch() { xcrun simctl launch --terminate-running-process "$UDID" "$BUNDLE" "$@" >/dev/null; }
shot() { local stem=$1 wait=$2; shift 2; launch "$@"; sleep "$wait"; snap "$stem"; }
IOS=/Users/jake/Projects/shaman/.worktrees/prototype-ios-duo-inner-display/ios
watch_requests() { local f n; rm -f /tmp/duo714/stop-watch; while [ ! -e /tmp/duo714/stop-watch ]; do for f in /tmp/duo714/req/*; do [ -e "$f" ] || continue; n=$(basename "$f"); [ "$n" = hold ] && continue; snap "${PREFIX}$n"; rm -f "$f"; done; sleep 0.3; done; }
uitest() { # uitest <prefix> <variant> [tests...]  (UDID may be overridden)
  PREFIX=$1; local v=$2; shift 2; local only=(); for t in "$@"; do only+=(-only-testing:ShamanUITests/PrototypeWalkTests/$t); done
  rm -f /tmp/duo714/req/*; watch_requests & local w=$!
  (cd $IOS && rm -rf build/results/proto-$PREFIX$v.xcresult && TEST_RUNNER_PROTO_VARIANT=$v xcodebuild test-without-building -project Shaman.xcodeproj -scheme Shaman -destination "platform=iOS Simulator,id=$UDID" -derivedDataPath build/xcode -resultBundlePath build/results/proto-$PREFIX$v.xcresult "${only[@]}" 2>&1 | grep -E "Test Case .*(passed|failed)|error:|\*\* TEST")
  touch /tmp/duo714/stop-watch; wait $w 2>/dev/null
}
held() { PREFIX=$1; local v=$2; rm -f /tmp/duo714/req/*; (uitest "$1" "$v" testHold > /tmp/duo714/held.log 2>&1) & HELD=$!; local n=0; until [ -e /tmp/duo714/req/hold ]; do sleep 1; n=$((n+1)); [ $n -gt 150 ] && echo "never held" && return 1; done; sleep 1; }
release() { rm -f /tmp/duo714/req/hold; wait $HELD 2>/dev/null; grep -E "passed|failed" /tmp/duo714/held.log; }
