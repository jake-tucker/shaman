#!/bin/sh
# Throwaway (#774): the real window driving itself, once for each way of crossing to the UI thread.
#   window-all.sh <scratch-dir> <rune-transcript executable> <session-id>
set -eu
scratch=$1; app=$2; session=$3
for bridge in ${RT_BRIDGES:-batch update poll}; do
  pkill -f rune-transcript-server 2>/dev/null || true
  while pgrep -f rune-transcript-server >/dev/null; do sleep 0.2; done
  cp "$scratch/pristine.db" "$scratch/state/rune-transcript/shaman.db"
  env RT_STATE="$scratch/state" RT_AUTO=1 RT_BRIDGE=$bridge RT_SESSION="$session" "$app"
done
pkill -f rune-transcript-server 2>/dev/null || true
