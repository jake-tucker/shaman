#!/bin/zsh
# Walks the iPhone 18 Pro upright and turned both ways, and the iPhone 18 Pro Max upright and turned.
S=/Users/jake/Projects/shaman/.worktrees/prototype-ios-duo-foot/docs/research/0729-duo-foot/scratch
. $S/lib.sh
export UDID=$PRO LOG=pro
turn portrait; $S/walk.sh pro-portrait
turn landscapeLeft; echo "pro $(facing)"; $S/walk.sh pro-landscapeLeft
turn landscapeRight; echo "pro $(facing)"; $S/walk.sh pro-landscapeRight
turn portrait
export UDID=$MAX LOG=max
turn portrait; $S/walk.sh max-portrait
turn landscapeLeft; echo "max $(facing)"; $S/walk.sh max-landscapeLeft
turn landscapeRight; echo "max $(facing)"; $S/walk.sh max-landscapeRight
turn portrait
echo "phones walk done"
