# Scratch helpers for #712. Source it: `. /tmp/duo712/lib.sh`
UDID=$(cat /tmp/duo712/udid)
NAME="Shaman shaman duo"
APP=/Users/jake/Projects/shaman/ios/build/xcode/Build/Products/Debug-iphonesimulator/Shaman.app
BUNDLE=com.jaketucker.shaman
SHOTS=/tmp/duo712/shots

pose() { # pose "Open" | "Closed" | "Partially Open" | "Rotate Right"
  osascript /tmp/duo712/pose.applescript "$UDID" "$NAME" "$1" && sleep "${2:-4}"
}

active_display() {
  xcrun devicectl device info displays --device "$UDID" -j - 2> /dev/null \
    | jq -r '.result.displays[] | select(.active) | .displayId'
}

orientation() { xcrun devicectl device orientation get --device "$UDID" 2> /dev/null | tail -1; }

probe() { # the newest probe line of each scene
  for f in /tmp/duo712/probe/duo-probe-*.txt; do [ -f "$f" ] && cat "$f" && echo; done
}

probe_reset() {
  rm -f /tmp/duo712/probe/duo-probe-*.txt
}

snap() { # snap <file-stem>: the active display, downscaled copy for reading
  local d
  d=$(active_display)
  xcrun simctl io "$UDID" screenshot --display="$d" "$SHOTS/$1.png" > /dev/null 2>&1
  sips -Z 1100 "$SHOTS/$1.png" --out "$SHOTS/$1.small.png" > /dev/null 2>&1
}

launch() { # launch <args...>
  xcrun simctl launch --terminate-running-process "$UDID" "$BUNDLE" "$@" > /dev/null
}

shot() { # shot <file-stem> <wait> <launch args...>
  local stem=$1 wait=$2
  shift 2
  probe_reset
  launch "$@"
  sleep "$wait"
  snap "$stem"
  probe > "$SHOTS/$stem.probe.txt"
}

setopt nullglob 2> /dev/null || shopt -s nullglob

walk() { # walk <pose-prefix>: every screen a launch reaches without a tap
  local p=$1
  shot "$p-listing-several" 6 --fake several --forget-scene
  shot "$p-listing-unreachable" 6 --fake unreachable --forget-scene
  shot "$p-listing-releases" 5 --fake releases --forget-scene --notice-stands 600
  shot "$p-listing-empty" 5 --fake emptyServer --forget-scene
  shot "$p-session-working" 7 --fake paired --open workstation-key/1
  shot "$p-session-settled" 6 --fake paired --open workstation-key/7
  shot "$p-session-stopped" 6 --fake paired --open workstation-key/8
  shot "$p-session-failed" 6 --fake paired --open workstation-key/9
  shot "$p-session-undelivered" 6 --fake paired --open workstation-key/11
  shot "$p-session-approval" 7 --fake paired --open workstation-key/3
  shot "$p-session-questionnaire" 7 --fake paired --open workstation-key/13
  shot "$p-session-subagents" 7 --fake paired --open workstation-key/12
  shot "$p-session-monitoring" 6 --fake several --open homelab-key/6
  shot "$p-session-loading" 6 --fake reaching --open homelab-key/2
  shot "$p-session-unreachable" 6 --fake unreachable --open homelab-key/2
  shot "$p-session-too-old" 6 --fake releases --open laptop-key/4 --notice-stands 600
  shot "$p-session-subagents-wait" 7 --fake subagentsWait --open workstation-key/12
  shot "$p-session-long-subagent-name" 7 --fake longSubagentName --open workstation-key/12
  shot "$p-session-long" 8 --fake longSession --open workstation-key/30
  shot "$p-new-session" 6 --fake composing --compose
  shot "$p-new-session-no-workspace" 6 --fake paired --compose
  shot "$p-pairing" 5 --fake fresh
  shot "$p-pair-again" 5 --fake pairAgain
  SIMCTL_CHILD_SHAMAN_INVITE=shaman-v1-fake-workstation shot "$p-confirm" 6 --fake fresh
  SIMCTL_CHILD_SHAMAN_INVITE=shaman-v1-fake-relay shot "$p-confirm-relay" 6 --fake fresh
  shot "$p-servers-device" 5 --fake several --servers-and-device
  shot "$p-servers-device-relay" 5 --fake relayed --servers-and-device
}

watch_requests() { # watch_requests <pose-prefix>: answers the UI test's requests until /tmp/duo712/stop-watch exists
  local p=$1 f n
  rm -f /tmp/duo712/stop-watch
  while [ ! -e /tmp/duo712/stop-watch ]; do
    for f in /tmp/duo712/req/*; do
      n=$(basename "$f")
      [ "$n" = hold ] && continue
      snap "$p-$n"
      probe > "$SHOTS/$p-$n.probe.txt"
      rm -f "$f"
    done
    sleep 0.3
  done
}

uitest() { # uitest <pose-prefix> [test names...]: runs DuoWalkTests, or the tests named, answering their requests
  local p=$1
  shift
  local only=()
  if [ $# -eq 0 ]; then only=(-only-testing:ShamanUITests/DuoWalkTests); fi
  for t in "$@"; do only+=(-only-testing:ShamanUITests/DuoWalkTests/$t); done
  rm -f /tmp/duo712/req/*
  watch_requests "$p" &
  local w=$!
  (cd /Users/jake/Projects/shaman/ios && rm -rf build/results/walk-$p.xcresult && xcodebuild test-without-building -project Shaman.xcodeproj -scheme Shaman \
    -destination "platform=iOS Simulator,id=$UDID" -derivedDataPath build/xcode \
    -resultBundlePath build/results/walk-$p.xcresult "${only[@]}" 2>&1 | grep -E "Test Case .*(passed|failed)|error:|\*\* TEST" )
  touch /tmp/duo712/stop-watch
  wait $w 2> /dev/null
  mkdir -p /tmp/duo712/notes/$p && mv /tmp/duo712/notes/*.txt /tmp/duo712/notes/$p/ 2> /dev/null
}

WALK_TESTS=(testListingSearchAndScope testListingScrolled testServersSheetPairingAndScanner testScannerAtTheRoot testUnreachableSheet testWorkAndDetail testTreeSheet testSidekickTreeSheet testRelayLoginSheet testWorkspacePicker testQuestionnaireFields testApprovalLarge testComposerKeyboardAndBanner testLongSessionScrolls)

sheets() { # sheets <pose-prefix>: contact sheets of everything taken in a pose, ten to a sheet
  local p=$1 files i=1
  files=("$SHOTS"/$p-*.small.png)
  while [ ${#files[@]} -gt 0 ]; do
    /tmp/duo712/tools/sheet /tmp/duo712/sheets/$p-s$i.png 5 380 "${files[@]:0:10}"
    files=("${files[@]:10}")
    i=$((i + 1))
  done
}

state() { echo "display=$(active_display) $(orientation)"; }

whole() { # whole <pose-prefix> [extra tests...]: the launch walk and the tap walk in the pose the device is in
  local p=$1
  shift
  echo "== $p: $(state)"
  walk "$p"
  shot "$p-session-notice" 5 --fake releases --open workstation-key/1 --notice-stands 600
  uitest "$p" "${WALK_TESTS[@]}" "$@"
  echo "== $p done: $(state)"
}

held() { # held <prefix> <test> : starts a test that holds, and returns once it is holding
  rm -f /tmp/duo712/req/*
  uitest "$1" "$2" > /tmp/duo712/held.log 2>&1 &
  HELD=$!
  local n=0
  until [ -e /tmp/duo712/req/hold ]; do
    sleep 1
    n=$((n + 1))
    [ $n -gt 120 ] && echo "never held" && return 1
  done
  sleep 1
}

release() { rm -f /tmp/duo712/req/hold; wait $HELD 2> /dev/null; grep -E "passed|failed" /tmp/duo712/held.log; }

mark() { # mark <stem>: a screenshot and the probe, by hand
  snap "$1"
  probe > "$SHOTS/$1.probe.txt"
}
