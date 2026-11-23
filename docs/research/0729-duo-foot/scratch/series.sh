#!/bin/zsh
# series.sh <tag> <bundle to bring forward> <seconds>: sends the simulator ($UDID) to Settings, brings the bundle
# forward again and takes screenshots one after another for that many seconds, each named by the tenths of a second
# since the app was asked forward; then says in which of them a home indicator is drawn.
S=/Users/jake/Projects/shaman/.worktrees/prototype-ios-duo-foot/docs/research/0729-duo-foot/scratch
. $S/lib.sh
tag=$1; bundle=$2; secs=${3:-12}
d=""; [ "$UDID" = "$DUO" ] && d="--display=$(active_display)"
xcrun simctl launch "$UDID" com.apple.Preferences >/dev/null; sleep 3
[ "$UDID" = "$DUO" ] && d="--display=$(active_display)"
start=$(python3 -c 'import time; print(time.time())')
xcrun simctl launch "$UDID" "$bundle" >/dev/null
while :; do
  t=$(python3 -c "import time; print(int((time.time()-$start)*10))")
  [ $t -gt $((secs*10)) ] && break
  xcrun simctl io "$UDID" screenshot $d "$W/series-$tag-$(printf %03d $t).png" >/dev/null 2>&1
done
(cd $W && python3 $S/indicator.py 3 series-$tag-*.png)
