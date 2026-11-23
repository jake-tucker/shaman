# Scratch for #729. Source it: `. docs/research/0729-duo-foot/scratch/lib.sh`. The app writes /tmp/duo729/<name>.log
# and reads commands from /tmp/duo729/<name>.cmd; screenshots go to the directory /tmp/duo729/workdir names.
D=/tmp/duo729
W=$(cat $D/workdir)
HERE=/Users/jake/Projects/shaman/.worktrees/prototype-ios-duo-foot
IOS=$HERE/ios
SCRATCH=$HERE/docs/research/0729-duo-foot/scratch
APP=$IOS/build/xcode/Build/Products/Debug-iphonesimulator/Shaman.app
BUNDLE=com.jaketucker.shaman
PRO=245F8377-2CB2-4CEF-A6CC-AEE1FADE24AE
MAX=0634E966-11C4-4F25-BE04-872119F786F3
DUO=C2F3038F-36AD-45F7-B847-F37B000FBE21
DUONAME="Shaman shaman duo"
UDID=${UDID:-$DUO}
LOG=${LOG:-duo}
build() { (cd $IOS && xcodebuild build -project Shaman.xcodeproj -scheme Shaman -destination "platform=iOS Simulator,id=$PRO" -derivedDataPath build/xcode 2>&1 | grep -E "error:|BUILD" | sort -u | cut -c1-400 | head -30); }
install() { xcrun simctl install "$UDID" "$APP"; }
# launch <foot variant> <more launch arguments...>: a fresh log each time, the last kept as <name>.log.
launch() { local v=$1; shift; rm -f $D/$LOG.cmd; : > $D/$LOG.log; xcrun simctl launch --terminate-running-process "$UDID" "$BUNDLE" --fake paired --lab uikit --lab-aside inspector --foot "$v" --foot-log "$LOG" "$@" >/dev/null; }
front() { xcrun simctl launch "$UDID" "$BUNDLE" >/dev/null; }
away() { xcrun simctl launch "$UDID" com.apple.Preferences >/dev/null; }
# cmd <words>: one command to the app, taken within a few frames.
cmd() { local n=0; while [ -e $D/$LOG.cmd ] && [ $n -lt 40 ]; do sleep 0.05; n=$((n+1)); done; echo "$*" > $D/$LOG.cmd.tmp && mv $D/$LOG.cmd.tmp $D/$LOG.cmd; sleep ${PAUSE:-0.4}; }
mark() { echo "------- mac: $*" >> $D/$LOG.log; }
keep() { cp $D/$LOG.log "$HERE/docs/research/0729-duo-foot/logs/$1.log"; }
pose() { local n; for n in 1 2 3 4 5 6; do osascript $SCRATCH/pose.applescript "$DUO" "$DUONAME" "$1" 2>$D/pose.err >/dev/null && { sleep "${2:-4}"; return 0; }; sleep 2; done; cat $D/pose.err; return 1; }
turn() { xcrun devicectl device orientation set --device "$UDID" "$1" >/dev/null 2>&1; sleep "${2:-3}"; }
facing() { xcrun devicectl device orientation get --device "$UDID" 2>/dev/null | tail -1; }
active_display() { xcrun devicectl device info displays --device "$UDID" -j - 2>/dev/null | jq -r '.result.displays[] | select(.active) | .displayId'; }
snap() { if [ "$UDID" = "$DUO" ]; then xcrun simctl io "$UDID" screenshot --display="$(active_display)" "$W/$1.png" >/dev/null 2>&1; else xcrun simctl io "$UDID" screenshot "$W/$1.png" >/dev/null 2>&1; fi; }
# burst <name> <count>: screenshots one after another, as fast as simctl takes them.
burst() { local d=""; [ "$UDID" = "$DUO" ] && d="--display=$(active_display)"; local i; for i in $(seq 1 ${2:-6}); do xcrun simctl io "$UDID" screenshot $d "$W/$1-$i.png" >/dev/null 2>&1; done; }
# uitest <tag> <test>: one of PrototypeFootTests on $UDID; what it says goes to /tmp/duo729/<tag>.txt.
uitest() { (cd $IOS && rm -rf build/results/foot-$1.xcresult && TEST_RUNNER_FOOT_TAG="$1" xcodebuild test-without-building -project Shaman.xcodeproj -scheme Shaman -destination "platform=iOS Simulator,id=$UDID" -derivedDataPath build/xcode -resultBundlePath build/results/foot-$1.xcresult -only-testing:ShamanUITests/PrototypeFootTests/$2 2>&1 | grep -E "Test Case .*(passed|failed)|error:|\*\* TEST" | cut -c1-400); }
# strip <png> <points> [scale]: the bottom <points> points of a screenshot, as <name>.strip.png beside it.
strip() { $D/crop "$1" "${1%.png}.strip.png" $(( $2 * ${3:-3} )); echo "${1%.png}.strip.png"; }
# sheet <name> <columns> <cell width> <png>...: a contact sheet as sheets/<name>.jpg.
sheet() { local name=$1; shift; $D/sheet $W/$name.png "$@" && sips -s format jpeg -s formatOptions 80 $W/$name.png --out $HERE/docs/research/0729-duo-foot/sheets/$name.jpg >/dev/null; }
MAINAPP=/Users/jake/Projects/shaman/ios/build/xcode/Build/Products/Debug-iphonesimulator/Shaman.app
# real <tag>: on $UDID in the pose it stands in, the real app's Session screen and new-session screen (set
# AutomaticMinimizationEnabled true first, so the new-session screen's focused field keeps its keyboard off screen), first from main's own build (built 22 Nov, after main's last change under ios/) and then from this branch's.
real() {
  local which app
  for which in main branch; do
    app=$MAINAPP; [ $which = branch ] && app=$APP
    xcrun simctl install "$UDID" "$app"
    xcrun simctl launch --terminate-running-process "$UDID" "$BUNDLE" --fake paired --open workstation-key/7 >/dev/null; sleep ${REALWAIT:-8}; snap real-$which-$1-session
    xcrun simctl launch --terminate-running-process "$UDID" "$BUNDLE" --fake composing --compose >/dev/null; sleep ${REALWAIT:-8}; snap real-$which-$1-compose
  done
}
# compose <tag>: on $UDID, the new-session screen with its keyboard dragged down, first from main's build (a copy
# installed under the bundle identifier com.jaketucker.shaman.main, since a test run puts this branch's build back
# under the app's own) and then from this branch's, by PrototypeFootTests.testComposeKeyboardDown; screenshots
# real-<build>-<tag>-compose.png.
MAINCOPY=$D/mainapp/Shaman.app
maincopy() { [ -d $MAINCOPY ] && return; mkdir -p $D/mainapp && cp -R $MAINAPP $MAINCOPY && /usr/libexec/PlistBuddy -c "Set :CFBundleIdentifier com.jaketucker.shaman.main" $MAINCOPY/Info.plist && codesign --force --sign - $MAINCOPY >/dev/null 2>&1; }
compose() {
  local which t n
  maincopy; xcrun simctl install "$UDID" "$MAINCOPY"
  for which in main branch; do
    t=compose-$which-$1; rm -f $D/$t.txt $D/$t.go
    export TEST_RUNNER_FOOT_BUNDLE=com.jaketucker.shaman; [ $which = main ] && export TEST_RUNNER_FOOT_BUNDLE=com.jaketucker.shaman.main
    (uitest $t testComposeKeyboardDown > $D/$t.out 2>&1 &)
    n=0; until grep -q waiting $D/$t.txt 2>/dev/null || [ $n -gt 240 ]; do sleep 0.5; n=$((n+1)); done
    touch $D/$t.go
    n=0; until grep -q ready $D/$t.txt 2>/dev/null || [ $n -gt 120 ]; do sleep 0.5; n=$((n+1)); done
    sleep 1; snap real-$which-$1-compose
    until grep -q "TEST EXECUTE" $D/$t.out 2>/dev/null; do sleep 1; done
    grep -E "keyboard|field" $D/$t.txt | cut -c1-200
  done
  unset TEST_RUNNER_FOOT_BUNDLE
}
