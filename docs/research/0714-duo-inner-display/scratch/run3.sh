. /tmp/duo714/lib.sh
cd $IOS && xcodebuild build-for-testing -project Shaman.xcodeproj -scheme Shaman -destination "platform=iOS Simulator,id=$UDID" -derivedDataPath build/xcode -quiet 2>&1 | grep -E "error|BUILD" | head
UDID=DE942D4E-C4DF-4157-B7BE-5A7EBFCFAFCD; PLAIN=1
rm -f /tmp/duo714/notes/*
uitest pro- stack testWalk testCompose testTree
uitest pro- two testWalk testCompose testTree
cd /tmp/duo714
tools/sheet sheets-pro-stack.png 8 250 shots/pro-stack-*.small.png
tools/sheet sheets-pro-two.png 8 250 shots/pro-two-*.small.png
for f in notes/*.txt; do echo "== $f"; cut -c1-500 "$f"; done
