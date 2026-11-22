shopt -s nullglob 2>/dev/null || setopt nullglob
UDID=C2F3038F-36AD-45F7-B847-F37B000FBE21
NAME="Shaman shaman duo"
APP=/Users/jake/Projects/shaman/.worktrees/prototype-ios-duo-laptop-pose/ios/build/xcode/Build/Products/Debug-iphonesimulator/Shaman.app
BUNDLE=com.jaketucker.shaman
SHOTS=/tmp/duo718/shots
pose() { local n; for n in 1 2 3 4 5; do osascript /tmp/duo718/pose.applescript "$UDID" "$NAME" "$1" 2>/tmp/duo718/pose.err && { sleep "${2:-4}"; return 0; }; sleep 2; done; cat /tmp/duo718/pose.err; return 1; }  # Device Hub answers accessibility only some of the time
active_display() { xcrun devicectl device info displays --device "$UDID" -j - 2>/dev/null | jq -r '.result.displays[] | select(.active) | .displayId'; }
orientation() { xcrun devicectl device orientation get --device "$UDID" 2>/dev/null | tail -1; }
state() { echo "display=$(active_display) $(orientation)"; }
snap() { if [ -n "$PLAIN" ]; then xcrun simctl io "$UDID" screenshot "$SHOTS/$1.png" >/dev/null 2>&1; sips -Z 1000 "$SHOTS/$1.png" --out "$SHOTS/$1.small.png" >/dev/null 2>&1; return; fi; local d; d=$(active_display); xcrun simctl io "$UDID" screenshot --display="$d" "$SHOTS/$1.png" >/dev/null 2>&1; sips -Z 1300 "$SHOTS/$1.png" --out "$SHOTS/$1.small.png" >/dev/null 2>&1; }
launch() { xcrun simctl launch --terminate-running-process "$UDID" "$BUNDLE" "$@" >/dev/null; }
shot() { local stem=$1 wait=$2; shift 2; launch "$@"; sleep "$wait"; snap "$stem"; }
IOS=/Users/jake/Projects/shaman/.worktrees/prototype-ios-duo-laptop-pose/ios
watch_requests() { local f n; rm -f /tmp/duo718/stop-watch; while [ ! -e /tmp/duo718/stop-watch ]; do for f in /tmp/duo718/req/*; do [ -e "$f" ] || continue; n=$(basename "$f"); [ "$n" = hold ] && continue; snap "${PREFIX}$n"; rm -f "$f"; done; sleep 0.3; done; }
uitest() { # uitest <prefix> <variant> [tests...]  (UDID may be overridden)
  PREFIX=$1; local v=$2; shift 2; local only=(); for t in "$@"; do only+=(-only-testing:ShamanUITests/PrototypeWalkTests/$t); done
  rm -f /tmp/duo718/req/*; watch_requests & local w=$!
  (cd $IOS && rm -rf build/results/proto-$PREFIX$v.xcresult && TEST_RUNNER_PROTO_VARIANT=$v xcodebuild test-without-building -project Shaman.xcodeproj -scheme Shaman -destination "platform=iOS Simulator,id=$UDID" -derivedDataPath build/xcode -resultBundlePath build/results/proto-$PREFIX$v.xcresult "${only[@]}" 2>&1 | grep -E "Test Case .*(passed|failed)|error:|\*\* TEST")
  touch /tmp/duo718/stop-watch; wait $w 2>/dev/null
}
held() { PREFIX=$1; local v=$2; rm -f /tmp/duo718/req/*; (uitest "$1" "$v" testHold > /tmp/duo718/held.log 2>&1) & HELD=$!; local n=0; until [ -e /tmp/duo718/req/hold ]; do sleep 1; n=$((n+1)); [ $n -gt 150 ] && echo "never held" && return 1; done; sleep 1; }
release() { rm -f /tmp/duo718/req/hold; wait $HELD 2>/dev/null; grep -E "passed|failed" /tmp/duo718/held.log; }
# --- #717: a test that does what it is told
drive() { # drive <args...>   builds nothing; starts the driven test with these launch arguments
  rm -f /tmp/duo718/ready /tmp/duo718/cmd /tmp/duo718/app.log
  (cd $IOS && rm -rf build/results/driven.xcresult && TEST_RUNNER_PROTO_ARGS="$*" xcodebuild test-without-building -project Shaman.xcodeproj -scheme Shaman -destination "platform=iOS Simulator,id=$UDID" -derivedDataPath build/xcode -resultBundlePath build/results/driven.xcresult -only-testing:ShamanUITests/PrototypeConversationTests/testDriven > /tmp/duo718/driven.log 2>&1) &
  local n=0; until [ -e /tmp/duo718/ready ]; do sleep 1; n=$((n+1)); [ $n -gt 180 ] && echo "never ready" && tail -5 /tmp/duo718/driven.log && return 1; done; sleep ${SETTLE:-3}
}
tell() { echo "$*" > /tmp/duo718/cmd.tmp && mv /tmp/duo718/cmd.tmp /tmp/duo718/cmd; local n=0; while [ -e /tmp/duo718/cmd ]; do sleep 0.3; n=$((n+1)); [ $n -gt 100 ] && echo "not done: $*" && return 1; done; local s; s=$(cat /tmp/duo718/said 2>/dev/null); [ "$s" = done ] || echo "$s"; sleep ${AFTER:-1}; }
quit() { tell quit; wait 2>/dev/null; }
