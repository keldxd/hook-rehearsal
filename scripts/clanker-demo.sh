#!/usr/bin/env bash
# Rebuilds Clanker's ClankerHookStaticFeeV2 (Base) from source, clean and with three planted
# bugs, then rehearses all four builds on the hook's most recent real transactions.
# Needs: BASE_RPC_URL (an archive RPC for Base), Foundry (forge, cast, anvil), git, jq.
# Usage: scripts/clanker-demo.sh [number of transactions, default 10]
set -euo pipefail
: "${BASE_RPC_URL:?set BASE_RPC_URL to an archive RPC endpoint for Base}"

N=${1:-10}
HOOK=0xb429d62f8f3bFFb98CdB9569533eA23bF0Ba28CC
CLANKER_COMMIT=b004c2edda29fa282a16d5d1441a26484f70b37f
ROOT=$(cd "$(dirname "$0")/.." && pwd)
WORK="${WORK:-$ROOT/target/clanker-demo}"
SRC="$WORK/v4-contracts"
BUILDS="$WORK/builds"
mkdir -p "$WORK" "$BUILDS"

# 1. Clanker source at a pinned commit, with only the dependencies the hook needs
if [ ! -d "$SRC" ]; then
  git clone -q https://github.com/clanker-devco/v4-contracts.git "$SRC"
fi
cd "$SRC"
git checkout -q "$CLANKER_COMMIT"
for m in forge-std openzeppelin-contracts v4-core v4-periphery permit2; do
  git submodule update --init --depth 1 "lib/$m"
done
git -C lib/v4-periphery submodule update --init --recursive --depth 1
if [ ! -f lib/optimism/packages/contracts-bedrock/src/libraries/Predeploys.sol ]; then
  # The hook needs 3 files from the Optimism monorepo; fetch only those
  OP=$(git ls-tree HEAD lib/optimism | awk '{print $3}')
  rm -rf lib/optimism && mkdir -p lib/optimism
  git -C lib/optimism init -q
  git -C lib/optimism remote add origin https://github.com/ethereum-optimism/optimism.git
  git -C lib/optimism fetch -q --depth 1 --filter=blob:none origin "$OP"
  git -C lib/optimism sparse-checkout set --no-cone \
    /packages/contracts-bedrock/interfaces/L2/IERC7802.sol \
    /packages/contracts-bedrock/src/libraries/Predeploys.sol \
    /packages/contracts-bedrock/src/libraries/errors/CommonErrors.sol
  git -C lib/optimism checkout -q FETCH_HEAD
fi
# The repo imports IClankerLpLocker.sol but the file is IClankerLPLocker.sol
# (fine on case-insensitive macOS, fails on Linux); add the expected name
cp src/interfaces/IClankerLPLocker.sol src/interfaces/IClankerLpLocker.sol

ART=out/ClankerHookStaticFeeV2.sol/ClankerHookStaticFeeV2.json
build() {
  forge build src/hooks/ClankerHookStaticFeeV2.sol >/dev/null
  cp "$ART" "$BUILDS/$1.json"
  git checkout -q -- src/hooks
}
must_change() {
  if git diff --quiet -- src/hooks; then echo "patch for $1 did not apply" >&2; exit 1; fi
}

git checkout -q -- src/hooks
build clean

# Bug 1: protocol fee raised from 20% to 25% of the LP fee
sed -i '49s/PROTOCOL_FEE_NUMERATOR = 200_000;/PROTOCOL_FEE_NUMERATOR = 250_000;/' src/hooks/ClankerHookV2.sol
must_change bug1; build bug1_fee

# Bug 2: exact-input buys use the scaling formula of the exact-output branch
sed -i '465s/(1_000_000 + protocolFee)/(1_000_000 - protocolFee)/' src/hooks/ClankerHookV2.sol
must_change bug2; build bug2_math

# Bug 3: the LP locker fee claim passes the paired token instead of the Clanker token
sed -i '406s/poolKey.currency0/poolKey.currency1/; 407s/poolKey.currency1/poolKey.currency0/' src/hooks/ClankerHookV2.sol
must_change bug3; build bug3_token

# 2. Constructor arguments, read from the deployed hook
CTOR=$(cast abi-encode "f(address,address,address,address)" \
  "$(cast call $HOOK 'poolManager()(address)' --rpc-url "$BASE_RPC_URL")" \
  "$(cast call $HOOK 'factory()(address)' --rpc-url "$BASE_RPC_URL")" \
  "$(cast call $HOOK 'poolExtensionAllowlist()(address)' --rpc-url "$BASE_RPC_URL")" \
  "$(cast call $HOOK 'weth()(address)' --rpc-url "$BASE_RPC_URL")")

# 3. Real transactions, fidelity check, and the rehearsal of all four builds
cd "$ROOT"
cargo build --release -q
cargo run --release -q -- fetch "$HOOK" "$N"
cargo run --release -q -- replay
cargo run --release -q -- rehearse "$CTOR" "$BUILDS/clean.json" "$BUILDS/bug1_fee.json" \
  "$BUILDS/bug2_math.json" "$BUILDS/bug3_token.json"
echo "full reports are in reports/"
