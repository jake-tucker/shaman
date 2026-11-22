# C: a sheet already up, walked through the poses without the app being launched again.
#   zsh c.sh <name>      (a driven test must be running, the simulator closed and upright, the sheet up)
source /tmp/duo719/lib.sh
n=$1
mark() { echo "=== $1" >> /tmp/duo719/app.log; }
step() { mark $1; shift; for p in "$@"; do pose "$p" 4 || echo "POSE FAILED: $p"; done; sleep 2; }
cap c-$n-1-closed
step 2-flat "Open"; cap c-$n-2-flat
step 3-book "Partially Open"; cap c-$n-3-book
step 4-flat "Open"; cap c-$n-4-flat
step 5-laptop "Partially Open" "Rotate Right"; cap c-$n-5-laptop
step 6a-open-portrait "Open"; cap c-$n-6a-open-portrait
step 6-flat "Rotate Right" "Rotate Right" "Rotate Right"; cap c-$n-6-flat
step 7-closed "Closed"; cap c-$n-7-closed
state
grep -E "===|sheet .* safe|questionnaire dete|laptop fold" /tmp/duo719/app.log | sed 's/^[0-9.]* //' > /tmp/duo719/frames/c-$n-walk.txt
