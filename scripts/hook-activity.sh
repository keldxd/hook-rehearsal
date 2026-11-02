#!/usr/bin/env bash
# Counts events and distinct transactions emitted by given hook addresses over the last N blocks.
# Usage: hook-activity.sh <blocks> <hook address> [more hook addresses...]
set -uo pipefail
: "${BASE_RPC_URL:?set BASE_RPC_URL first}"
BLOCKS=$1; shift

rpc() { curl -s -X POST -H 'content-type: application/json' --data "$1" "$BASE_RPC_URL"; }
hex() { printf '0x%x' "$1"; }
LATEST=$(($(rpc '{"jsonrpc":"2.0","id":1,"method":"eth_blockNumber","params":[]}' | jq -r .result)))

for HOOK in "$@"; do
  TXS=$(mktemp)
  for ((TO = LATEST; TO > LATEST - BLOCKS; TO -= 10)); do
    FROM=$((TO - 9))
    rpc "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"eth_getLogs\",\"params\":[{\"address\":\"$HOOK\",\"fromBlock\":\"$(hex $FROM)\",\"toBlock\":\"$(hex $TO)\"}]}" \
      | jq -r '.result[]?.transactionHash' >> "$TXS"
    sleep 0.05
  done
  echo "$HOOK: $(wc -l < "$TXS") events in $(sort -u "$TXS" | wc -l) transactions over the last $BLOCKS blocks"
  rm -f "$TXS"
done
