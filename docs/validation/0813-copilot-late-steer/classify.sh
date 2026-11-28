#!/bin/sh
# One line per natural-race trial: when the steer was asked for, what Copilot was sent and how it
# took it, and what the Session read at the end.
for report in "$@"; do
  frames="${report%.json}.frames.jsonl"
  sent=$(jq -r 'select(.method=="session.send" and (.prompt|startswith("STEER"))) | "sent mode=\(.mode // "enqueue") at \(.at_ms)"' "$frames" | head -1)
  idle=$(jq -r 'select(.event=="assistant.idle") | .at_ms' "$frames" | head -1)
  msg=$(jq -r 'select(.event=="assistant.message" and .content=="ALPHA") | .at_ms' "$frames" | head -1)
  taken=$(jq -r 'select(.event=="user.message" and (.content|startswith("STEER"))) | "delivery=\(.delivery)"' "$frames" | head -1)
  end=$(jq -r '.[-1].session | "turns=\(.turns|join(",")) steer=\(.prompts[1].status)/turn\(.prompts[1].taken_in_turn) bravo=\([.messages[]|select(.content=="BRAVO")|"turn\(.turn)"]|join(",") | if .=="" then "NOWHERE" else . end)"' "$report")
  before=$(jq -r '.[0].note.turn_just_before' "$report")
  echo "$(basename "$report" .json): ALPHA@${msg} idle@${idle} | ${sent:-never sent} ${taken} | turn before=${before} | ${end}"
done
