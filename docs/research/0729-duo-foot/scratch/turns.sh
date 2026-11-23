#!/bin/zsh
# turns.sh <variant>: from the open Duo in portrait (turned once to the right), turns it round and closes and opens
# it with the foot following the edge read in the main view's column; the log is kept as turns-<variant>.log.
S=/Users/jake/Projects/shaman/.worktrees/prototype-ios-duo-foot/docs/research/0729-duo-foot/scratch
. $S/lib.sh
export UDID=$DUO LOG=duo
v=$1
step() { mark "pressing $1"; pose "$1" 4 || echo "POSE FAILED: $1"; mark "after $1: facing $(facing)"; cmd dump "after $2"; }
launch $v --lab-open 1; sleep 5; cmd dump "start open-portrait"
step "Rotate Right" "turn to open-landscape, upside down"
step "Rotate Right" "turn to open-portrait, the other way"
step "Rotate Right" "turn to open-landscape"
step "Closed" "close to closed-portrait"
step "Open" "open to open-landscape"
step "Rotate Right" "turn to open-portrait"
step "Closed" "close to closed, turned"
step "Open" "open to open-portrait"
keep turns-$v
echo "turns $v done"
