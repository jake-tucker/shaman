shopt -s nullglob 2>/dev/null || setopt nullglob
UDID=C2F3038F-36AD-45F7-B847-F37B000FBE21
NAME="Shaman shaman duo"
APP=/Users/jake/Projects/shaman/.worktrees/prototype-ios-duo-sheets/ios/build/xcode/Build/Products/Debug-iphonesimulator/Shaman.app
BUNDLE=com.jaketucker.shaman
SHOTS=/tmp/duo719/shots
pose() { local n; for n in 1 2 3 4 5; do osascript /tmp/duo719/pose.applescript "$UDID" "$NAME" "$1" 2>/tmp/duo719/pose.err && { sleep "${2:-4}"; return 0; }; sleep 2; done; cat /tmp/duo719/pose.err; return 1; }  # Device Hub answers accessibility only some of the time
active_display() { xcrun devicectl device info displays --device "$UDID" -j - 2>/dev/null | jq -r '.result.displays[] | select(.active) | .displayId'; }
orientation() { xcrun devicectl device orientation get --device "$UDID" 2>/dev/null | tail -1; }
state() { echo "display=$(active_display) $(orientation)"; }
snap() { if [ -n "$PLAIN" ]; then xcrun simctl io "$UDID" screenshot "$SHOTS/$1.png" >/dev/null 2>&1; sips -Z 1000 "$SHOTS/$1.png" --out "$SHOTS/$1.small.png" >/dev/null 2>&1; return; fi; local d; d=$(active_display); xcrun simctl io "$UDID" screenshot --display="$d" "$SHOTS/$1.png" >/dev/null 2>&1; sips -Z 1300 "$SHOTS/$1.png" --out "$SHOTS/$1.small.png" >/dev/null 2>&1; }
launch() { xcrun simctl launch --terminate-running-process "$UDID" "$BUNDLE" "$@" >/dev/null; }
shot() { local stem=$1 wait=$2; shift 2; launch "$@"; sleep "$wait"; snap "$stem"; }
IOS=/Users/jake/Projects/shaman/.worktrees/prototype-ios-duo-sheets/ios
watch_requests() { local f n; rm -f /tmp/duo719/stop-watch; while [ ! -e /tmp/duo719/stop-watch ]; do for f in /tmp/duo719/req/*; do [ -e "$f" ] || continue; n=$(basename "$f"); [ "$n" = hold ] && continue; snap "${PREFIX}$n"; rm -f "$f"; done; sleep 0.3; done; }
uitest() { # uitest <prefix> <variant> [tests...]  (UDID may be overridden)
  PREFIX=$1; local v=$2; shift 2; local only=(); for t in "$@"; do only+=(-only-testing:ShamanUITests/PrototypeWalkTests/$t); done
  rm -f /tmp/duo719/req/*; watch_requests & local w=$!
  (cd $IOS && rm -rf build/results/proto-$PREFIX$v.xcresult && TEST_RUNNER_PROTO_VARIANT=$v xcodebuild test-without-building -project Shaman.xcodeproj -scheme Shaman -destination "platform=iOS Simulator,id=$UDID" -derivedDataPath build/xcode -resultBundlePath build/results/proto-$PREFIX$v.xcresult "${only[@]}" 2>&1 | grep -E "Test Case .*(passed|failed)|error:|\*\* TEST")
  touch /tmp/duo719/stop-watch; wait $w 2>/dev/null
}
held() { PREFIX=$1; local v=$2; rm -f /tmp/duo719/req/*; (uitest "$1" "$v" testHold > /tmp/duo719/held.log 2>&1) & HELD=$!; local n=0; until [ -e /tmp/duo719/req/hold ]; do sleep 1; n=$((n+1)); [ $n -gt 150 ] && echo "never held" && return 1; done; sleep 1; }
release() { rm -f /tmp/duo719/req/hold; wait $HELD 2>/dev/null; grep -E "passed|failed" /tmp/duo719/held.log; }
# --- #717: a test that does what it is told
drive() { # drive <args...>   builds nothing; starts the driven test with these launch arguments
  rm -f /tmp/duo719/ready /tmp/duo719/cmd /tmp/duo719/app.log
  (cd $IOS && rm -rf build/results/driven.xcresult && TEST_RUNNER_PROTO_ARGS="$*" xcodebuild test-without-building -project Shaman.xcodeproj -scheme Shaman -destination "platform=iOS Simulator,id=$UDID" -derivedDataPath build/xcode -resultBundlePath build/results/driven.xcresult -only-testing:ShamanUITests/PrototypeConversationTests/testDriven > /tmp/duo719/driven.log 2>&1) &
  local n=0; until [ -e /tmp/duo719/ready ]; do sleep 1; n=$((n+1)); [ $n -gt 180 ] && echo "never ready" && tail -5 /tmp/duo719/driven.log && return 1; done; sleep ${SETTLE:-3}
}
tell() { echo "$*" > /tmp/duo719/cmd.tmp && mv /tmp/duo719/cmd.tmp /tmp/duo719/cmd; local n=0; while [ -e /tmp/duo719/cmd ]; do sleep 0.3; n=$((n+1)); [ $n -gt 100 ] && echo "not done: $*" && return 1; done; local s; s=$(cat /tmp/duo719/said 2>/dev/null); [ "$s" = done ] || echo "$s"; sleep ${AFTER:-1}; }
quit() { tell quit; wait 2>/dev/null; }
# --- #719
appbuild() { (cd $IOS && xcodebuild build-for-testing -project Shaman.xcodeproj -scheme Shaman -destination "platform=iOS Simulator,id=$UDID" -derivedDataPath build/xcode -quiet 2>&1 | grep -E "error|BUILD" ; xcrun simctl install "$UDID" "$APP"); }
applog() { cat /tmp/duo719/app.log 2>/dev/null; }
clearlog() { rm -f /tmp/duo719/app.log; }
jpg() { sips -s format jpeg -s formatOptions 80 "$1" --out "$2" >/dev/null 2>&1; }
# cap <name>: a screenshot, where the test says the sheet's parts stand, and where the app last said each sheet stood
cap() { sleep ${CAPWAIT:-1.5}; snap "$1"; { tell frames; echo "--- app"; applog | grep -E "sheet .* safe|questionnaire dete" | tail -4; } > /tmp/duo719/frames/$1.txt 2>&1; }
relaunch() { clearlog; tell relaunch "$@" --inner-pill off; sleep ${LAUNCHWAIT:-4}; }
# contact <out stem> <columns> <cell width> <caption>=<shot stem>...   a contact sheet as a JPEG under /tmp/duo719/sheets
contact() { local out=$1 cols=$2 w=$3; shift 3; local a=(); for c in "$@"; do a+=("${c%%=*}=$SHOTS/${c#*=}.png"); done; mkdir -p /tmp/duo719/sheets; /tmp/duo719/sheet /tmp/duo719/sheets/$out.png $cols $w "${a[@]}" && jpg /tmp/duo719/sheets/$out.png /tmp/duo719/sheets/$out.jpg && rm /tmp/duo719/sheets/$out.png; }
walksheet() { local n=$1 t=$2; contact c-$n-through-poses 4 520 "$t: 1 closed=c-$n-1-closed" "2 open flat=c-$n-2-flat" "3 book=c-$n-3-book" "4 flat again=c-$n-4-flat" "5 laptop pose=c-$n-5-laptop" "6a open, still turned=c-$n-6a-open-portrait" "6 flat again=c-$n-6-flat" "7 closed again=c-$n-7-closed"; }
