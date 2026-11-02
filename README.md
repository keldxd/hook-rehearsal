# hook-rehearsal

Rehearse a Uniswap v4 hook upgrade on **real mainnet swaps** before you ship it.

hook-rehearsal takes the most recent real transactions that went through a hook, rebuilds the state each one saw on a local fork, and runs every one of them twice: once with the deployed hook code and once with your new build at the same hook address. It then reports what the new code would change for real users:

- **swappers who would end up with less** (`DANGER`), and which accounts gain the difference
- swaps that **start reverting** (or start working)
- tokens that move differently between other accounts
- different events, and gas changes above 5%

```
==== hook upgrade rehearsal: bug1_fee ====
real transactions replayed:     10
deployed code matches mainnet:  9 / 10
same behaviour:                 1
different behaviour:            9  (8 of them on transactions whose replay matches mainnet exactly)

findings (transactions affected):
  DANGER swapper-loses   4
  REVIEW events-changed  9
  REVIEW flows-changed   5

DANGER: swappers who would lose value with the new code:
  0x8accea636955a0c4  swapper ends with 0.161971 ClawBank less (-0.050%); gained by: PoolManager +0.161971
  0x838901b95c650c64  swapper ends with 0.000000202846 WETH less (-0.050%); gained by: 0xe85a..83a9 +0.0000203067
  ...
```

Status: **v0.1, early.** Tested on one hook (Clanker's ClankerHookStaticFeeV2 on Base). Read [Limits](#limits) before relying on it.

## Why

Hooks run inside every swap of their pools, and many hold or move user funds through custom accounting. The [Uniswap v4 security framework](https://developers.uniswap.org/docs/protocols/v4/security) asks teams to test upgrades with simulations of swap flows, and recommends shipping new versions instead of upgrading in place. Existing hook security tools ([hookguard](https://github.com/chaosxcode/hookguard), [hookrisk](https://github.com/0xmvercosa/hookrisk), [HookScan](https://github.com/blocksecteam/hookscan) and others) read the code or fuzz synthetic trades. hook-rehearsal is complementary: it replays the swaps real users actually made, so it shows the concrete effect of a change on real traffic, in token amounts.

## Results on Clanker's hook (Base mainnet, 2 Nov 2026)

Target: `ClankerHookStaticFeeV2` at `0xb429d62f8f3bFFb98CdB9569533eA23bF0Ba28CC`, a hook with return-delta fee accounting across four swap cases. Sample: its 10 most recent transactions.

**1. Does the replay match mainnet?** The deployed code, replayed on the rebuilt state, compared with the real receipts: **9 of 10 identical** (status and every event, byte for byte) and **10 of 10 identical gas**. The one difference is a dependency chain through another transaction earlier in the same block (see Limits).

**2. Does it raise false alarms?** Clanker's hook rebuilt from [source](https://github.com/clanker-devco/v4-contracts) with no changes: **0 of 10** transactions behave differently.

**3. Does it catch real mistakes?** Three one-line bugs planted in the source, each built and rehearsed:

| Planted bug | What the rehearsal reports |
|---|---|
| Protocol fee raised from 20% to 25% of the LP fee (a "config tweak") | **9 of 10** change. **DANGER on 4**: swappers lose 0.05%, gained by the PoolManager (the hook's balance) and the fee recipient |
| Exact-input buys use the scaling formula of the neighbouring branch (a copy-paste error) | **5 of 10** change, exactly the transactions that take that branch. **DANGER on 2** |
| The LP locker fee claim gets the paired token instead of the Clanker token | **9 of 10** change. **5 swaps now revert**, **3 sellers receive 0 WETH**, one swap uses **406% more gas** |

These are planted bugs in a local copy, **not vulnerabilities in Clanker**. Full outputs are in [`demo/`](demo/). [`scripts/clanker-demo.sh`](scripts/clanker-demo.sh) rebuilds all four hooks from a pinned commit and reruns everything on that day's transactions, so a rerun gives different counts.

## How it works

1. `fetch` finds the hook's recent transactions through the hook's own events (works with free-tier RPC limits), and for each one records the **earlier transactions in the same block** that touched the same v4 pools, contracts or token holders.
2. Each transaction is replayed on an [anvil](https://getfoundry.sh/anvil/overview) fork of the block before it: the related earlier transactions first, then the target, in their real order, in one block with the real timestamp, base fee and transaction fees, sent as the real senders.
3. For a new build, the hook's constructor runs **at the hook's own address** (so immutables and the hook-address permission check are real), and the resulting code replaces the hook's code. Storage is kept, as in an in-place upgrade.
4. Both runs start from the same snapshot. Value moved is measured from ERC20 transfers; the swapper is the transaction's sender.

## Usage

Needs Rust, [Foundry](https://getfoundry.sh) (anvil) and an **archive** RPC for Base (an Alchemy free-tier key works).

```
git clone https://github.com/keldxd/hook-rehearsal
cd hook-rehearsal
export BASE_RPC_URL=https://base-mainnet.g.alchemy.com/v2/<your key>

cargo run --release -- fetch <hook address> 20      # the 20 most recent transactions (cache/)
cargo run --release -- replay                       # fidelity check against the real receipts
cargo run --release -- rehearse <constructor args hex> out/MyHook.sol/MyHook.json [more builds...]
```

`<constructor args hex>` is the ABI-encoded constructor arguments of the new build (`cast abi-encode "f(address,...)" ...`), or `0x` if it has none. Each build's full report is written to `reports/<build name>.txt`.

## Limits

- **No findings does not mean safe.** Only what real users did is replayed. A bug that only an attacker would trigger (for example a removed access check on a callback) is not exercised. Use static scanners and fuzzing for that.
- **Base only in v0.1**, and the hook must emit events of its own (that is how its transactions are found).
- **The new build must keep the same hook permission flags**, since it replaces code at the same address. A v2 at a new address is not supported yet. Library linking is not supported.
- **Earlier transactions in the same block are selected by what they touched.** Indirect dependency chains are missed, which caused the 1 mismatch above.
- **Value is measured from ERC20 transfers.** Native ETH movements are not counted yet, and the swapper is assumed to be the transaction's sender.
- **Speed:** on a free-tier RPC, the first replay of a transaction takes about 30 to 100 seconds, because anvil fetches state as it goes. Reruns use anvil's cache.

## Roadmap

- v1 to v2 migration: rehearse a new version deployed at a different address by recreating its pools.
- Native ETH flows, recipient-aware value accounting, and storage-layout checks for upgradeable hooks.
- Ethereum, Unichain and Arbitrum, plus discovery of transactions for hooks that emit no events.
- JSON output and a GitHub Action.

## License

MIT
