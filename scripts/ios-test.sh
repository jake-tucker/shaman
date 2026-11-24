#!/usr/bin/env bash
# Runs the app's tests on this checkout's own simulators (scripts/ios-simulator.sh), and holds the table of which
# simulator runs which tests. Run scripts/build-ios-core.sh first.
#
#   scripts/ios-test.sh pro|duo|pro-max|all [--ad-hoc] [xcodebuild argument]...
#
#   scripts/ios-test.sh all                                             # every row, one simulator after another
#   scripts/ios-test.sh pro                                             # the Pro's row: everything
#   scripts/ios-test.sh duo -only-testing:ShamanUITests/SessionTests    # these tests alone, on the closed Duo
#
# An `-only-testing:` takes the place of the row's own, so it may name tests the row leaves out, while what the row
# skips is still skipped; a `-skip-testing:` is added to the row. Either reaches only the scheme that holds its target,
# and a scheme left with nothing to run is not run. Any other argument reaches every xcodebuild.
#
# `--ad-hoc` signs as the workflow does, under no team, which needs no Apple account.
#
# The classes of the app's UI tests run in parallel on up to four clones of the simulator, which xcodebuild makes and
# which go as it ends, even killed; the simulators themselves go one after another. A run narrowed to one class has
# nothing to run beside it, so it runs on the simulator itself, sooner, as clones have first to boot; so do the
# package's tests, and the hosted ones alone. A run on clones leaves nothing on the simulator, as the app's container
# goes with them. Pass xcodebuild's own `-parallel-testing-enabled NO` to have any run stay on the simulator, or its
# `-parallel-testing-worker-count` or `-maximum-parallel-testing-workers` for another number of clones.
#
# Everything chosen is run, whatever fails before it, and the script fails if any of it did, or ran no test. Result
# bundles are written to ios/build/results, and what xcodebuild builds beside them.

set -euo pipefail

# The table: what each simulator runs, in xcodebuild's words. `ShamanUIUnitTests` are the package's tests; `ShamanTests`
# and `ShamanUITests` are the app's. A test target added later is added to the Pro's row, which names every one. A test
# that does not hold on some shape is skipped here, in that simulator's row, and never by itself.
simulators=(pro duo pro-max)
row_of() {
  case "$1" in
    pro) row=(-only-testing:ShamanUIUnitTests -only-testing:ShamanTests -only-testing:ShamanUITests) ;;
    duo) row=(-only-testing:ShamanUITests) ;;
    pro-max) row=(-only-testing:ShamanUITests/SmokeTests) ;;
  esac
}

usage() {
  echo "usage: $0 pro|duo|pro-max|all [--ad-hoc] [xcodebuild argument]..." >&2
  exit 2
}

case "${1:-}" in
  pro | duo | pro-max) chosen=("$1") ;;
  all) chosen=("${simulators[@]}") ;;
  *) usage ;;
esac
shift

root=$(cd "$(dirname "$0")/.." && pwd)
build="$root/ios/build"
ad_hoc=false
only=()
rest=()
skips=()
while [ $# -gt 0 ]; do
  argument=$1
  shift
  # xcodebuild takes a selector as one word or two.
  case "$argument" in
    -only-testing | -skip-testing)
      [ -n "${1:-}" ] || usage
      argument="$argument:$1"
      shift
      ;;
  esac
  case "$argument" in
    --ad-hoc) ad_hoc=true ;;
    -only-testing:?*) only+=("$argument") ;;
    -skip-testing:?*) skips+=("$argument") ;;
    *) rest+=("$argument") ;;
  esac
done

# How many clones of a simulator run the UI tests' classes at once, at most.
clones=4

# How many clones the app's selected tests have a use for: one for each class of UI tests they name, up to the most
# there are, and the most where they name the whole target. The hosted tests take a few seconds in one process, and have
# a use for none.
clones_for() {
  local selector classes=()
  for selector in "$@"; do
    case "$selector" in
      -only-testing:ShamanUITests)
        echo "$clones"
        return
        ;;
      -only-testing:ShamanUITests/*) classes+=("$(cut -d/ -f2 <<< "$selector")") ;;
    esac
  done
  # An empty array is unset to the bash macOS ships.
  printf '%s\n' ${classes[@]+"${classes[@]}"} | sort -u | head -n "$clones" | grep -c . || true
}

# Reads the result bundle: what ran and where, and each failure. Fails where no test ran, as xcodebuild does not.
report() {
  local summary
  summary=$(xcrun xcresulttool get test-results summary --path "$1") || return 1
  jq -r '
    "\(.result) on \([.devicesAndConfigurations[].device.deviceName] | join(", "))"
      # A run cut short may have no times.
      + (if .startTime and .finishTime then (.finishTime - .startTime | floor)
        | " in \(. / 60 | floor)m\(. % 60)s" else "" end)
      + ": \(.passedTests) passed, \(.failedTests) failed, \(.skippedTests) skipped",
    (.testFailures[] | "  \(.targetName) \(.testName): \(.failureText)")' <<< "$summary"
  [ "$(jq .totalTestCount <<< "$summary")" -gt 0 ] || {
    echo "No test ran." >&2
    return 1
  }
}

# Runs the scheme `package` or `app` on a simulator, with what is selected of its tests.
run() {
  local scheme=$1 simulator=$2 udid=$3 status=0
  shift 3
  local bundle="$build/results/$simulator-$scheme-$(date +%Y%m%d-%H%M%S)-$$.xcresult"
  local arguments=(-destination "platform=iOS Simulator,id=$udid" -resultBundlePath "$bundle" -quiet)
  local directory="$root/ios" argument parallel=0 enabling=true counting=true
  for argument in "$@"; do
    case "$argument" in
      -parallel-testing-enabled) enabling=false ;;
      -parallel-testing-worker-count | -maximum-parallel-testing-workers) counting=false ;;
    esac
  done
  if $enabling; then
    if [ "$scheme" = app ]; then parallel=$(clones_for "$@"); fi
    if [ "$parallel" -gt 1 ]; then
      arguments+=(-parallel-testing-enabled YES)
      if $counting; then arguments+=(-parallel-testing-worker-count "$parallel"); fi
    else
      arguments+=(-parallel-testing-enabled NO)
    fi
  fi
  if [ "$scheme" = package ]; then
    directory="$root/ios/ShamanUI"
    arguments+=(-scheme ShamanUI -derivedDataPath "$build/package")
    if $ad_hoc; then arguments+=(CODE_SIGNING_ALLOWED=NO); fi
  else
    arguments+=(-project Shaman.xcodeproj -scheme Shaman -derivedDataPath "$build/xcode")
    # Ad hoc and not unsigned: the Keychain answers an unsigned app with errSecMissingEntitlement, and the app keeps its
    # identity key's blob there.
    if $ad_hoc; then arguments+=(CODE_SIGN_IDENTITY=- CODE_SIGN_STYLE=Manual DEVELOPMENT_TEAM=); fi
  fi
  echo "== $simulator, $scheme: $bundle" >&2
  (cd "$directory" && xcodebuild test "${arguments[@]}" "$@") || status=$?
  # A build that fails may leave no bundle to read.
  if [ -d "$bundle" ] || [ "$status" = 0 ]; then report "$bundle" >&2 || status=1; fi
  return "$status"
}

# In one group, which bash reads whole before it runs any, so that an edit to this file does not reach a run going on.
{
  failed=()
  for simulator in "${chosen[@]}"; do
    row_of "$simulator"
    selected=()
    for selector in "${row[@]}"; do
      case "$selector" in
        -only-testing:*) [ "${#only[@]}" -gt 0 ] || selected+=("$selector") ;;
        *) selected+=("$selector") ;;
      esac
    done
    # Written for the bash macOS ships, to which an empty array is unset.
    selected+=(${only[@]+"${only[@]}"} ${skips[@]+"${skips[@]}"})

    udid=
    for scheme in package app; do
      arguments=()
      runs=false
      for selector in "${selected[@]}"; do
        case "$selector" in
          *-testing:ShamanUIUnitTests | *-testing:ShamanUIUnitTests/*) holder=package ;;
          *) holder=app ;;
        esac
        [ "$holder" = "$scheme" ] || continue
        arguments+=("$selector")
        case "$selector" in -only-testing:*) runs=true ;; esac
      done
      $runs || continue
      if [ -z "$udid" ] && ! udid=$("$root/scripts/ios-simulator.sh" "$simulator"); then
        failed+=("$simulator, which did not boot")
        break
      fi
      run "$scheme" "$simulator" "$udid" "${arguments[@]}" ${rest[@]+"${rest[@]}"} || failed+=("$simulator, $scheme")
    done
  done

  if [ "${#failed[@]}" -gt 0 ]; then
    printf 'Failed: %s\n' "${failed[@]}" >&2
    exit 1
  fi
  exit 0
}
