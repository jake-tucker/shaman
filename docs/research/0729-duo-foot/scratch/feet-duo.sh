#!/bin/zsh
# From the open Duo in portrait: stands the foot the ways that work on the inner display in portrait, then turns the
# Duo upright, closes it and stands the foot every way on the outer display.
S=/Users/jake/Projects/shaman/.worktrees/prototype-ios-duo-foot/docs/research/0729-duo-foot/scratch
. $S/lib.sh
export UDID=$DUO LOG=duo
$S/foot.sh open-portrait band a-content a-outer b-negpad c-additional
for n in 1 2 3; do pose "Rotate Right" 3 || echo "POSE FAILED"; done
echo "now $(facing)"
pose "Closed" 4 || echo "POSE FAILED: Closed"
$S/foot.sh closed-portrait band a-content a-inner a-outer b-negpad c-bar c-bar-ignore c-bar-outer c-additional c-safepad follow follow-content follow-b follow-additional
echo "feet duo done"
