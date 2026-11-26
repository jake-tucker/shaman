#!/bin/sh
# Throwaway (#774): the headless measurements, each from a pristine scratch Channel.
#   measure-all.sh <scratch-dir> <rune-transcript executable> <largest-session-id>
set -eu
scratch=$1; app=$2; largest=$3
run() {
  pkill -f rune-transcript-server 2>/dev/null || true
  while pgrep -f rune-transcript-server >/dev/null; do sleep 0.2; done
  cp "$scratch/pristine.db" "$scratch/state/rune-transcript/shaman.db"
  env RT_STATE="$scratch/state" RT_MEASURE=1 "$@" "$app"
}
run
run RT_SESSIONS="$largest" RT_MEMO=0
run RT_SESSIONS="$largest" RT_ACTIVITY_LINES=0
run RT_SESSIONS="$largest" RT_REPEAT=10
run RT_SESSIONS="$largest" RT_REPEAT=10 RT_MEMO=0
run RT_SESSIONS="$largest" RT_REPEAT=10 RT_VERSIONED_KEYS=0
run RT_SESSIONS="$largest" RT_REPEAT=100
run RT_SESSIONS="$largest" RT_REPEAT=100 RT_VERSIONED_KEYS=0
pkill -f rune-transcript-server 2>/dev/null || true
