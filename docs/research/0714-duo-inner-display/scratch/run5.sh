. /tmp/duo714/lib.sh
mark() { snap "$1"; }
cd $IOS && xcodebuild build-for-testing -project Shaman.xcodeproj -scheme Shaman -destination "platform=iOS Simulator,id=$UDID" -derivedDataPath build/xcode -quiet 2>&1 | grep -E "error|BUILD" | head
pose "Open" 2 || { echo "no Device Hub window"; exit 1; }
state; rm -f /tmp/duo714/app.log
export TEST_RUNNER_PROTO_SESSION=workstation-key/1 TEST_RUNNER_PROTO_TAP_TURN=1
held pose- three || exit 1
mark fold3-1-open
pose "Partially Open" 6; mark fold3-2-book
pose "Open" 5; mark fold3-3-open-again
pose "Partially Open" 6; pose "Rotate Right" 5; mark fold3-4-laptop
pose "Open" 4; pose "Rotate Right" 3; pose "Rotate Right" 3; pose "Rotate Right" 4; mark fold3-5-open-upright
release; state
grep -E "fold|aside" /tmp/duo714/app.log | cut -c19-200
cd /tmp/duo714; tools/sheet sheets-fold3.png 3 760 shots/fold3-*.small.png
