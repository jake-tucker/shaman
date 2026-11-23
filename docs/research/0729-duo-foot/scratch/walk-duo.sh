#!/bin/zsh
# Walks the Duo simulator through every pose, from closed and turned once to the right, pressing Device Hub's buttons.
S=/Users/jake/Projects/shaman/.worktrees/prototype-ios-duo-foot/docs/research/0729-duo-foot/scratch
. $S/lib.sh
export UDID=$DUO LOG=duo
step() { pose "$1" 4 || echo "POSE FAILED: $1"; }
$S/walk.sh closed-turned-once
step "Rotate Right"; echo "now $(facing)"
launch band; sleep 5; cmd dump "closed-upside-down listing"; cmd open; sleep 1.5; cmd dump "closed-upside-down main"; snap closed-upside-down-2main; keep walk-closed-upside-down
step "Rotate Right"; echo "now $(facing)"
$S/walk.sh closed-turned-thrice
step "Rotate Right"; echo "now $(facing)"
$S/walk.sh closed-portrait
step "Open"
$S/walk.sh open-landscape
step "Partially Open"
$S/walk.sh book
step "Rotate Right"; echo "now $(facing)"
$S/walk.sh laptop
step "Open"
$S/walk.sh open-portrait
echo "duo walk done"
