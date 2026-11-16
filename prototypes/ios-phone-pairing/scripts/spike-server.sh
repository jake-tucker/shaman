#!/bin/sh
# Runs `shaman` against a Server of its own in a scratch directory, on its own channel and port, so trying the
# spike leaves the machine's real Server alone. Usage: spike-server.sh serve invite --json
set -eu
root="${SPIKE_SERVER_ROOT:-/tmp/shaman-spike-server}"
shaman="${SHAMAN_BIN:-$(git rev-parse --path-format=absolute --git-common-dir)/../target/release/shaman}"
mkdir -p "$root/state" "$root/data" "$root/config"
[ -f "$root/config/shaman.jsonc" ] || printf '{\n  "serving": { "enabled": true, "port": 7791 }\n}\n' > "$root/config/shaman.jsonc"
SHAMAN_STATE_DIR="$root/state" SHAMAN_DATA_DIR="$root/data" SHAMAN_CONFIG_DIR="$root/config" \
  SHAMAN_CHANNEL=spike SHAMAN_IDENTITY_STORE=file exec "$shaman" "$@"
