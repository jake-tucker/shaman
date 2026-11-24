#!/bin/zsh
# walk-max.sh <words> <name>: the turn between landscape and a made-up fold on the Pro Max, one variant a run.
# The driven test must be up: `UDID=$MAX drive --fake paired`.
. /tmp/duo732/lib.sh; UDID=$MAX
words=$1; n=$2
land() { xcrun simctl io $MAX screenshot $D/shots/$1.png >/dev/null 2>&1; sips -Z 900 $D/shots/$1.png --out $D/shots/$1.small.png >/dev/null 2>&1; }
A=(--fake paired --laptop-container $words,fakefold)
tell relaunch $A --open workstation-key/7; sleep 3; mark "$n upright"; snap $n-1-upright
tell turn left; sleep 2; tell label "Show aside"; sleep 1; mark "$n landscape, aside shown"; land $n-2-landscape
tell turn upright; sleep 2; mark "$n upright again"; snap $n-3-below
tell holding "Read src/serving.rs"; sleep 1; snap $n-4-pushed-below
tell turn left; sleep 2; mark "$n landscape, pushed"; land $n-5-landscape-pushed
tell turn upright; sleep 2; mark "$n upright, pushed"; snap $n-6-below-pushed
mark "$n focus"; tell focus; sleep 2; snap $n-7-keyboard; echo "after focus: $(tell focused)"
tell type hi; echo "after typing: $(tell focused)"
cp $D/app.log $D/logs/$n.log
grep -c "field: made" $D/logs/$n.log; grep -n "field:\|holder\|lent\|aside" $D/logs/$n.log | cut -c1-150
