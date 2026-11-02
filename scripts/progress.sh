#!/usr/bin/env bash
# Shows which cached transaction the running rehearsal is replaying.
CACHE=~/work/hook-rehearsal/cache/txs.json
TOTAL=$(jq length "$CACHE")
FORK=$(ps -eo args | grep "[a]nvil --fork" | grep -o "fork-block-number [0-9]*" | awk '{print $2}')
if [ -z "$FORK" ]; then
  echo "no replay running right now (between transactions, or finished)"
else
  BLOCK=$((FORK + 1))
  POS=$(jq -r '.[].block' "$CACHE" | grep -n "^$BLOCK$" | cut -d: -f1 | paste -sd, -)
  echo "replaying block $BLOCK = transaction $POS of $TOTAL"
fi
ls ~/work/hook-rehearsal/reports/ 2>/dev/null | sed 's/^/report written: /'
