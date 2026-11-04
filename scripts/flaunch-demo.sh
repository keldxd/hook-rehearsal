#!/usr/bin/env bash
# Rebuilds a Flaunch hook on Base from its verified source, clean and with three planted one-line
# bugs in its fee code, then rehearses all four builds on the hook's most recent real transactions.
# Needs: BASE_RPC_URL (an archive RPC for Base), Foundry (forge, anvil), python3.
# The source is the verified standard JSON input of the hook (from Basescan or Sourcify), saved as a file.
# Usage: TARGET=any|pm scripts/flaunch-demo.sh <source.json> [number of transactions, default 10]
#   any: AnyPositionManager 0x8DC3b85e...e5dC (default)
#   pm:  PositionManager v1.3.1 0x588C683E...25DC
set -euo pipefail
: "${BASE_RPC_URL:?set BASE_RPC_URL to an archive RPC endpoint for Base}"
export CHAIN=base

SOURCE_JSON=$(realpath "${1:?usage: TARGET=any|pm scripts/flaunch-demo.sh <source.json> [count]}")
N=${2:-10}
TARGET=${TARGET:-any}
# Constructor arguments of each deployed hook, as shown on Basescan (ABI-encoded ConstructorParams)
case "$TARGET" in
  any)
    HOOK=0x8DC3b85e1dc1C846ebf3971179a751896842e5dC
    NAME=AnyPositionManager
    SOLC=0.8.27
    CTOR=0x000000000000000000000000000000000d564d5be76f7f0d28fe52605afc7cf8000000000000000000000000498581ff718922c3f8e6a244956af099b2652b2b000000000000000000000000000000000000000000000000000000000000006400000000000000000000000000000000000000000000000000000000000001f400000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000001000000000000000000000000d565c460922ea1220eb65733b111cc8bde5a2a93000000000000000000000000b8a70b4d1547bf6193bd67a73f4f98ea9fd0a9730000000000000000000000001150c53eb4ce3ade47808d1d1ac9636b774ee079000000000000000000000000000000000bb63d5c070d0d5791517886a4d8c54500000000000000000000000072e6f7948b1b1a343b477f39aabd2e35e6d27dde000000000000000000000000fdce459071c74b732b2dec579afb38ea552c4e06000000000000000000000000fb5c20c4e60c9c64648dd3692437e3e313add4a40000000000000000000000002154c604df568a5285284d1c4918dc98c39240df
    ;;
  pm)
    HOOK=0x588C683EcC450F8b2aAdb13D7f63792b840425DC
    NAME=PositionManager
    SOLC=0.8.26
    CTOR=0x000000000000000000000000498581ff718922c3f8e6a244956af099b2652b2b000000000000000000000000000000000000000000000000000000000000006400000000000000000000000000000000000000000000000000000000000003e800000000000000000000000000000000000000000000000000000000000003e80000000000000000000000000000000000000000000000000000000000000001000000000000000000000000dc89befab80b22e27a75af6eb1f96984f5172225000000000000000000000000b8a70b4d1547bf6193bd67a73f4f98ea9fd0a97300000000000000000000000054cd200a46cd5081825cf6a0afe3da5c78afbcc4000000000000000000000000000000000bb63d5c070d0d5791517886a4d8c545000000000000000000000000fdce459071c74b732b2dec579afb38ea552c4e06000000000000000000000000fb5c20c4e60c9c64648dd3692437e3e313add4a40000000000000000000000000dae90b70f62ce3b1d5278f4763bd1f595d6a687000000000000000000000000331d433f2d8f3df1bf29d796e12836603045cfa2000000000000000000000000b91f808b74817dea46fc5cc17eafb67a9542274a00000000000000000000000026958422636655b5a4ece23a062e2eb61332c6da
    ;;
  *) echo "TARGET must be any or pm" >&2; exit 1 ;;
esac
ROOT=$(cd "$(dirname "$0")/.." && pwd)
WORK="${WORK:-$ROOT/target/flaunch-demo-$TARGET}"
TREE="$WORK/source"
BUILDS="$WORK/builds"
BUG_FILE="$TREE/src/contracts/hooks/FeeDistributor.sol"
mkdir -p "$WORK" "$BUILDS"

# 1. Unpack the verified sources and write a Foundry config with the verified settings,
#    including the linked library addresses
rm -rf "$TREE"
python3 -I - "$SOURCE_JSON" "$TREE" "$SOLC" <<'PY'
import json, os, re, sys
src, tree, solc = sys.argv[1], sys.argv[2], sys.argv[3]
data = json.load(open(src))
sources = data["sources"]
settings = json.loads(sources["settings.json"]["content"]) if "settings.json" in sources else data["settings"]
for path, body in sources.items():
    if path == "settings.json":
        continue
    out = os.path.join(tree, path)
    os.makedirs(os.path.dirname(out), exist_ok=True)
    open(out, "w").write(body["content"])
# Linked libraries: find the file that defines each library
libs = []
for _, names in (settings.get("libraries") or {}).items():
    for name, addr in names.items():
        where = [p for p, b in sources.items() if re.search(r"^\s*library\s+%s\b" % re.escape(name), b["content"], re.M)]
        if not where:
            sys.exit(f"library {name} not found in the sources")
        libs.append(f"{where[0]}:{name}:{addr}")
opt = settings.get("optimizer", {})
meta = settings.get("metadata", {})
lines = [
    "[profile.default]",
    'src = "src"',
    'out = "out"',
    'libs = ["lib"]',
    f'solc_version = "{solc}"',
    f'optimizer = {str(opt.get("enabled", False)).lower()}',
    f'optimizer_runs = {opt.get("runs", 200)}',
    f'via_ir = {str(settings.get("viaIR", False)).lower()}',
    f'evm_version = "{settings.get("evmVersion", "cancun")}"',
    f'bytecode_hash = "{meta.get("bytecodeHash", "ipfs")}"',
    f'cbor_metadata = {str(meta.get("appendCBOR", True)).lower()}',
    "libraries = [" + ", ".join(f'"{l}"' for l in libs) + "]",
    "remappings = [",
] + [f'    "{r}",' for r in settings.get("remappings", [])] + ["]", "", "[lint]", "lint_on_build = false"]
open(os.path.join(tree, "foundry.toml"), "w").write("\n".join(lines) + "\n")
print(f"unpacked {len(sources) - 1} source files; optimizer {opt.get('enabled')} ({opt.get('runs')} runs), "
      f"via_ir {settings.get('viaIR')}, evm {settings.get('evmVersion')}, linked: {libs}")
PY

cd "$TREE"
cp "$BUG_FILE" "$WORK/FeeDistributor.original.sol"
ART="out/$NAME.sol/$NAME.json"
build() {
  forge build >/dev/null
  cp "$ART" "$BUILDS/$1.json"
  cp "$WORK/FeeDistributor.original.sol" "$BUG_FILE"
  echo "built $1"
}
# Replaces exactly one occurrence of a text; fails loudly otherwise
plant() {
  python3 -I - "$BUG_FILE" "$1" "$2" <<'PY'
import sys
path, old, new = sys.argv[1:4]
text = open(path).read()
if text.count(old) != 1:
    sys.exit(f"bug not planted: {old!r} found {text.count(old)} times")
open(path, "w").write(text.replace(old, new))
PY
}

build clean

# Bug 1: the swap fee is divided by the wrong constant (10% instead of 100%), so swappers pay 10x
plant 'swapFee_ = _swapAmount * baseSwapFee / ONE_HUNDRED_PERCENT;' \
      'swapFee_ = _swapAmount * baseSwapFee / MAX_PROTOCOL_ALLOCATION;'
build bug1_fee_10x

# Bug 2: the protocol's cut of the fees reads the wrong field of the fee distribution
plant 'protocol_ = _amount * _poolFeeDistribution.protocol / ONE_HUNDRED_PERCENT;' \
      'protocol_ = _amount * _poolFeeDistribution.swapFee / ONE_HUNDRED_PERCENT;'
build bug2_protocol_field

# Bug 3: the creator's share is not taken out of what goes to the bid wall (protocol's is, twice)
plant '_amount -= creator_;' '_amount -= protocol_;'
build bug3_double_count

# 2. Real transactions, fidelity check, and the rehearsal of all four builds
cd "$ROOT"
cargo build --release -q
./target/release/hook-rehearsal fetch "$HOOK" "$N"
./target/release/hook-rehearsal replay
./target/release/hook-rehearsal rehearse "$CTOR" "$BUILDS/clean.json" "$BUILDS/bug1_fee_10x.json" \
  "$BUILDS/bug2_protocol_field.json" "$BUILDS/bug3_double_count.json"
echo "full reports are in reports/"
