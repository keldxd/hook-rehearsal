#!/usr/bin/env bash
# Finds the busiest Uniswap v4 hooks on Base over the last N blocks (default 600, about 20 minutes).
# Needs BASE_RPC_URL. Works with the Alchemy free tier (10-block eth_getLogs limit).
set -uo pipefail
: "${BASE_RPC_URL:?set BASE_RPC_URL first}"

PM=0x498581ff718922c3f8e6a244956af099b2652b2b     # PoolManager (Base)
POSM=0x7c5f5a4bbd8fd63184577525326123b519429bdc   # PositionManager (Base), stores each pool's key
SWAP=$(cast keccak "Swap(bytes32,address,int128,int128,uint160,uint128,int24,uint24)")
BLOCKS=${1:-600}

rpc() { curl -s -X POST -H 'content-type: application/json' --data "$1" "$BASE_RPC_URL"; }
hex() { printf '0x%x' "$1"; }

LATEST=$(($(rpc '{"jsonrpc":"2.0","id":1,"method":"eth_blockNumber","params":[]}' | jq -r .result)))
IDS=$(mktemp)
echo "scanning blocks $((LATEST - BLOCKS + 1))..$LATEST for v4 swaps"
for ((TO = LATEST; TO > LATEST - BLOCKS; TO -= 10)); do
  FROM=$((TO - 9))
  RES=$(rpc "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"eth_getLogs\",\"params\":[{\"address\":\"$PM\",\"topics\":[\"$SWAP\"],\"fromBlock\":\"$(hex $FROM)\",\"toBlock\":\"$(hex $TO)\"}]}")
  echo "$RES" | jq -r '.result[]?.topics[1]' >> "$IDS"
  sleep 0.05
done
echo "swaps found: $(wc -l < "$IDS"), pools: $(sort -u "$IDS" | wc -l)"

symbol() {
  if [ "$1" = "0x0000000000000000000000000000000000000000" ]; then echo ETH; return; fi
  cast call "$1" "symbol()(string)" --rpc-url "$BASE_RPC_URL" 2>/dev/null | tr -d '"' | cut -c1-12 || echo "?"
}

POOLS=$(mktemp)
sort "$IDS" | uniq -c | sort -rn | head -40 | while read -r N ID; do
  KEY=$(cast call "$POSM" "poolKeys(bytes25)(address,address,uint24,int24,address)" "${ID:0:52}" --rpc-url "$BASE_RPC_URL" 2>/dev/null)
  C0=$(echo "$KEY" | sed -n 1p); C1=$(echo "$KEY" | sed -n 2p); HOOK=$(echo "$KEY" | sed -n 5p)
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
