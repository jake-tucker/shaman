shopt -s nullglob 2>/dev/null || setopt nullglob
D=/tmp/duo725
IOS=/Users/jake/Projects/shaman/.worktrees/prototype-ios-duo-columns/ios
APP=$IOS/build/xcode/Build/Products/Debug-iphonesimulator/Shaman.app
BUNDLE=com.jaketucker.shaman
PRO=2A2FEAA8-23F3-473F-AFD8-581C8925A361
DUO=C2F3038F-36AD-45F7-B847-F37B000FBE21
DUONAME="Shaman shaman duo"
UDID=${UDID:-$PRO}
build() { (cd $IOS && xcodebuild ${1:-build} -project Shaman.xcodeproj -scheme Shaman -destination "platform=iOS Simulator,id=$PRO" -derivedDataPath build/xcode 2>&1 | grep -E "error:|BUILD|TEST BUILD" | sort -u | cut -c1-400 | head -30); }
install() { xcrun simctl install "$UDID" "$APP"; }
launch() { : > $D/app.log; xcrun simctl launch --terminate-running-process "$UDID" "$BUNDLE" --fake paired "$@" >/dev/null; }
pose() { local n; for n in 1 2 3 4 5; do osascript $D/pose.applescript "$DUO" "$DUONAME" "$1" 2>$D/pose.err >/dev/null && { sleep "${2:-4}"; return 0; }; sleep 2; done; cat $D/pose.err; return 1; }
active_display() { xcrun devicectl device info displays --device "$UDID" -j - 2>/dev/null | jq -r '.result.displays[] | select(.active) | .displayId'; }
snap() { local d; if [ "$UDID" = "$DUO" ]; then d=$(active_display); xcrun simctl io "$UDID" screenshot --display="$d" "$D/shots/$1.png" >/dev/null 2>&1; else xcrun simctl io "$UDID" screenshot "$D/shots/$1.png" >/dev/null 2>&1; fi; sips -Z ${2:-900} "$D/shots/$1.png" --out "$D/shots/$1.small.png" >/dev/null 2>&1; }
mark() { echo "------- $*" >> $D/app.log; }
uitest() { # uitest <name> <variant args...> -- tests...
  local name=$1; shift; local args=(); while [ $# -gt 0 ] && [ "$1" != "--" ]; do args+=("$1"); shift; done; shift
  local only=(); for t in "$@"; do only+=(-only-testing:ShamanUITests/PrototypeColumnsTests/$t); done
  : > $D/app.log
  (cd $IOS && rm -rf build/results/lab-$name.xcresult && TEST_RUNNER_LAB_ARGS="${args[*]}" xcodebuild test-without-building -project Shaman.xcodeproj -scheme Shaman -destination "platform=iOS Simulator,id=$UDID" -derivedDataPath build/xcode -resultBundlePath build/results/lab-$name.xcresult "${only[@]}" 2>&1 | grep -E "Test Case .*(passed|failed)|error:|\*\* TEST" | cut -c1-600)
}
MAX=EED9F182-8C16-40DA-B001-29865132DB85
