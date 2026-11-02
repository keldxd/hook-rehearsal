#!/usr/bin/env bash
# Checks what the RPC in $BASE_RPC_URL supports. Never prints the URL (it contains the key).
set -uo pipefail
: "${BASE_RPC_URL:?set BASE_RPC_URL first}"

PM=0x498581ff718922c3f8e6a244956af099b2652b2b   # Uniswap v4 PoolManager on Base
SWAP_TOPIC=$(cast keccak "Swap(bytes32,address,int128,int128,uint160,uint128,int24,uint24)")

rpc() {
  curl -s -X POST -H 'content-type: application/json' --data "$1" "$BASE_RPC_URL"
}
hex() { printf '0x%x' "$1"; }

echo "== 1. latest block"
LATEST=$(rpc '{"jsonrpc":"2.0","id":1,"method":"eth_blockNumber","params":[]}' | jq -r .result)
LATEST=$((LATEST))
echo "latest: $LATEST"

echo "== 2. archive access (PoolManager code about 115 days ago)"
OLD=$((LATEST - 5000000))
CODE=$(rpc "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"eth_getCode\",\"params\":[\"$PM\",\"$(hex $OLD)\"]}")
echo "$CODE" | jq -r 'if .error then "ERROR: \(.error.message)" else "code length at block '"$OLD"': \((.result | length) / 2 - 1) bytes" end'

echo "== 3. eth_getLogs block range limits (PoolManager Swap events)"
FIRST_TX=""
for R in 9; do
  FROM=$((LATEST - R))
  RES=$(rpc "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"eth_getLogs\",\"params\":[{\"address\":\"$PM\",\"topics\":[\"$SWAP_TOPIC\"],\"fromBlock\":\"$(hex $FROM)\",\"toBlock\":\"$(hex $LATEST)\"}]}")
  if echo "$RES" | jq -e .error >/dev/null; then
    echo "range $R blocks: ERROR: $(echo "$RES" | jq -r .error.message | cut -c1-150)"
  else
    N=$(echo "$RES" | jq '.result | length')
    echo "range $R blocks: OK, $N swaps"
    [ -z "$FIRST_TX" ] && FIRST_TX=$(echo "$RES" | jq -r '.result[0].transactionHash // empty')
  fi
done

echo "== 4. debug_traceTransaction with prestateTracer"
if [ -z "$FIRST_TX" ]; then
  echo "no swap transaction found to test with"
else
  echo "tx: $FIRST_TX"
  RES=$(rpc "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"debug_traceTransaction\",\"params\":[\"$FIRST_TX\",{\"tracer\":\"prestateTracer\"}]}")
  echo "$RES" | jq -r 'if .error then "ERROR: \(.error.message)" else "OK: pre-state has \(.result | length) accounts, \([.result[] | (.storage // {}) | length] | add) storage slots" end'
fi

echo "== 5. alternative: trace_replayTransaction"
if [ -n "$FIRST_TX" ]; then
  RES=$(rpc "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"trace_replayTransaction\",\"params\":[\"$FIRST_TX\",[\"stateDiff\"]]}")
  echo "$RES" | jq -r 'if .error then "ERROR: \(.error.message)" else "OK" end'
fi
