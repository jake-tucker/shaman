# Scratch helpers for #733. Source it: `. /tmp/duo733/lib.sh`; UDID chooses the simulator, the Duo unless said.
D=/tmp/duo733
IOS=/Users/jake/Projects/shaman/.worktrees/prototype-ios-duo-sidebar/ios
APP=$IOS/build/xcode/Build/Products/Debug-iphonesimulator/Shaman.app
BUNDLE=com.jaketucker.shaman
. $D/sims.env
DUONAME="Shaman prototype-ios-duo-sidebar duo"
UDID=${UDID:-$DUO}
# build [build|build-for-testing]: one build, which the driven test also runs from.
build() { (cd $IOS && xcodebuild ${1:-build-for-testing} -project Shaman.xcodeproj -scheme Shaman -destination "platform=iOS Simulator,id=$DUO" -derivedDataPath build/xcode 2>&1 | grep -E "error:|BUILD|TEST BUILD" | sort -u | cut -c1-400 | head -30); }
install() { local u; for u in "${@:-$UDID}"; do xcrun simctl install "$u" "$APP"; done; }
launch() { : > $D/app.log; xcrun simctl launch --terminate-running-process "$UDID" "$BUNDLE" "$@" >/dev/null; }
pose() { local n; for n in 1 2 3 4 5; do osascript $D/pose.applescript "$DUO" "$DUONAME" "$1" 2>$D/pose.err >/dev/null && { sleep "${2:-4}"; return 0; }; sleep 2; done; cat $D/pose.err; return 1; }
active_display() { xcrun devicectl device info displays --device "$UDID" -j - 2>/dev/null | jq -r '.result.displays[] | select(.active) | .displayId'; }
snap() { local d; if [ "$UDID" = "$DUO" ]; then d=$(active_display); xcrun simctl io "$UDID" screenshot --display="$d" "$D/shots/$1.png" >/dev/null 2>&1; else xcrun simctl io "$UDID" screenshot "$D/shots/$1.png" >/dev/null 2>&1; fi; sips -Z ${2:-900} "$D/shots/$1.png" --out "$D/shots/$1.small.png" >/dev/null 2>&1; }
mark() { echo "------- $*" >> $D/app.log; }
# poke <line>: has the running app do it (see PrototypeSidebar.swift), and waits for it to be taken.
poke() { echo "$*" > $D/poke.tmp && mv $D/poke.tmp $D/poke; local n=0; while [ -e $D/poke ]; do sleep 0.1; n=$((n+1)); [ $n -gt 50 ] && echo "not taken: $*" && return 1; done; sleep ${AFTER:-1}; }
# where: the split view's own account, last line.
where() { poke dump >/dev/null; grep "poke: after" $D/app.log | tail -1 | sed 's/.*poke: after //'; }
# drive <launch args...>: starts the driven test on $UDID; tell <command>; quit.
drive() { rm -f $D/ready $D/cmd $D/said; : > $D/app.log
  (cd $IOS && TEST_RUNNER_PROTO_ARGS="$*" xcodebuild test-without-building -project Shaman.xcodeproj -scheme Shaman -destination "platform=iOS Simulator,id=$UDID" -derivedDataPath build/xcode -resultBundlePath build/results/driven-$(date +%s).xcresult -only-testing:ShamanUITests/PrototypeRealTests/testDriven > $D/driven.log 2>&1) &
  local n=0; until [ -e $D/ready ]; do sleep 1; n=$((n+1)); [ $n -gt 240 ] && echo "never ready" && tail -5 $D/driven.log && return 1; done; sleep ${SETTLE:-3}; }
tell() { echo "$*" > $D/cmd.tmp && mv $D/cmd.tmp $D/cmd; local n=0; while [ -e $D/cmd ]; do sleep 0.3; n=$((n+1)); [ $n -gt 100 ] && echo "not done: $*" && return 1; done; local s; s=$(cat $D/said 2>/dev/null); [ "$s" = done ] || echo "$s"; sleep ${AFTER:-1}; }
quit() { tell quit; wait 2>/dev/null; }
# config <udid> <words...>: what the app reads where no launch argument says, so the suite runs unchanged.
config() { local u=$1; shift; echo "$*" > $D/config-$u; }
# suite <name> <udid>: #731's eight tests on that simulator, into logs/<name>.log.
suite() { (cd $IOS && xcodebuild test-without-building -project Shaman.xcodeproj -scheme Shaman -destination "platform=iOS Simulator,id=$2" -derivedDataPath build/xcode -resultBundlePath build/results/$1-$(date +%s).xcresult -only-testing:ShamanUITests/PrototypeRealTests -skip-testing:ShamanUITests/PrototypeRealTests/testDriven 2>&1 | grep -E "Test Case .*(passed|failed|skipped)|error:|\*\* TEST" | cut -c1-400 > $D/logs/$1.log; tail -1 $D/logs/$1.log); }
