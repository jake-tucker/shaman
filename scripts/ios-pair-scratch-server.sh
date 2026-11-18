#!/usr/bin/env bash
# Pairs the app in this checkout's simulator with a scratch Server: a Shaman of this checkout's own build, on a channel
# and in directories of its own under a scratch root, Serving on the Mac's own addresses, which the simulator dials as
# a phone on the same network would (an Invite offers no loopback address). It starts that Server afresh, its
# database emptied first, has it make an Invite, and launches the debug app with the Invite in its launch environment and
# a core directory and identity key of its own, so the app opens on the confirmation screen and nothing of another run
# is touched. Run it before merging a change to pairing or transport; never in CI.
#
#   scripts/ios-pair-scratch-server.sh            # launch the app with the Invite; press Pair in it
#   scripts/ios-pair-scratch-server.sh --test     # run the UI test that presses Pair, checks where it lands,
#                                                  # opens a Session begun there and reads it live, and stops the
#                                                  # Server mid-run to see it read Unreachable and answer again
#   scripts/ios-pair-scratch-server.sh --stop     # stop the scratch Server
#   scripts/ios-pair-scratch-server.sh --resume   # start the stopped scratch Server again, its records kept, so the
#                                                  # app that was paired sees the Remote answer again
#
# Build the app first (see ios/AGENTS.md); the script builds Shaman itself. `SHAMAN_SCRATCH_ROOT` moves the scratch
# root and `SHAMAN_SCRATCH_PORT` the port the Server listens on.

set -euo pipefail

mode=${1:-launch}
root=$(cd "$(dirname "$0")/.." && pwd)
checkout=$(basename "$root")
scratch=${SHAMAN_SCRATCH_ROOT:-${TMPDIR:-/tmp}/shaman-scratch-$checkout}
port=${SHAMAN_SCRATCH_PORT:-7791}
shaman="$root/target/debug/shaman"

scratch_shaman() {
  SHAMAN_STATE_DIR="$scratch/state" SHAMAN_DATA_DIR="$scratch/data" SHAMAN_CONFIG_DIR="$scratch/config" \
    SHAMAN_CHANNEL="scratch-$checkout" SHAMAN_IDENTITY_STORE=file "$shaman" "$@"
}

case "$mode" in
  launch | --test) ;;
  --stop)
    scratch_shaman server stop
    exit 0
    ;;
  --resume)
    scratch_shaman server start
    exit 0
    ;;
  *)
    echo "usage: $0 [--test|--stop|--resume]" >&2
    exit 2
    ;;
esac

echo "Building Shaman" >&2
cargo build --locked --bin shaman --manifest-path "$root/Cargo.toml"

# The listener's address is read as the Server starts, so one left from an earlier run is stopped first. Each run
# starts from an empty database, so what one left behind — its Sessions, its Peers — is never what the next finds.
scratch_shaman server stop > /dev/null 2>&1 || true
rm -rf "$scratch/state" "$scratch/data"
mkdir -p "$scratch/state" "$scratch/data" "$scratch/config"
cat > "$scratch/config/shaman.jsonc" <<EOF
{
  "serving": { "enabled": true, "port": $port, "bindAddress": "0.0.0.0" }
}
EOF

# A test stops the Server it started however it ends, its xcodebuild failing among the ways.
if [ "$mode" = --test ]; then
  trap 'scratch_shaman server stop >&2 || true' EXIT
fi

echo "Making an invite on the scratch Server in $scratch" >&2
invite=$(scratch_shaman serve invite)
echo "Invite: $invite" >&2

udid=$("$root/scripts/ios-simulator.sh")
app="$root/ios/build/xcode/Build/Products/Debug-iphonesimulator/Shaman.app"
run="scratch-$(date +%s)"

if [ "$mode" = --test ]; then
  # One Session for the test to open over the Pairing, begun as the Server's own user would, over its loopback API, in
  # an empty directory of the scratch root's. It runs on whichever Agent the Server would give it, asked for one word.
  prompt="Reply with the single word ready, and nothing else. Use no tools."
  workspace="$scratch/workspace"
  mkdir -p "$workspace"
  runtime="$scratch/state/scratch-$checkout/runtime.json"
  jq -n --arg path "$workspace" --arg text "$prompt" --arg id "$(uuidgen | tr '[:upper:]' '[:lower:]')" \
    '{execution_directory: {path: $path}, prompt: {id: $id, text: $text, skill_invocations: []}}' |
    curl --fail --silent --show-error -o /dev/null -H "Authorization: Bearer $(jq -r .token "$runtime")" \
      -H 'Content-Type: application/json' --data @- "$(jq -r .base_url "$runtime")/v1/sessions"
  echo "Began a session on the scratch Server: $prompt" >&2

  # The test asks for the Server to be stopped and started again by files here, to see it read Unreachable and
  # answer again, and is told each is done the same way.
  signals="$scratch/signals"
  rm -rf "$signals"
  mkdir -p "$signals"
  (
    until [ -e "$signals/stop" ]; do sleep 0.2; done
    scratch_shaman server stop >&2 && touch "$signals/stopped"
    until [ -e "$signals/resume" ]; do sleep 0.2; done
    scratch_shaman server start >&2 && touch "$signals/resumed"
  ) &
  serving=$!
  trap 'kill "$serving" 2> /dev/null || true; scratch_shaman server stop >&2 || true' EXIT

  cd "$root/ios"
  result="build/results/live-pairing-$(date +%s).xcresult"
  TEST_RUNNER_SHAMAN_LIVE_INVITE="$invite" TEST_RUNNER_SHAMAN_LIVE_RUN="$run" \
    TEST_RUNNER_SHAMAN_LIVE_PROMPT="$prompt" TEST_RUNNER_SHAMAN_LIVE_SIGNALS="$signals" xcodebuild test \
    -project Shaman.xcodeproj -scheme Shaman -destination "platform=iOS Simulator,id=$udid" \
    -derivedDataPath build/xcode -resultBundlePath "$result" -quiet \
    -only-testing:ShamanUITests/LivePairingTests
  summary=$(xcrun xcresulttool get test-results summary --path "$result")
  echo "$summary"
  [ "$(jq -r .result <<< "$summary")" = Passed ]
  exit
fi

if [ ! -d "$app" ]; then
  echo "Build the app first: no $app" >&2
  exit 1
fi
xcrun simctl install "$udid" "$app"
SIMCTL_CHILD_SHAMAN_INVITE="$invite" SIMCTL_CHILD_SHAMAN_CORE_DIRECTORY="$run" SIMCTL_CHILD_SHAMAN_KEY_ITEM="$run" \
  xcrun simctl launch --terminate-running-process "$udid" com.jaketucker.shaman
echo "Launched the app on $udid with the invite, its records in \"$run\". Press Pair in it." >&2
echo "Stop the scratch Server with: $0 --stop" >&2
