#!/bin/zsh
# walk.sh <tag>: with UDID and LOG set and the simulator already posed, reads everything items 1, 2, 3, 6 and 7 ask
# for in this pose, and keeps the log as logs/walk-<tag>.log.
. /Users/jake/Projects/shaman/.worktrees/prototype-ios-duo-foot/docs/research/0729-duo-foot/scratch/lib.sh
tag=$1
launch band
burst $tag-launch 4
sleep 4
mark "pose $tag, facing $(facing)"
cmd dump "$tag listing"; snap $tag-1listing
cmd open; sleep 1.5; cmd dump "$tag main"; snap $tag-2main
cmd aside; sleep 2.5; cmd dump "$tag aside"; snap $tag-3aside; cmd aside; sleep 2
cmd sheet medium; sleep 2.5; cmd dump "$tag sheet-medium"; snap $tag-4sheet-medium; cmd sheet off; sleep 2
cmd sheet large; sleep 2.5; cmd dump "$tag sheet-large"; snap $tag-5sheet-large; cmd sheet off; sleep 2
cmd bar off; sleep 2; cmd dump "$tag bar-off main"; snap $tag-6baroff-main
cmd sheet medium; sleep 2.5; cmd dump "$tag bar-off sheet-medium"; snap $tag-7baroff-sheet-medium; cmd sheet off; sleep 2
cmd sheet large; sleep 2.5; cmd dump "$tag bar-off sheet-large"; snap $tag-8baroff-sheet-large; cmd sheet off; sleep 2
cmd bar on; sleep 1.5; cmd dump "$tag bar-on-again main"
mark "away to Settings"; away; sleep 3
mark "front again"; front; burst $tag-return 6; sleep 3
cmd dump "$tag returned"
mark "away to Messages"; xcrun simctl launch "$UDID" com.apple.MobileSMS >/dev/null; sleep 4; snap $tag-messages
mark "front again from Messages"; front; burst $tag-return2 4; sleep 2
cmd dump "$tag returned from Messages"
keep walk-$tag
echo "walked $tag: $(grep -c 'dump\[' $D/$LOG.log) dump lines"
