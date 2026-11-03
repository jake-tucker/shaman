#!/usr/bin/env bash
# Runs the tests with their temporary files on a RAM disk, which on macOS takes the full suite from about 174s to
# 132s. Each test keeps its Server's state, SQLite databases and Git repositories under TMPDIR, and on the system
# volume parallel tests queue on its flushes to disk while Spotlight indexes what they write. A RAM disk makes the
# flushes nearly free and is never indexed. Elsewhere the tests run as `cargo nextest run` runs them.
#
#   scripts/test-on-ramdisk.sh [<cargo nextest run arguments>...]
#   scripts/test-on-ramdisk.sh --detach
#
# The disk is made on first use and reused until --detach or a reboot. It holds as much of the Mac's memory as has
# been written to it, up to its size, until it is detached.
#
# Environment:
#   SURU_TEST_RAMDISK_MB  The disk's size when it is made. Defaults to 2048.
#
# Kept to what macOS's bash 3.2 runs.

set -euo pipefail

VOLUME=/Volumes/suru-test-ram

# Enough descriptors for the Relay tests, which open more than macOS's default 256.
ulimit -n 4096 2> /dev/null || true

if [ "$(uname -s)" != Darwin ]; then
  if [ "${1:-}" = --detach ]; then
    exit 0
  fi
  exec cargo nextest run "$@"
fi

if [ "${1:-}" = --detach ]; then
  if [ -d "$VOLUME" ]; then
    diskutil eject "$VOLUME" > /dev/null
  fi
  exit 0
fi

if [ ! -d "$VOLUME" ]; then
  size_mb=${SURU_TEST_RAMDISK_MB:-2048}
  device=$(hdiutil attach -nomount "ram://$((size_mb * 2048))" 2> /dev/null | awk '{ print $1 }')
  diskutil erasevolume APFS "$(basename "$VOLUME")" "$device" > /dev/null
fi
mkdir -p "$VOLUME/tmp"

TMPDIR="$VOLUME/tmp/" exec cargo nextest run "$@"
