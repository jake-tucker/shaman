#!/bin/zsh
# walk-duo.sh <words> <name>: the ticket's walk on the Duo simulator, posed through Device Hub at a Mac, one variant a
# run. The driven test must be up on the Duo: `UDID=$DUO drive --fake paired`, the simulator open flat and upright
# (or closed), the software keyboard set to stay up.
. /tmp/duo732/lib.sh; UDID=$DUO
words=$1; n=$2
A=(--fake paired --laptop-container $words)
say() { mark "$n $*"; echo "--- $*"; }
shot() { snap $n-$1; }
: > $D/app.log
pose "Open" || exit 1
tell relaunch $A --open workstation-key/7; sleep 3
say "open flat"; shot 01-open
tell label "Show aside"; sleep 1; say "aside shown beside the main view"; shot 02-open-aside
pose "Partially Open"; say "book"; shot 03-book
pose "Rotate Right"; say "laptop"; shot 04-laptop; echo "below: $(tell has laptop.below)"
tell holding "Read src/serving.rs"; sleep 1; say "an Activity pushed below"; shot 05-laptop-pushed-below
say "focus"; tell focus; sleep 2; shot 06-laptop-keyboard; echo "after focus: $(tell focused)"
tell type hi; sleep 1; echo "after typing: $(tell focused)"; shot 07-laptop-typed
say "keyboard dragged down"; tell drag 0.3 0.95; sleep 2; shot 08-laptop-keyboard-down; echo "after the keyboard: $(tell focused)"
tell label "Show Sidebar"; sleep 1; say "sidebar over"; shot 09-laptop-sidebar
tell holding "Tidy zsh startup time"; sleep 2; say "the same session chosen in the sidebar"; shot 10-laptop-chosen
pose "Open"; say "flat, upright"; shot 11-portrait-flat
pose "Rotate Right"; pose "Rotate Right"; pose "Rotate Right"; say "flat, landscape again"; shot 12-open-again
xcrun devicectl device orientation get --device $DUO 2>&1 | tail -1
cp $D/app.log $D/logs/$n.log
echo "field made: $(grep -c "field: made" $D/logs/$n.log)"; grep -n "field:\|holder\|lent\|pose:\|showing\|hiding\|display mode\|-------" $D/logs/$n.log | cut -c1-150
