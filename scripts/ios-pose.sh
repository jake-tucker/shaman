#!/usr/bin/env bash
# Poses this checkout's own iPhone Duo simulator (scripts/ios-simulator.sh duo), at a Mac and never in CI, and prints
# its UDID. Nothing in Xcode 27.2 opens, folds or turns a Duo simulator but the buttons under it in Device Hub, which
# this presses by their tooltips through accessibility (scripts/ios-device-hub.swift), checking after each that it did
# what it says.
#
#   UDID=$(scripts/ios-pose.sh closed|open|book|laptop|portrait)
#   scripts/ios-pose.sh <pose> -- <command> [argument]...
#
#   closed     shut and upright, as a Duo boots: the outer display, 466 x 678 points
#   open       open flat and upright: the inner display in landscape, 951 x 669
#   book       partly open and upright: the inner display in landscape, 951 x 669
#   laptop     partly open and turned right once: the inner display in portrait, 669 x 951
#   portrait   open flat and turned right once: the inner display in portrait, 669 x 951
#
# Each is reached from closed, by "Open", "Partially Open" where it is partly open, and "Rotate Right" where it is
# turned, in that order. The display in use and how the device is held say that the first and the last did what they
# say. Nothing says whether an open Duo is partly open, so a "Partially Open" answered as pressed and lost is not
# noticed here.
#
# With a command, the Duo holds the pose while it runs and is closed after, and the script exits as the command did.
# Without one it stays posed: close it when done, as nothing else does but booting it again.
#
# It needs accessibility permission for whatever runs the shell, and the Mac unlocked from start to finish, as a locked
# one takes Device Hub's windows away. Of the Device Hubs running it uses one that answers, brought to the front, starts
# one only where none does, and quits none. It opens a window on the simulator where none shows it, and closes that as
# it ends.
#
# Device Hub serves every checkout on the Mac, so a lock is held, /tmp/shaman-pose.lock, while the buttons are pressed
# and while a command runs. Whoever comes second waits, saying so, and takes over the lock of a holder that has died. A
# shell may hold it by `mkdir` and let it go by `rmdir`, but then nothing can tell that it died.

set -euo pipefail

usage() {
  echo "usage: $0 closed|open|book|laptop|portrait [-- command [argument]...]" >&2
  exit 2
}

fail() {
  echo "$0: $*" >&2
  exit 1
}

# What is pressed to open it, how it is then held, and the display in use.
aim() {
  pose=$1
  case "$pose" in
    closed) openers=() held=portrait shown=1 ;;
    open) openers=("Open") held=portrait shown=3 ;;
    book) openers=("Open" "Partially Open") held=portrait shown=3 ;;
    laptop) openers=("Open" "Partially Open") held=landscapeRight shown=3 ;;
    portrait) openers=("Open") held=landscapeRight shown=3 ;;
    *) usage ;;
  esac
}

aim "${1:-}"
shift
if [ $# -gt 0 ]; then
  if [ "$1" != -- ] || [ $# -lt 2 ]; then usage; fi
  shift
fi
command=("$@")

root=$(cd "$(dirname "$0")/.." && pwd)
name="Shaman $(basename "$root") duo"
hub_app="$(dirname "$(xcode-select -p)")/Applications/DeviceHub.app"

# --- The lock: a directory, holding a file named for the process that holds it and when that began, as a process's
# number alone is used again.

lock=${SHAMAN_POSE_LOCK:-/tmp/shaman-pose.lock}

began() {
  LC_ALL=C TZ=UTC ps -o lstart= -p "$1" 2> /dev/null | tr -cs 'A-Za-z0-9' '-' || true
}
me="$$.$(began $$)"

take_lock() {
  local said="" holder by
  while true; do
    # Made and named with no signal heeded between, or nothing could tell that its holder died.
    if sh -c 'trap "" HUP INT TERM; mkdir "$1" 2> /dev/null && : > "$1/$2"' - "$lock" "$me"; then return; fi
    # Let go since, where nothing is listed: the directory may be gone.
    holder=$(ls "$lock" 2> /dev/null | head -n 1) || true
    if [ ! -d "$lock" ]; then continue; fi
    # Renamed, which of two that find the holder dead only one can do.
    if [ -n "$holder" ] && [ "$(began "${holder%%.*}")" != "${holder#*.}" ] &&
      mv "$lock/$holder" "$lock/$me" 2> /dev/null; then
      echo "Took over $lock from process ${holder%%.*}, which died holding it" >&2
      return
    fi
    if [ -z "$said" ]; then
      by=${holder:+process ${holder%%.*}}
      echo "Waiting for $lock, held by ${by:-a shell}: one run poses a Duo at a time" >&2
      said=yes
    fi
    sleep 2
  done
}

release_lock() {
  if rm "$lock/$me" 2> /dev/null; then rmdir "$lock" 2> /dev/null || true; fi
}

# --- Device Hub, through accessibility alone: it has no scripting of its own.

# Bounded, should Device Hub keep it waiting.
device_hub() {
  perl -e 'alarm shift; exec @ARGV' 60 "$scratch/device-hub" "$@"
}

locked() {
  ioreg -n Root -d1 -a | grep -A1 IOConsoleLocked | grep -q '<true/>'
}

need_unlocked() {
  if locked; then
    fail "the Mac is locked, and Device Hub shows no window to a locked screen: unlock it, and keep it so to the end"
  fi
}

# Finds the Device Hub to use, as `hub`, and says why where the shell may not press anything or none answers.
reach_device_hub() {
  local tries
  need_unlocked
  xcrun --sdk macosx swiftc -o "$scratch/device-hub" "$root/scripts/ios-device-hub.swift" ||
    fail "scripts/ios-device-hub.swift did not build"
  case "$(device_hub permitted || true)" in
    true) ;;
    false)
      fail "this shell has no accessibility permission, so it can press nothing: give it to the app the shell runs" \
        "in, in System Settings > Privacy & Security > Accessibility"
      ;;
    *) fail "scripts/ios-device-hub.swift built and did not run" ;;
  esac
  for tries in {1..20}; do
    hub=$(device_hub answering "$name" || true)
    if [ -n "$hub" ]; then
      device_hub front "$hub" || true
      return
    fi
    if [ "$tries" = 3 ]; then
      echo "Starting a Device Hub, as none running answers" >&2
      # Without the windows of the last, which pile up.
      open -n -a "$hub_app" --args -ApplePersistenceIgnoreState YES || fail "Device Hub did not open: $hub_app"
    fi
    sleep 2
  done
  need_unlocked
  fail "no Device Hub answers accessibility, though one was started: one that is stuck has to be quit by hand"
}

# --- The simulator, which says what a press did: devicectl, as nothing Device Hub shows can be believed.

display() {
  xcrun devicectl device info displays --device "$udid" -j - 2> /dev/null |
    jq -r '[.result.displays[]? | select(.active) | .displayId][0] // empty' || true
}

orientation() {
  xcrun devicectl device orientation get --device "$udid" -j - 2> /dev/null |
    jq -r '.result.deviceOrientation // empty' || true
}

# Waits for `$1` to print `$2`.
comes() {
  local tries
  for tries in 1 2 3 4 5 6 7 8; do
    if [ "$("$1")" = "$2" ]; then return 0; fi
    sleep 1
  done
  return 1
}

# Presses a button, tried again while Device Hub does not answer or is still connecting the display.
press() {
  local tries answer="" window
  for tries in {1..15}; do
    answer=$(device_hub press "$hub" "$name" "$1" || true)
    case "$answer" in
      pressed) return ;;
      "no window")
        window=$(device_hub open "$hub" "$name" "$udid" || true)
        case "$window" in
          shown) ;;
          com.apple.*) echo "$window" >> "$scratch/opened" ;;
          *) answer="no window ($window)" ;;
        esac
        ;;
    esac
    # A window that could not be opened is not asked for fifteen times.
    if [ "$tries" = 4 ] && [ "${answer%% (*}" = "no window" ]; then break; fi
    if [ $((tries % 5)) = 0 ]; then device_hub front "$hub" || true; fi
    sleep 4
  done
  need_unlocked
  case "$answer" in
    "no window"*)
      fail "Device Hub shows no window of \"$name\" and opened none: choose it in Device Hub's list, on the Space in" \
        "front. A Mac that locked since loses the window. $answer"
      ;;
  esac
  fail "Device Hub did not press \"$1\" for \"$name\": ${answer:-no answer}. Its window says:" \
    "$(device_hub says "$hub" "$name" || true)"
}

# Presses a button until the display it leads to is in use: a press answered as done may have done nothing.
press_until() {
  local tries
  for tries in 1 2 3; do
    press "$1"
    if comes display "$2"; then return; fi
  done
  fail "Device Hub pressed \"$1\" three times, and the display in use is \"$(display)\" and not \"$2\""
}

pose_it() {
  local tries button
  if [ "$(display)" != 1 ]; then press_until "Closed" 1; fi
  # An empty array is unset to the bash macOS ships.
  for button in ${openers[@]+"${openers[@]}"}; do
    press_until "$button" 3
    # The hinge is animated, and "Partially Open" has the display it leads to in use already.
    sleep 4
  done
  # Read before each turn, so that one lost or made twice is seen.
  for tries in {1..8}; do
    if [ "$(orientation)" = "$held" ]; then break; fi
    press "Rotate Right"
    sleep 3
  done
  if ! comes orientation "$held" || ! comes display "$shown"; then
    fail "the Duo is held $(orientation) with display $(display) in use, and not $held with $shown: it is not in the" \
      "pose \"$pose\""
  fi
  # The hinge is animated and the app told of each step.
  sleep 4
}

udid=$("$root/scripts/ios-simulator.sh" duo)
scratch=$(mktemp -d "${TMPDIR:-/tmp}/shaman-pose.XXXXXX")
hub=
posing=
child=

# However it ends, a Duo posed for a command is closed, the windows opened are closed, and the lock is let go. A second
# signal is not heeded, nor by what is run here, as it would leave all of that undone. A signal comes straight here:
# were it to exit, a second, come before this begins, would exit again and this not be run.
end() {
  local status=${1:-$?} window
  trap '' HUP INT TERM
  trap - EXIT
  set +e
  if [ -n "$child" ]; then
    kill -TERM "$child" 2> /dev/null
    wait "$child" 2> /dev/null
  fi
  if [ -n "$posing" ] && [ "${#command[@]}" -gt 0 ]; then
    aim closed
    # In a shell of its own, which is all that ends where it fails: what the command answered counts for more.
    (pose_it) || {
      echo "The Duo was not closed after the command: shutting it down, as it boots closed" >&2
      xcrun simctl shutdown "$udid"
    }
  fi
  if [ -e "$scratch/opened" ]; then
    while read -r window; do device_hub close "$hub" "$window" < /dev/null; done < "$scratch/opened"
  fi
  release_lock
  rm -rf "$scratch"
  exit "$status"
}
trap end EXIT
trap 'end 129' HUP
trap 'end 130' INT
trap 'end 143' TERM

take_lock
# No display is in use for some seconds after a boot.
for _ in {1..30}; do
  if [ -n "$(display)" ]; then break; fi
  sleep 2
done
if [ -z "$(display)" ]; then
  fail "devicectl names no display of \"$name\" ($udid) in use. Where \`xcrun devicectl list devices\` does not list" \
    "it, CoreDevice's service has stopped following the simulators, and has to be started again by hand"
fi
reach_device_hub
posing=yes
pose_it
if [ "${#command[@]}" -eq 0 ]; then
  echo "$udid"
  exit
fi
# Waited for in the background, so that a signal is heeded as it comes, and passed on.
"${command[@]}" &
child=$!
status=0
wait "$child" || status=$?
child=
exit "$status"
