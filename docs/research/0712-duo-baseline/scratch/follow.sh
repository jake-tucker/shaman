#!/bin/zsh
. /tmp/duo712/lib.sh
L=/tmp/duo712/probe/duo-probe.log
echo "start: $(state)"
# 1. Closed. A long Session scrolled away from its foot, opened, folded and closed.
export TEST_RUNNER_DUO_HOLD_SESSION=workstation-key/30 TEST_RUNNER_DUO_HOLD_SCROLLED=1
held foldB testHoldLongSession || exit 1
pose "Open" 5; mark foldB-2-open
pose "Partially Open" 5; mark foldB-3-book
pose "Closed" 5; mark foldB-4-closed
release
unset TEST_RUNNER_DUO_HOLD_SESSION TEST_RUNNER_DUO_HOLD_SCROLLED
uitest closed-portrait testListingScrolled testTreeBanner
shot closed-portrait-session-notice 5 --fake releases --open workstation-key/1 --notice-stands 600
shot closed-portrait-session-subagent-child 7 --fake paired --open workstation-key/12.1
shot closed-portrait-session-sidekick-child 7 --fake sidekick --open workstation-key/21
shot closed-portrait-new-session-no-workspace-again 6 --fake paired --compose

# 2. Open, upright.
pose "Open" 6
echo "== open-landscape: $(state)"
uitest open-landscape testListingSearchAndScope testListingScrolled testUnreachableSheet testTreeBanner
export TEST_RUNNER_DUO_HOLD=180
rm -f $L
held open-landscape testSplitView || exit 1
mark splitfold-1-held
pose "Closed" 6; mark splitfold-2-closed
pose "Open" 6; mark splitfold-3-reopened
release
cp $L /tmp/duo712/splitfold-probe.log
rm -f $L
held second testSecondWindowAsked || exit 1
mark secondfold-1-held
pose "Closed" 6; mark secondfold-2-closed
pose "Open" 6; mark secondfold-3-reopened
release
cp $L /tmp/duo712/second-asked-probe.log
unset TEST_RUNNER_DUO_HOLD
rm -f $L
uitest second testSecondWindowFromHomeScreen testSecondWindowIconMenu
cp $L /tmp/duo712/second-home-probe.log

# 3. Open, turned a quarter: the inner display in portrait.
pose "Rotate Right" 6
echo "== open-portrait: $(state)"
uitest open-portrait testListingSearchAndScope testListingScrolled testUnreachableSheet
export TEST_RUNNER_DUO_SPLIT_DX=0.5 TEST_RUNNER_DUO_SPLIT_DY=0.04
uitest open-portrait-top testSplitView
export TEST_RUNNER_DUO_SPLIT_DX=0.97 TEST_RUNNER_DUO_SPLIT_DY=0.5
uitest open-portrait-right testSplitView
unset TEST_RUNNER_DUO_SPLIT_DX TEST_RUNNER_DUO_SPLIT_DY

# 4. Partly folded, still turned: the laptop pose.
pose "Partially Open" 6
echo "== laptop: $(state)"
uitest laptop testListingSearchAndScope testListingScrolled testUnreachableSheet testApprovalLarge

# 5. Partly folded, upright: the book pose.
pose "Rotate Right" 3; pose "Rotate Right" 3; pose "Rotate Right" 6
echo "== book: $(state)"
uitest book testListingSearchAndScope testListingScrolled testUnreachableSheet
pose "Closed" 6
echo "end: $(state)"
