#!/usr/bin/env bash
# Runs the app's tests on this checkout's own simulators (scripts/ios-simulator.sh), and holds the table of which
# simulator runs which tests. Run scripts/build-ios-core.sh first.
#
#   scripts/ios-test.sh pro|duo|pro-max|all [--ad-hoc] [xcodebuild argument]...
#   scripts/ios-test.sh posed open|book|laptop|portrait|all [--ad-hoc] [xcodebuild argument]...
#
#   scripts/ios-test.sh all                                             # every row, one simulator after another
#   scripts/ios-test.sh pro                                             # the Pro's row: all but the columns' tests
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
# A posed run is for a Mac, unlocked from start to finish, and never CI. It builds the app and its tests in a scratch
# directory of its own, then has scripts/ios-pose.sh pose this checkout's Duo, hold the pose while the tests written for
# it run, one after another on the simulator itself, and close it; `all` does so for each pose in turn. The pose is
# named to the tests as SHAMAN_POSE in the test runner's environment. A clone boots closed, so no posed run is made on
# clones.
#
# Everything chosen is run, whatever fails before it, and the script fails if any of it did, or ran no test. Result
# bundles are written to ios/build/results, and what xcodebuild builds beside them.

set -euo pipefail

# The table: what each simulator runs, in xcodebuild's words. `ShamanUIUnitTests` are the package's tests; `ShamanTests`
# and `ShamanUITests` are the app's. A test target added later is added to the Pro's row, which names every one. A test
# that does not hold on some shape is skipped here, in that simulator's row, and never by itself.
#
# Three kinds of the UI tests' classes do not hold on every shape, and a class of any is named here as it is added:
# `columns`, for which columns stand side by side, as they do on a turned Pro Max; `one_column`, for which one stands
# alone on its side, as on a turned Pro; and `turning`, which turn the simulator and hold on any that turns, which a Duo
# does for no test.
columns=(AsideTests AsideWaitsTests NoticeTests NothingOpenTests PlaceBesideTests SidebarTests)
one_column=(OneColumnTests)
turning=(PlaceTests TurningTests)
simulators=(pro duo pro-max)
row_of() {
  case "$1" in
    pro)
      row=(-only-testing:ShamanUIUnitTests -only-testing:ShamanTests -only-testing:ShamanUITests)
      row+=("${columns[@]/#/-skip-testing:ShamanUITests/}")
      ;;
    duo)
      row=(-only-testing:ShamanUITests)
      for kind in "${columns[@]}" "${one_column[@]}" "${turning[@]}"; do
        row+=("-skip-testing:ShamanUITests/$kind")
      done
      ;;
    pro-max)
      row=("${columns[@]/#/-only-testing:ShamanUITests/}" "${turning[@]/#/-only-testing:ShamanUITests/}")
      ;;
  esac
}

# The posed half of the table: what each pose of the Duo runs. Every other run skips those tests, there being no pose.
poses=(open book laptop portrait)
posed_row_of() {
  case "$1" in
    open) row=(-only-testing:ShamanUITests/PosedOpenTests) ;;
    book) row=(-only-testing:ShamanUITests/PosedBookTests) ;;
    laptop) row=(-only-testing:ShamanUITests/PosedLaptopTests) ;;
    portrait) row=(-only-testing:ShamanUITests/PosedPortraitTests) ;;
  esac
}

usage() {
  echo "usage: $0 pro|duo|pro-max|all [--ad-hoc] [xcodebuild argument]..." >&2
  echo "       $0 posed open|book|laptop|portrait|all [--ad-hoc] [xcodebuild argument]..." >&2
  exit 2
}

posed=false
case "${1:-}" in
  pro | duo | pro-max) chosen=("$1") ;;
  all) chosen=("${simulators[@]}") ;;
  posed)
    posed=true
    shift
    case "${1:-}" in
      open | book | laptop | portrait) chosen=("$1") ;;
      all) chosen=("${poses[@]}") ;;
      *) usage ;;
    esac
    ;;
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
  # In a posed run a test that skips is one of another pose.
  local counted='.totalTestCount'
  if $posed; then counted='.passedTests + .failedTests + .expectedFailures'; fi
  [ "$(jq "$counted" <<< "$summary")" -gt 0 ] || {
    echo "No test ran." >&2
    return 1
  }
}

# Ad hoc and not unsigned: the Keychain answers an unsigned app with errSecMissingEntitlement, and the app keeps its
# identity key's blob there.
ad_hoc_signing=(CODE_SIGN_IDENTITY=- CODE_SIGN_STYLE=Manual DEVELOPMENT_TEAM=)

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
    if $ad_hoc; then arguments+=("${ad_hoc_signing[@]}"); fi
  fi
  echo "== $simulator, $scheme: $bundle" >&2
  (cd "$directory" && xcodebuild test "${arguments[@]}" "$@") || status=$?
  # A build that fails may leave no bundle to read.
  if [ -d "$bundle" ] || [ "$status" = 0 ]; then report "$bundle" >&2 || status=1; fi
  return "$status"
}

# Runs a command in ios/, in the background, so that a signal is heeded as it comes and passed on by `stop`. Bash heeds
# one otherwise only once the command ends, and takes a command that ends by its own exit to have dealt with it.
child=
attend() {
  local status=0
  (cd "$root/ios" && exec "$@") &
  child=$!
  wait "$child" || status=$?
  child=
  return "$status"
}

stop() {
  trap '' HUP INT TERM
  if [ -n "$child" ]; then
    kill -TERM "$child" 2> /dev/null || true
    wait "$child" || true
  fi
  exit "$1"
}

# Builds the app and its tests once for the poses chosen, in a directory no other run shares, and runs each pose's
# tests while the Duo holds it. Adds what failed to `failed`.
run_posed() {
  local udid pose argument bundle status arguments
  for argument in "$@"; do
    case "$argument" in
      *parallel-testing* | -derivedDataPath | -resultBundlePath | -destination)
        echo "A posed run stays on the checkout's Duo, and keeps what it makes by run: not $argument" >&2
        exit 2
        ;;
    esac
  done
  mkdir -p "$build/results"
  scratch=$(mktemp -d "$build/posed.XXXXXX")
  # A signal ends what is run, is waited out, and ends the run whatever poses are left. Only then does the scratch
  # directory go. Not a local of this function's, as the trap runs after it.
  trap 'rm -rf "$scratch"' EXIT
  trap 'stop 129' HUP
  trap 'stop 130' INT
  trap 'stop 143' TERM
  if ! udid=$("$root/scripts/ios-simulator.sh" duo); then
    failed+=("posed, the Duo of which did not boot")
    return
  fi
  arguments=(-project Shaman.xcodeproj -scheme Shaman -destination "platform=iOS Simulator,id=$udid"
    -derivedDataPath "$scratch/derived" -quiet)
  if $ad_hoc; then arguments+=("${ad_hoc_signing[@]}"); fi
  # Built before any pose is held, and so the lock on Device Hub.
  if ! attend xcodebuild build-for-testing "${arguments[@]}" "$@"; then
    failed+=("posed, which did not build")
    return
  fi
  for pose in "${chosen[@]}"; do
    posed_row_of "$pose"
    if [ "${#only[@]}" -gt 0 ]; then row=("${only[@]}"); fi
    bundle="$build/results/posed-$pose-$(date +%Y%m%d-%H%M%S)-$$.xcresult"
    echo "== posed $pose: $bundle" >&2
    status=0
    attend env TEST_RUNNER_SHAMAN_POSE="$pose" "$root/scripts/ios-pose.sh" "$pose" -- \
      xcodebuild test-without-building "${arguments[@]}" -resultBundlePath "$bundle" -parallel-testing-enabled NO \
      "${row[@]}" ${skips[@]+"${skips[@]}"} "$@" || status=$?
    if [ -d "$bundle" ] || [ "$status" = 0 ]; then report "$bundle" >&2 || status=1; fi
    [ "$status" = 0 ] || failed+=("posed $pose")
  done
}

# In one group, which bash reads whole before it runs any, so that an edit to this file does not reach a run going on.
{
  failed=()
  if $posed; then
    run_posed ${rest[@]+"${rest[@]}"}
    chosen=()
  fi
  for simulator in ${chosen[@]+"${chosen[@]}"}; do
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
