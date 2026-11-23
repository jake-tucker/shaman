# Scratch helpers for #731. Source it: `. /tmp/duo731/lib.sh`; UDID chooses the simulator.
D=/tmp/duo731
IOS=/Users/jake/Projects/shaman/.worktrees/prototype-ios-duo-real-screens/ios
APP=$IOS/build/xcode/Build/Products/Debug-iphonesimulator/Shaman.app
BUNDLE=com.jaketucker.shaman
. $D/sims.env
DUONAME="Shaman prototype-ios-duo-real-screens duo"
UDID=${UDID:-$PRO}
build() { (cd $IOS && xcodebuild ${1:-build} -project Shaman.xcodeproj -scheme Shaman -destination "platform=iOS Simulator,id=$PRO" -derivedDataPath build/xcode 2>&1 | grep -E "error:|BUILD|TEST BUILD" | sort -u | cut -c1-400 | head -30); }
install() { local u; for u in "${@:-$UDID}"; do xcrun simctl install "$u" "$APP"; done; }
launch() { : > $D/app.log; xcrun simctl launch --terminate-running-process "$UDID" "$BUNDLE" "$@" >/dev/null; }
pose() { local n; for n in 1 2 3 4 5; do osascript $D/pose.applescript "$DUO" "$DUONAME" "$1" 2>$D/pose.err >/dev/null && { sleep "${2:-4}"; return 0; }; sleep 2; done; cat $D/pose.err; return 1; }
active_display() { xcrun devicectl device info displays --device "$UDID" -j - 2>/dev/null | jq -r '.result.displays[] | select(.active) | .displayId'; }
snap() { local d; if [ "$UDID" = "$DUO" ]; then d=$(active_display); xcrun simctl io "$UDID" screenshot --display="$d" "$D/shots/$1.png" >/dev/null 2>&1; else xcrun simctl io "$UDID" screenshot "$D/shots/$1.png" >/dev/null 2>&1; fi; sips -Z ${2:-900} "$D/shots/$1.png" --out "$D/shots/$1.small.png" >/dev/null 2>&1; }
mark() { echo "------- $*" >> $D/app.log; }
# config <udid> <words...>: what the app reads where no launch argument says, so the suite runs unchanged.
config() { local u=$1; shift; echo "$*" > $D/config-$u; }
# suite <name> [-only-testing:...]: the app's tests on $UDID, into build/results/<name>.xcresult.
suite() { local name=$1; shift; (cd $IOS && xcodebuild test-without-building -project Shaman.xcodeproj -scheme Shaman -destination "platform=iOS Simulator,id=$UDID" -derivedDataPath build/xcode -resultBundlePath build/results/$name-$(date +%s).xcresult "$@" 2>&1 | grep -E "Test Case .*(passed|failed)|error:|\*\* TEST" | cut -c1-400 > $D/logs/$name.log; tail -1 $D/logs/$name.log); }
# A second build, apart from the one the suites run from: build2 [build-for-testing]; APP2 is its app.
APP2=$IOS/build/xcode2/Build/Products/Debug-iphonesimulator/Shaman.app
build2() { (cd $IOS && nice -n 5 xcodebuild ${1:-build-for-testing} -project Shaman.xcodeproj -scheme Shaman -destination "platform=iOS Simulator,id=$MAX" -derivedDataPath build/xcode2 2>&1 | grep -E "error:|BUILD|TEST BUILD" | sort -u | cut -c1-400 | head -30); }
# drive <launch args...>: starts the driven test on $UDID from the second build; tell <command>; quit.
drive() { rm -f $D/ready $D/cmd $D/said; : > $D/app.log
  (cd $IOS && TEST_RUNNER_PROTO_ARGS="$*" xcodebuild test-without-building -project Shaman.xcodeproj -scheme Shaman -destination "platform=iOS Simulator,id=$UDID" -derivedDataPath build/xcode2 -resultBundlePath build/results/driven-$(date +%s).xcresult -only-testing:ShamanUITests/PrototypeRealTests/testDriven > $D/driven.log 2>&1) &
  local n=0; until [ -e $D/ready ]; do sleep 1; n=$((n+1)); [ $n -gt 240 ] && echo "never ready" && tail -5 $D/driven.log && return 1; done; sleep ${SETTLE:-3}; }
tell() { echo "$*" > $D/cmd.tmp && mv $D/cmd.tmp $D/cmd; local n=0; while [ -e $D/cmd ]; do sleep 0.3; n=$((n+1)); [ $n -gt 100 ] && echo "not done: $*" && return 1; done; local s; s=$(cat $D/said 2>/dev/null); [ "$s" = done ] || echo "$s"; sleep ${AFTER:-1}; }
quit() { tell quit; wait 2>/dev/null; }
