# B: where an Intervention's sheet stands on the open display, for each presentationPlacement.
#   zsh b.sh <pose tag> <placements...>     (a driven test must be running, the simulator already posed)
source /tmp/duo719/lib.sh
pose=$1; shift
for place in "$@"; do
  extra=(); [ $place != default ] && extra=(--sheet-place $place)
  relaunch --fake paired --open workstation-key/3 $extra; cap b-$pose-approval-$place-medium; tell grab 0.03; cap b-$pose-approval-$place-large
  relaunch --fake paired --open workstation-key/13 $extra; cap b-$pose-questionnaire-$place-large
done
