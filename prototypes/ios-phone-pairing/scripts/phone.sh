#!/bin/sh
# Drives the spike on a phone from the Mac it is cabled to. The phone must be unlocked.
#   phone.sh install                      puts the device build on the phone
#   phone.sh run <steps> [invite]         starts the app afresh and has it run the steps, comma separated
#   phone.sh log                          copies the app's log off the phone and prints it
#   phone.sh uninstall                    removes the app, which is how the Keychain's survival is tried
set -eu
here="$(cd "$(dirname "$0")/.." && pwd)"
bundle="${BUNDLE_ID:-com.jaketucker.shaman-phone-spike}"
device="${SPIKE_DEVICE:-$(xcrun devicectl list devices 2>/dev/null | awk '/physical/ { for (i = 1; i <= NF; i++) if ($i ~ /^[0-9A-F]{8}-[0-9A-F]{16}$/) print $i }' | head -1)}"
case "${1:-}" in
  install)
    xcrun devicectl device install app --device "$device" "$here"/build/device/Build/Products/Release-iphoneos/Spike.app ;;
  run)
    DEVICECTL_CHILD_SPIKE_RUN="$2" DEVICECTL_CHILD_SPIKE_INVITE="${3:-}" \
      xcrun devicectl device process launch --device "$device" --terminate-existing "$bundle" ;;
  log)
    mkdir -p "$here/build"
    xcrun devicectl device copy from --device "$device" --domain-type appDataContainer \
      --domain-identifier "$bundle" --source Documents/spike.log --destination "$here/build/phone.log" >/dev/null
    cat "$here/build/phone.log" ;;
  uninstall)
    xcrun devicectl device uninstall app --device "$device" "$bundle" ;;
  *) sed -n 2,7p "$0" >&2; exit 2 ;;
esac
