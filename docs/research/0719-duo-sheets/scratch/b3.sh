# B, the third column up: the sheet swiped away, a Turn's work shown in the third column, and "Review" presenting the
# sheet again over it.   zsh b3.sh <pose tag> <placements...>
source /tmp/duo719/lib.sh
pose=$1; shift
for place in "$@"; do
  extra=(); [ $place != default ] && extra=(--sheet-place $place)
  for s in approval:3 questionnaire:13; do
    relaunch --fake paired --open workstation-key/${s#*:} $extra; tell grab 0.99; sleep 1; tell tap session.turn; sleep 2
    tell label Review; cap b-$pose-${s%:*}-$place
  done
done
