#!/bin/zsh
. /tmp/duo712/lib.sh
echo "start: $(state)"
# A: a settled long Session at its foot, through every pose and back.
export TEST_RUNNER_DUO_HOLD_SESSION=workstation-key/30
held foldA testHoldLongSession || exit 1
pose "Open" 5; mark foldA-2-open
pose "Partially Open" 5; mark foldA-3-book
pose "Rotate Right" 5; mark foldA-4-laptop
pose "Open" 5; mark foldA-5-open-portrait
pose "Rotate Right" 3; pose "Rotate Right" 3; pose "Rotate Right" 5; mark foldA-6-open-again
pose "Closed" 5; mark foldA-7-closed
release
# B: the same Session scrolled away from its foot, opened and closed.
export TEST_RUNNER_DUO_HOLD_SCROLLED=1
held foldB testHoldLongSession || exit 1
pose "Open" 5; mark foldB-2-open
pose "Partially Open" 5; mark foldB-3-book
pose "Closed" 5; mark foldB-4-closed
pose "Open" 5; mark foldB-5-open
pose "Closed" 5; mark foldB-6-closed
release
unset TEST_RUNNER_DUO_HOLD_SCROLLED
# C: a streaming Session at its foot, opened and closed.
export TEST_RUNNER_DUO_HOLD_SESSION=workstation-key/31
held foldC testHoldLongSession || exit 1
mark foldC-1b-closed-later
pose "Open" 5; mark foldC-2-open
sleep 4; mark foldC-2b-open-later
pose "Closed" 5; mark foldC-3-closed
sleep 4; mark foldC-3b-closed-later
release
echo "end: $(state)"
cp /tmp/duo712/probe/duo-probe.log /tmp/duo712/fold-probe.log
