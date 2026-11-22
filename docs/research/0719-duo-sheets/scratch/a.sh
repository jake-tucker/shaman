# A: every sheet on the closed display, as the branch has it and with the vertical bar off in each place.
#   zsh a.sh <tag> <sheets...> -- <launch arguments...>      (a driven test must be running: `drive` first)
source /tmp/duo719/lib.sh
tag=$1; shift; sheets=(); while [ "$1" != "--" ]; do sheets+=($1); shift; done; shift; extra=("$@")
P=${PREFIX:-a}
for s in $sheets; do case $s in
  approval) relaunch --fake paired --open workstation-key/3 $extra; cap $P-approval-$tag-medium; tell grab 0.03; cap $P-approval-$tag-large;;
  questionnaire) relaunch --fake paired --open workstation-key/13 $extra; cap $P-questionnaire-$tag-large;;
  tree) relaunch --fake paired --open workstation-key/12 $extra; sleep 2; tell tap session.tree; cap $P-tree-$tag-medium; tell grab 0.03; cap $P-tree-$tag-large;;
  unreachable) relaunch --fake unreachable $extra; tell swipe 0.4 0.8 0.4 0.2; tell swipe 0.4 0.8 0.4 0.2; tell tap listing.unreachable; cap $P-unreachable-$tag-medium;;
  servers) relaunch --fake relayLogin --login-confirms-after 900 $extra; tell tap home.servers; cap $P-servers-$tag-large; tell label "Log in"; sleep 2; cap $P-login-$tag-large;;
esac; done
