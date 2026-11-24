#!/bin/zsh
# walk-choice.sh: with nothing open, the Sidebar put away by its own button, turned to portrait and back, then shown
# and turned again. The Duo simulator open flat and upright, Device Hub showing it, no driven test up.
. /tmp/duo733/lib.sh
W() { echo "$1: $(where | sed 's/ collapsed=false//; s/ prefersBehavior=1//')"; mark "$1"; snap $2; }
P() { pose "$1" >/dev/null || { echo "Device Hub did not answer for $1"; quit; exit 1; }; }
P "Open"; echo "pose: $(xcrun devicectl device orientation get --device $DUO 2>&1 | tail -1)"
SETTLE=5 drive --fake paired --sidebar who --tree-badge || exit 1
W "launched flat, nothing open" c-01-flat
tell label "Hide Sidebar"; sleep 2; W "put away by its button" c-02-hidden
P "Rotate Right"; W "turned to portrait, put away" c-03-portrait-hidden
P "Rotate Right"; W "landscape, put away" c-04-landscape-hidden
tell label "Show Sidebar"; sleep 2; W "shown by its button" c-05-shown
P "Rotate Right"; W "turned to portrait, shown" c-06-portrait-over
P "Rotate Right"; W "landscape" c-07-landscape
grep -E "rest: showing|transition ended" $D/app.log | cut -c1-95
cp $D/app.log $D/logs/c-readers-choice.log
quit
