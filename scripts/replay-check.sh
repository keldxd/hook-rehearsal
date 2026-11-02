#!/usr/bin/env bash
# Replays the last K transactions that touched a hook on a local anvil fork and compares
# each one with its real mainnet receipt. Usage: replay-check.sh <hook address> [K]
# MODE=related (default): fork at the start of the block, replay the earlier transactions in
#   that block that touched the same pools, contracts or accounts as the target, then the
#   target, all in one block in their real order.
# MODE=fast: fork at the start of the block and replay only the target.
# Transactions keep their real fees, and the block gets its real base fee and timestamp.
set -uo pipefail
: "${BASE_RPC_URL:?set BASE_RPC_URL first}"
HOOK=$1
K=${2:-5}
MODE=${MODE:-related}
PORT=8546
LOCAL=http://127.0.0.1:$PORT
PM=0x498581ff718922c3f8e6a244956af099b2652b2b
TRANSFER=0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef

# call <url> <method> <params as JSON array>  -> prints .result (or the error)
call() {
  curl -s -X POST -H 'content-type: application/json' \
    --data "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"$2\",\"params\":$3}" "$1" \
    | jq -c 'if .error then {error: .error.message} else .result end'
}
hex() { printf '0x%x' "$1"; }

# What a log "touches": the v4 pool id for PoolManager logs, the two accounts for token
# transfers (not the token itself, everyone uses WETH), otherwise the emitting contract.
TOUCH='def addr(t): "0x" + (t | ascii_downcase | .[26:66]);
def touch: (.address | ascii_downcase) as $e
  | if $e == $pm then ["pool:" + ((.topics[1] // "") | ascii_downcase)]
    elif ((.topics[0] // "") | ascii_downcase) == $tr then
      [.topics[1:3][] | addr(.)] | map(select(. != $pm and . != "0x0000000000000000000000000000000000000000"))
    else [$e] end;'

# 1. find the last K transactions that emitted an event from the hook
LATEST=$(($(call "$BASE_RPC_URL" eth_blockNumber '[]' | tr -d '"')))
TXS=()
TO=$LATEST
while [ ${#TXS[@]} -lt "$K" ] && [ $TO -gt $((LATEST - 3000)) ]; do
  FROM=$((TO - 9))
  while read -r H; do
    [ -n "$H" ] && [[ ! " ${TXS[*]} " =~ " $H " ]] && TXS+=("$H")
  done < <(call "$BASE_RPC_URL" eth_getLogs "[{\"address\":\"$HOOK\",\"fromBlock\":\"$(hex $FROM)\",\"toBlock\":\"$(hex $TO)\"}]" \
    | jq -r '.[]?.transactionHash' | tac)
  TO=$((FROM - 1))
  sleep 0.05
done
TXS=("${TXS[@]:0:$K}")
echo "replaying ${#TXS[@]} transactions in $MODE mode"

# send <tx json>: impersonate the sender and queue the transaction with its real fees; prints the local hash
send() {
  local T=$1
  call $LOCAL anvil_impersonateAccount "[$(echo "$T" | jq -c .from)]" >/dev/null
  local S
  if echo "$T" | jq -e '.maxFeePerGas' >/dev/null; then
    S=$(echo "$T" | jq -c '{from, to, data: .input, value, gas, nonce, maxFeePerGas, maxPriorityFeePerGas}')
  else
    S=$(echo "$T" | jq -c '{from, to, data: .input, value, gas, nonce, gasPrice}')
  fi
  call $LOCAL eth_sendTransaction "[$S]" | tr -d '"'
}

NORM='[.logs[] | {a: (.address | ascii_downcase), t: [.topics[] | ascii_downcase], d: (.data | ascii_downcase)}]'
SAME=0
for TX in "${TXS[@]}"; do
  START=$(date +%s)
  REAL_TX=$(call "$BASE_RPC_URL" eth_getTransactionByHash "[\"$TX\"]")
  REAL=$(call "$BASE_RPC_URL" eth_getTransactionReceipt "[\"$TX\"]")
  BLOCK=$(($(echo "$REAL_TX" | jq -r .blockNumber)))
  IDX=$(($(echo "$REAL_TX" | jq -r .transactionIndex)))
  BLK=$(call "$BASE_RPC_URL" eth_getBlockByNumber "[\"$(hex $BLOCK)\",false]")
  TS=$(($(echo "$BLK" | jq -r .timestamp)))
  BASEFEE=$(echo "$BLK" | jq -r .baseFeePerGas)

  # earlier transactions in the block that touched anything the target touched, in block order
  PRE=()
  if [ "$MODE" = "related" ]; then
    WANT=$(echo "$REAL" | jq -c --arg pm "$PM" --arg tr "$TRANSFER" "$TOUCH [.logs[] | touch[]] | unique")
    while read -r I H; do
      [ -n "$H" ] && [ "$I" -lt "$IDX" ] && PRE+=("$H")
    done < <(call "$BASE_RPC_URL" eth_getLogs "[{\"fromBlock\":\"$(hex $BLOCK)\",\"toBlock\":\"$(hex $BLOCK)\"}]" \
      | jq -r --arg pm "$PM" --arg tr "$TRANSFER" --argjson want "$WANT" \
          "$TOUCH .[] | select(any(touch[]; . as \$x | \$want | index(\$x) != null)) | \"\(.transactionIndex) \(.transactionHash)\"" \
      | while read -r HI HH; do echo "$((HI)) $HH"; done | sort -n -u)
  fi

  anvil --fork-url "$BASE_RPC_URL" --fork-block-number $((BLOCK - 1)) --port $PORT \
    --order fifo --no-mining --silent &
  PID=$!
  for _ in $(seq 1 100); do
    [ "$(call $LOCAL eth_blockNumber '[]' 2>/dev/null)" != "" ] && break
    sleep 0.3
  done
  call $LOCAL anvil_setNextBlockBaseFeePerGas "[\"$BASEFEE\"]" >/dev/null

  for P in "${PRE[@]}"; do
    send "$(call "$BASE_RPC_URL" eth_getTransactionByHash "[\"$P\"]")" >/dev/null
  done
  SIM_HASH=$(send "$REAL_TX")
  call $LOCAL evm_setNextBlockTimestamp "[$TS]" >/dev/null
  call $LOCAL evm_mine '[]' >/dev/null
  SIM=$(call $LOCAL eth_getTransactionReceipt "[\"$SIM_HASH\"]")
  SECS=$(( $(date +%s) - START ))

  if [[ "$SIM_HASH" == *error* ]]; then
    echo "$TX (block $BLOCK, index $IDX): could not send: $SIM_HASH"
  elif [ -z "$SIM" ] || [ "$SIM" = "null" ]; then
    echo "$TX (block $BLOCK, index $IDX): not mined"
  else
    RS=$(echo "$REAL" | jq -r .status); SS=$(echo "$SIM" | jq -r .status)
    RG=$(($(echo "$REAL" | jq -r .gasUsed))); SG=$(($(echo "$SIM" | jq -r .gasUsed)))
    RL=$(echo "$REAL" | jq -c "$NORM"); SL=$(echo "$SIM" | jq -c "$NORM")
    RN=$(echo "$RL" | jq length); SN=$(echo "$SL" | jq length)
    if [ "$RL" = "$SL" ] && [ "$RS" = "$SS" ]; then MATCH="IDENTICAL events"; SAME=$((SAME + 1)); else MATCH="DIFFERENT"; fi
    echo "$TX (block $BLOCK, index $IDX, ${#PRE[@]} earlier, ${SECS}s): status $RS vs $SS, gas $RG vs $SG, events $RN vs $SN: $MATCH"
    if [ "$MATCH" = "DIFFERENT" ]; then
      I=$(jq -n --argjson r "$RL" --argjson s "$SL" '[range(0; ([$r, $s] | map(length) | max))] | map(select($r[.] != $s[.])) | first')
      echo "    first different event #$I from $(echo "$RL" | jq -r ".[$I].a")"
      jq -n --argjson r "$(echo "$RL" | jq -c ".[$I]")" --argjson s "$(echo "$SL" | jq -c ".[$I]")" -r '
        def words(d): [range(2; (d | length); 64) as $i | d[$i:$i+64]];
        if ($r.t != $s.t) then "    topics differ"
        else (words($r.d)) as $a | (words($s.d)) as $b
          | [range(0; ([$a, $b] | map(length) | max))] | map(select($a[.] != $b[.])) | first as $w
          | "    data word #\($w): real \($a[$w] | ltrimstr("0") ) vs sim \($b[$w] | ltrimstr("0"))"
        end'
    fi
  fi
  kill $PID 2>/dev/null; wait $PID 2>/dev/null
done
echo "identical: $SAME / ${#TXS[@]}"
