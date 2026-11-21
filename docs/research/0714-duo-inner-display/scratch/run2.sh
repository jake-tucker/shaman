. /tmp/duo714/lib.sh
mark() { snap "$1"; }
cd $IOS && xcodebuild build-for-testing -project Shaman.xcodeproj -scheme Shaman -destination "platform=iOS Simulator,id=$UDID" -derivedDataPath build/xcode -quiet 2>&1 | grep -E "error|BUILD" | head
round() { # round <stem> <variant>
  held pose- $2 || return 1
  mark $1-1-open
  pose "Partially Open" 5; mark $1-2-book
  pose "Rotate Right" 5; mark $1-3-laptop
  pose "Open" 5; mark $1-4-open-portrait
  pose "Closed" 5; mark $1-5-closed-landscape
  pose "Rotate Right" 3; pose "Rotate Right" 3; pose "Rotate Right" 4; mark $1-6-closed
  pose "Open" 5; mark $1-7-open-again
  release; state
  (cd /tmp/duo714; tools/sheet sheets-$1.png 4 560 shots/$1-*.small.png)
}
pose "Open" 1 || { echo "no Device Hub window"; exit 1; }
state
export TEST_RUNNER_PROTO_SESSION=none
round two-none two
export TEST_RUNNER_PROTO_ARGS="--inner-empty nothing"
round two-nothing two
unset TEST_RUNNER_PROTO_ARGS
export TEST_RUNNER_PROTO_SESSION=workstation-key/1
round two-w two
round three-s three
