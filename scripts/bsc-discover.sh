#!/usr/bin/env bash
# Finds the busiest PancakeSwap Infinity hooks (CL pools) on BNB Chain over the last N blocks
# (default 4000, about 30 minutes at 0.45 s per block). Also checks archive access.
# Needs BSC_RPC_URL. Never prints the URL (it contains the key).
set -uo pipefail
: "${BSC_RPC_URL:?set BSC_RPC_URL first}"

CLPM=0xa0FfB9c1CE1Fe56963B0321B32E7A0302114058b   # PancakeSwap Infinity CLPoolManager (BNB Chain)
SWAP=$(cast keccak "Swap(bytes32,address,int128,int128,uint160,uint128,int24,uint24,uint16)")
BLOCKS=${1:-4000}
STEP=${STEP:-10}   # blocks per eth_getLogs call (10 on the Alchemy free tier)

rpc() { curl -s -X POST -H 'content-type: application/json' --data "$1" "$BSC_RPC_URL"; }
hex() { printf '0x%x' "$1"; }

LATEST=$(($(rpc '{"jsonrpc":"2.0","id":1,"method":"eth_blockNumber","params":[]}' | jq -r .result)))
echo "latest block: $LATEST"

OLD=$((LATEST - 2000000))
echo "archive check (CLPoolManager code at block $OLD, about 10 days ago):"
rpc "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"eth_getCode\",\"params\":[\"$CLPM\",\"$(hex $OLD)\"]}" \
  | jq -r 'if .error then "  ERROR: \(.error.message)" else "  OK, \((.result | length) / 2 - 1) bytes" end'

IDS=$(mktemp)
echo "scanning blocks $((LATEST - BLOCKS + 1))..$LATEST for Infinity CL swaps, $STEP blocks per call"
for ((TO = LATEST; TO > LATEST - BLOCKS; TO -= STEP)); do
  FROM=$((TO - STEP + 1))
  RES=$(rpc "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"eth_getLogs\",\"params\":[{\"address\":\"$CLPM\",\"topics\":[\"$SWAP\"],\"fromBlock\":\"$(hex $FROM)\",\"toBlock\":\"$(hex $TO)\"}]}")
  if echo "$RES" | jq -e .error >/dev/null; then
    echo "  getLogs error at $FROM..$TO: $(echo "$RES" | jq -r .error.message | cut -c1-120)"
    break
  fi
  echo "$RES" | jq -r '.result[]?.topics[1]' >> "$IDS"
  sleep 0.05
done
echo "swaps found: $(wc -l < "$IDS"), pools: $(sort -u "$IDS" | wc -l)"

symbol() {
  if [ "$1" = "0x0000000000000000000000000000000000000000" ]; then echo BNB; return; fi
  cast call "$1" "symbol()(string)" --rpc-url "$BSC_RPC_URL" 2>/dev/null | tr -d '"' | cut -c1-12 || echo "?"
}

POOLS=$(mktemp)
sort "$IDS" | uniq -c | sort -rn | head -40 | while read -r N ID; do
  KEY=$(cast call "$CLPM" "poolIdToPoolKey(bytes32)(address,address,address,address,uint24,bytes32)" "$ID" --rpc-url "$BSC_RPC_URL" 2>/dev/null)
  C0=$(echo "$KEY" | sed -n 1p); C1=$(echo "$KEY" | sed -n 2p); HOOK=$(echo "$KEY" | sed -n 3p)
  [ -z "$HOOK" ] && HOOK="unknown"
  echo "$N $HOOK $(symbol "$C0")/$(symbol "$C1") $ID" >> "$POOLS"
done

echo
echo "== busiest pools (swaps, hook, pair)"
head -15 "$POOLS" | awk '{printf "%5s  %s  %s\n", $1, $2, $3}'
echo
echo "== swaps per hook (top 40 pools only; 0x000... means no hook)"
awk '{s[$2] += $1; p[$2] += 1} END {for (h in s) printf "%6d swaps  %3d pools  %s\n", s[h], p[h], h}' "$POOLS" | sort -rn
rm -f "$IDS" "$POOLS"
