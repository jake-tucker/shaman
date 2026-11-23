#!/bin/zsh
# foot.sh <tag> <variant>...: with UDID and LOG set and the simulator posed, stands the foot each way in the main
# view's column and says where it stands with the keyboard down, up and down again. Logs kept as foot-<tag>-<variant>.
S=/Users/jake/Projects/shaman/.worktrees/prototype-ios-duo-foot/docs/research/0729-duo-foot/scratch
. $S/lib.sh
tag=$1; shift
for v in "$@"; do
  launch $v --lab-open 1; sleep 4
  cmd end; sleep 1; cmd dump "$tag $v keyboard-down"; snap foot-$tag-$v-down
  cmd focus; sleep 3; cmd end; sleep 1; cmd dump "$tag $v keyboard-up"; snap foot-$tag-$v-up
  cmd blur; sleep 2.5; cmd end; sleep 1; cmd dump "$tag $v keyboard-down-again"
  keep foot-$tag-$v
  echo "$tag $v: $(grep -c 'dump\[' $D/$LOG.log) dump lines"
done
