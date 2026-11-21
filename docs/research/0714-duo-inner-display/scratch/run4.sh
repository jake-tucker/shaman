. /tmp/duo714/lib.sh
mark() { snap "$1"; }
round() {
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
pose "Open" 2 || { echo "no Device Hub window"; exit 1; }
state
rm -f /tmp/duo714/app.log /tmp/duo714/notes/*
export TEST_RUNNER_PROTO_SESSION=workstation-key/1
round two-w two
echo "--- log two-w"; sed -E "s/NavigationSplitViewColumn\(tag: SwiftUI.NavigationSplitViewColumn.Tag.//; s/Optional\(SwiftUI.UserInterfaceSizeClass.//" /tmp/duo714/app.log | cut -c1-9,19-160; rm -f /tmp/duo714/app.log
round three-s three
rm -f /tmp/duo714/app.log
pose "Closed" 5; state
uitest closed- two testOpenBack testWalk
echo "--- log closed"; sed -E "s/NavigationSplitViewColumn\(tag: SwiftUI.NavigationSplitViewColumn.Tag.//; s/Optional\(SwiftUI.UserInterfaceSizeClass.//" /tmp/duo714/app.log | cut -c1-9,19-160
for f in /tmp/duo714/notes/*; do echo "== $f"; cut -c1-160 $f; done
(cd /tmp/duo714; tools/sheet sheets-closed-two.png 6 330 shots/closed-two-*.small.png)
pose "Open" 4; state
