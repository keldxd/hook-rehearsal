#!/usr/bin/env bash
# Rebuilds CLAlphaHook (PancakeSwap Infinity launch-gate hook on BNB Chain) from its verified
# source, clean, with a comment-only change, and with three planted bugs, then rehearses all five
# builds on the hook's most recent real transactions.
# Needs: BSC_RPC_URL (an archive RPC for BNB Chain), Foundry (forge, cast, anvil), python3.
# The source is the verified standard JSON from BscScan for 0x72e09eBd...3d90, saved as a file.
# Usage: scripts/alpha-demo.sh <source.json> [number of transactions, default 10]
# Slow on a free-tier RPC: every earlier transaction in each block is replayed too, and anvil
# fetches their state as it goes (20 transactions took about 4 hours on an Alchemy free key).
set -euo pipefail
: "${BSC_RPC_URL:?set BSC_RPC_URL to an archive RPC endpoint for BNB Chain}"
export CHAIN=bsc

SOURCE_JSON=$(realpath "${1:?usage: scripts/alpha-demo.sh <source.json> [count]}")
N=${2:-10}
HOOK=0x72e09eBd9b24F47730b651889a4eD984CBa53d90
CLPM=0xa0FfB9c1CE1Fe56963B0321B32E7A0302114058b      # CLPoolManager (constructor argument 1)
DEPLOY_OWNER=0xEe7b429Ea01F76102f053213463D4e95D5D24AE8   # constructor argument 2, as deployed
ROOT=$(cd "$(dirname "$0")/.." && pwd)
WORK="${WORK:-$ROOT/target/alpha-demo}"
TREE="$WORK/source"
BUILDS="$WORK/builds"
HOOK_FILE="$TREE/src/pool-cl/CLAlphaHook.sol"
mkdir -p "$WORK" "$BUILDS"

# 1. Unpack the verified sources and write a Foundry config with the verified settings
rm -rf "$TREE"
python3 -I - "$SOURCE_JSON" "$TREE" <<'PY'
import json, os, sys
src, tree = sys.argv[1], sys.argv[2]
data = json.load(open(src))
settings = json.loads(data["sources"]["settings.json"]["content"])
for path, body in data["sources"].items():
    if path == "settings.json":
        continue
    out = os.path.join(tree, path)
    os.makedirs(os.path.dirname(out), exist_ok=True)
    open(out, "w").write(body["content"])
opt = settings.get("optimizer", {})
meta = settings.get("metadata", {})
lines = [
    "[profile.default]",
    'src = "src"',
    'out = "out"',
    'libs = ["lib"]',
    'solc_version = "0.8.26"',
    f'optimizer = {str(opt.get("enabled", False)).lower()}',
    f'optimizer_runs = {opt.get("runs", 200)}',
    f'via_ir = {str(settings.get("viaIR", False)).lower()}',
    f'evm_version = "{settings.get("evmVersion", "cancun")}"',
    f'bytecode_hash = "{meta.get("bytecodeHash", "ipfs")}"',
    f'cbor_metadata = {str(meta.get("appendCBOR", True)).lower()}',
    "remappings = [",
] + [f'    "{r}",' for r in settings.get("remappings", [])] + ["]", "", "[lint]", "lint_on_build = false"]
open(os.path.join(tree, "foundry.toml"), "w").write("\n".join(lines) + "\n")
print(f"unpacked {len(data['sources']) - 1} source files; optimizer {opt.get('enabled')}, "
      f"via_ir {settings.get('viaIR')}, evm {settings.get('evmVersion')}")
PY

cd "$TREE"
cp "$HOOK_FILE" "$WORK/CLAlphaHook.original.sol"
ART=out/CLAlphaHook.sol/CLAlphaHook.json
build() {
  forge build >/dev/null
  cp "$ART" "$BUILDS/$1.json"
  cp "$WORK/CLAlphaHook.original.sol" "$HOOK_FILE"
  echo "built $1"
}
# Replaces exactly one occurrence of a text, after an optional anchor; fails loudly otherwise
plant() {
  python3 -I - "$HOOK_FILE" "$1" "$2" "${3:-}" <<'PY'
import sys
path, old, new, anchor = sys.argv[1:5]
text = open(path).read()
start = text.index(anchor) if anchor else 0
head, tail = text[:start], text[start:]
if anchor:
    if old not in tail:
        sys.exit(f"bug not planted: {old!r} not found after {anchor!r}")
    tail = tail.replace(old, new, 1)
else:
    if text.count(old) != 1:
        sys.exit(f"bug not planted: {old!r} found {text.count(old)} times")
    tail = tail.replace(old, new)
open(path, "w").write(head + tail)
PY
}

build clean

# A comment only: behaviour is unchanged, but the code bytes (metadata hash) are not
echo "// comment only: behaviour is unchanged" >> "$HOOK_FILE"
build comment_only

# Bug 1: the launch check in beforeSwap is inverted, so trading fails once the pool is live
plant 'if (!isPoolStarted(key.toId())) {' 'if (isPoolStarted(key.toId())) {'
build bug1_swap_gate

# Bug 2: && becomes || in beforeRemoveLiquidity, so after launch only listed owners can withdraw
plant '&& !isPoolOwner' '|| !isPoolOwner' 'function _beforeRemoveLiquidity'
build bug2_lp_lock

# Bug 3: the start-time comparison is flipped, so a pool counts as started only before its start time
plant 'poolStartedTimestamp[poolId] <= block.timestamp' 'poolStartedTimestamp[poolId] >= block.timestamp'
build bug3_time

# 2. Real transactions and the rehearsal of all five builds (the rehearsal also reports how many
# replays of the deployed code match mainnet; run `hook-rehearsal replay` for the gas check too)
CTOR=$(cast abi-encode "f(address,address)" "$CLPM" "$DEPLOY_OWNER")
cd "$ROOT"
cargo build --release -q
./target/release/hook-rehearsal fetch "$HOOK" "$N"
./target/release/hook-rehearsal rehearse "$CTOR" "$BUILDS/clean.json" "$BUILDS/comment_only.json" \
  "$BUILDS/bug1_swap_gate.json" "$BUILDS/bug2_lp_lock.json" "$BUILDS/bug3_time.json"
echo "full reports are in reports/"
