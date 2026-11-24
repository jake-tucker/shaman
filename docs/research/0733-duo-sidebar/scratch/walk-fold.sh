#!/bin/zsh
# walk-fold.sh: closed and opened with nothing open, on its side and upright, and upright with a Session open. The Duo
# simulator open flat and upright, Device Hub showing it, no driven test up.
. /tmp/duo733/lib.sh
W() { echo "$1: $(where | sed 's/ collapsed=false//; s/ prefersBehavior=1//')"; mark "$1"; snap $2; }
P() { pose "$1" >/dev/null || { echo "Device Hub did not answer for $1"; quit; exit 1; }; }
P "Open"; echo "pose: $(xcrun devicectl device orientation get --device $DUO 2>&1 | tail -1)"
SETTLE=5 drive --fake paired --sidebar who --tree-badge || exit 1
W "launched flat, nothing open" f-01-flat
P "Closed"; W "closed" f-02-closed
P "Open"; W "opened" f-03-opened
P "Partially Open"; W "book" f-04-book
P "Closed"; W "closed from the book pose" f-05-closed
P "Partially Open"; W "opened to the book pose" f-06-book
P "Open"; P "Rotate Right"; W "portrait" f-07-portrait
P "Closed"; W "closed, on its side" f-08-closed-side
P "Open"; W "opened in portrait" f-09-portrait-opened
tell holding "Tidy zsh startup"; sleep 2; W "a session chosen" f-10-chosen
P "Closed"; W "closed with a session open" f-11-closed-open
P "Open"; W "opened in portrait with a session open" f-12-portrait-session
P "Rotate Right"; P "Rotate Right"; P "Rotate Right"; W "landscape" f-13-landscape
echo "pose: $(xcrun devicectl device orientation get --device $DUO 2>&1 | tail -1)"
grep -E "rest: showing|didExpand|didCollapse|the reader returned" $D/app.log | cut -c1-110
cp $D/app.log $D/logs/f-fold.log
quit
