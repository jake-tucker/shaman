#!/bin/zsh
# walk-duo-b.sh <words> <name>: what #726 and #718 left for the pose, on the Duo simulator posed through Device Hub.
# The driven test must be up on the Duo, the simulator open flat and upright.
. /tmp/duo732/lib.sh; UDID=$DUO
words=$1; n=$2
A=(--fake paired --laptop-container $words)
say() { mark "$n $*"; echo "--- $*"; }
shot() { snap $n-$1; }
: > $D/app.log
pose "Open" || exit 1
tell relaunch $A --open workstation-key/12; sleep 4
say "open flat, the aside not shown"; shot 21-open-hidden
pose "Partially Open"; pose "Rotate Right"; say "laptop, a tree below"; shot 22-laptop-tree; echo "aside button: $(tell has session.aside)"
tell label "Work"; sleep 1; say "work chosen below"; shot 23-laptop-work-chosen
pose "Open"; say "flat, upright"; shot 24-portrait-flat
tell tap session.turn; sleep 2; say "a turn's work pushed in the main view"; shot 25-portrait-work-pushed
pose "Partially Open"; say "laptop, work pushed"; shot 26-laptop-work-pushed; echo "below: $(tell has laptop.below)"
tell back; sleep 2; say "back"; shot 27-laptop-back
pose "Rotate Right"; pose "Rotate Right"; pose "Rotate Right"; say "book, the aside still not shown"; shot 28-book-hidden
pose "Open"; say "open flat"; shot 29-open-hidden
pose "Partially Open"; pose "Rotate Right"
tell relaunch $A --open workstation-key/7; sleep 4; say "launched in the pose"; shot 30-launched-laptop
tell relaunch $A; sleep 4; say "launched in the pose, nothing open"; shot 31-launched-nothing
tell relaunch $A --open workstation-key/3; sleep 5; say "an approval"; shot 32-approval
tell relaunch $A --open workstation-key/13; sleep 5; say "a questionnaire"; shot 33-questionnaire
pose "Rotate Right"; pose "Rotate Right"; pose "Rotate Right"; pose "Open"
xcrun devicectl device orientation get --device $DUO 2>&1 | tail -1
cp $D/app.log $D/logs/$n.log
grep -n "field:\|holder\|lent\|pose: fold\|showing\|hiding\|display mode\|sync\|read back\|returned\|-------" $D/logs/$n.log | cut -c1-150
