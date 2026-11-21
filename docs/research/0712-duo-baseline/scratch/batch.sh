#!/bin/zsh
. /tmp/duo712/lib.sh
pose "Open" 6
whole open-landscape testSplitView
sheets open-landscape
pose "Partially Open" 6
whole book testSplitView
sheets book
pose "Rotate Right" 6
whole laptop
sheets laptop
pose "Open" 6
whole open-portrait testSplitView
sheets open-portrait
pose "Closed" 6
whole closed-landscape
sheets closed-landscape
pose "Rotate Right" 3; pose "Rotate Right" 3; pose "Rotate Right" 4
echo "== end: $(state)"
