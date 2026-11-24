#!/bin/zsh
# walk-questionnaire.sh: #734's run, as it was driven by hand. The Duo simulator booted, Device Hub showing it, the Mac
# unlocked, the software keyboard kept up (and put back after):
#   xcrun simctl spawn $DUO defaults write com.apple.Preferences AutomaticMinimizationEnabled -bool false
# It rests on #733's lib.sh and its directory, /tmp/duo733, which the app and the driven test are built to read.
# `--sheet-follows` is the mend tried; without it the sheet is today's.
. /tmp/duo733/lib.sh
F="questionnaire.sheet questionnaire.question questionnaire.choice questionnaire.field questionnaire.next"
pose "Open" 5
SETTLE=6 drive --fake paired --open workstation-key/13 --tree-badge "$@" || exit 1
# The Sidebar beside the main view: each field in turn.
tell tap questionnaire.field; sleep 2; snap k-03-q1-additional; tell frames $F
tell holding Staging
tell type "Use the staging box, but only after the certificates are renewed, and tell me before anything is deployed there, since the others are using it this week."
snap k-05-q1-typed; tell frames $F
tell drag 0.42 0.2; snap k-07-q1-dragged
tell tap questionnaire.next; sleep 2; tell tap questionnaire.field; sleep 2; snap k-09-q2-written; tell frames $F
# A wrap under taps on the keys themselves, by where "m" and the space bar stand in the window.
tell type "Only the unit suite and the integration suite for now, and soak at the weekend"
AFTER=0; for i in {1..14}; do tell at 0.749 0.862; (( i % 4 )) || tell at 0.5 0.94; done; AFTER=1
snap k-22-q2-keys-wrapped; tell frames $F
tell tap questionnaire.skip; sleep 2; tell tap questionnaire.field; sleep 2; snap k-11-q3-secret; tell frames $F
# The Aside beside the main view: the sheet swiped away, the Aside shown, the sheet reviewed.
tell drag 0.07 0.95; sleep 2; tell tap session.aside; sleep 2; tell tap session.review; sleep 2
tell tap questionnaire.field; sleep 2; snap k-16-aside-q3-secret; tell frames $F
# A close and an open with a field holding the keyboard.
pose "Closed" 5; snap k-19-closed; tell frames $F
pose "Open" 5; snap k-21-reopened; tell frames $F
# Upright with nothing open.
tell relaunch --fake paired --tree-badge; sleep 5
pose "Rotate Right" 5; snap k-31-portrait-sidebar-over
tell holding "Search by title"; sleep 2; snap k-32-portrait-search-focused
tell type relay; snap k-33-portrait-search-typed
tell at 0.415 0.047; pose "Rotate Right" 5
tell tap landing.composer; sleep 2; pose "Rotate Right" 6; snap k-36-portrait-turned-with-keyboard; where
tell relaunch --fake paired --tree-badge; sleep 5; tell tap home.compose; sleep 3; snap k-39-portrait-compose-pressed; where
quit
