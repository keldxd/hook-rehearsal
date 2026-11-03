# hook-rehearsal

Rehearse a hook upgrade on **real mainnet transactions** before you ship it. Works with **PancakeSwap Infinity hooks on BNB Chain** and **Uniswap v4 hooks on Base**.

hook-rehearsal takes the most recent real transactions that went through a hook, rebuilds the exact state each one saw on a local fork, and runs every one of them twice: once with the deployed hook code and once with your new build at the same hook address. It then reports what the new code would change for real users:

- traders who would **end up with less** (`DANGER`), and which accounts gain the difference
- transactions that **start reverting** (or start working)
- tokens that move differently between other accounts
- different events, and gas changes above 5%

```
==== hook upgrade rehearsal: bug1_swap_gate ====
real transactions replayed:     20
could not replay:               0
deployed code matches mainnet:  20 / 20
same behaviour:                 0
different behaviour:            20  (20 of them on transactions whose replay matches mainnet exactly)

findings (transactions affected):
  REVIEW swap-now-fails  20

transactions that behave differently:

0x65be5e9a0601c0e3  swapper 0x95ec..01b3
  (replay of the deployed code matches mainnet exactly)
  REVIEW swap-now-fails  worked on the deployed code, reverts on the new code
...
```

Status: **v0.2, early.** Tested on two hooks: CLAlphaHook on BNB Chain and Clanker's ClankerHookStaticFeeV2 on Base. Read [Limits](#limits) before relying on it.

## Why

Hooks run inside every swap of their pools, and many hold or move user funds through custom accounting. Security guidance for hooks ([Uniswap v4 security framework](https://developers.uniswap.org/docs/protocols/v4/security), [CertiK on PancakeSwap Infinity hooks](https://www.certik.com/blog/pancakeswap-infinity-hooks-security-considerations)) asks teams to test changes against realistic swap flows. Existing hook security tools read the code or fuzz synthetic trades. hook-rehearsal is complementary: it replays the transactions real users actually sent, so it shows the concrete effect of a change on real traffic, including effects on bots and integrators that code review cannot see.

## Results on BNB Chain: CLAlphaHook (3 Nov 2026)

Target: `CLAlphaHook` at `0x72e09eBd9b24F47730b651889a4eD984CBa53d90`, a PancakeSwap Infinity launch-gate hook. Before a pool's start time only its listed owners can add or remove liquidity and every swap reverts; after it, anyone can trade. Deployments of this contract carried 8,135 of the 13,736 CL swaps on PancakeSwap Infinity in a 30-minute sample (about 59%, counting the 40 busiest pools). Sample: its 20 most recent transactions.

**1. Does the replay match mainnet?** The deployed code, replayed on the rebuilt state, compared with the real receipts: **20 of 20 identical** (status and every event, byte for byte) and **19 of 20 identical gas**.

**2. Does it raise false alarms?** The hook rebuilt from its verified source produces **identical bytes to the deployed contract**, and **0 of 20** transactions behave differently.

**3. Does it catch real mistakes?** Three one-line bugs planted in the source, each built and rehearsed:

| Planted bug | What the rehearsal reports |
|---|---|
| The launch check in `beforeSwap` is inverted (`!isPoolStarted` becomes `isPoolStarted`) | **20 of 20** change: every real trade now reverts |
| The start-time comparison is flipped (`<=` becomes `>=`), so a live pool counts as not started | **20 of 20** change: every real trade now reverts |
| `&&` becomes `\|\|` in `beforeRemoveLiquidity`, so after launch only listed owners could withdraw liquidity | Not exercised: none of the 20 transactions was a liquidity withdrawal (see [Limits](#limits)). **1 of 20** changed for another reason, below |

**4. A finding nobody planted.** That one transaction is an arbitrage bot buying on the hook's pool and selling on PancakeSwap V3. It reverts with *any* change to the hook's code: a build with **only an added comment** (identical behaviour, different bytes because of the compiler's metadata hash) breaks it too, while the byte-identical clean build does not. The bot pins the exact code of the hook it trades through. So even a harmless redeploy or upgrade of a hook can silently stop bots and integrators that rely on it, which a code review would not show.

These are planted bugs in a local copy, **not vulnerabilities in CLAlphaHook**. Full reports are in [`demo/bsc/`](demo/bsc/). [`scripts/alpha-demo.sh`](scripts/alpha-demo.sh) rebuilds all builds from the verified source (the standard JSON input from BscScan) and reruns everything on that day's transactions, so a rerun gives different counts.

## Results on Base: Clanker's hook (2 Nov 2026)

Target: `ClankerHookStaticFeeV2` at `0xb429d62f8f3bFFb98CdB9569533eA23bF0Ba28CC`, a Uniswap v4 hook with return-delta fee accounting. Sample: its 10 most recent transactions.

- **Fidelity:** 9 of 10 identical to the real receipts, 10 of 10 identical gas.
- **Clean rebuild** from [source](https://github.com/clanker-devco/v4-contracts): 0 of 10 behave differently.
- **Planted bugs:**

| Planted bug | What the rehearsal reports |
|---|---|
| Protocol fee raised from 20% to 25% of the LP fee | **9 of 10** change. **DANGER on 4**: swappers lose 0.05%, gained by the PoolManager and the fee recipient |
| Exact-input buys use the scaling formula of the neighbouring branch | **5 of 10** change, exactly the transactions that take that branch. **DANGER on 2** |
| The LP locker fee claim gets the paired token instead of the Clanker token | **9 of 10** change. **5 swaps now revert**, **3 sellers receive 0 WETH**, one swap uses **406% more gas** |

Not vulnerabilities in Clanker. Full outputs in [`demo/`](demo/), rerun with [`scripts/clanker-demo.sh`](scripts/clanker-demo.sh).

## How it works

1. `fetch` finds the hook's recent transactions. On BNB Chain it reads the CLPoolManager's events and keeps those of pools that use the hook, so hooks that emit no events of their own are covered. On Base it uses the hook's own events.
2. Each transaction is replayed on an [anvil](https://getfoundry.sh/anvil/overview) fork of the block before it, with the earlier transactions of its block replayed first, in their real order, in one block with the real timestamp, base fee and transaction fees, sent as the real senders. On BNB Chain **every** earlier transaction in the block is replayed (exact); on Base only the ones that touched the same pools, contracts or token holders.
3. For a new build, the hook's constructor runs **at the hook's own address** (so immutables are real), and the resulting code replaces the hook's code. Storage is kept, as in an in-place upgrade.
4. Both runs start from the same snapshot. Value moved is measured from ERC20 transfers; the trader is the transaction's sender.

## Usage

Needs Rust, [Foundry](https://getfoundry.sh) (anvil, cast) and an **archive** RPC (an Alchemy free-tier key works for both chains, slowly).

```
git clone https://github.com/keldxd/hook-rehearsal
cd hook-rehearsal

# BNB Chain (PancakeSwap Infinity)
export CHAIN=bsc
export BSC_RPC_URL=https://bnb-mainnet.g.alchemy.com/v2/<your key>
./scripts/bsc-discover.sh                           # busiest Infinity hooks right now

# or Base (Uniswap v4)
export BASE_RPC_URL=https://base-mainnet.g.alchemy.com/v2/<your key>

cargo run --release -- fetch <hook address> 10      # the 10 most recent transactions (cache/)
cargo run --release -- replay                       # fidelity check against the real receipts
cargo run --release -- rehearse <constructor args hex> out/MyHook.sol/MyHook.json [more builds...]
```

`<constructor args hex>` is the ABI-encoded constructor arguments of the new build (`cast abi-encode "f(address,...)" ...`), or `0x` if it has none. Each build's full report is written to `reports/<build name>.txt`.

## Limits

- **No findings does not mean safe.** Only what real users did is replayed. A bug on a path no recent transaction took (like the liquidity-withdrawal bug above) is not exercised, and neither is one only an attacker would trigger. Use static scanners and fuzzing for that.
- **CL pools only on BNB Chain** for now (no Bin pools), and Base only for Uniswap v4.
- **The new build must keep the same hook permissions**, since it replaces code at the same address. A v2 at a new address is not supported yet. Library linking is not supported.
- **On Base, earlier transactions in the same block are selected by what they touched.** Indirect dependency chains are missed, which caused the 1 mismatch above.
- **Value is measured from ERC20 transfers.** Native BNB or ETH movements are not counted yet.
- **Speed:** slow on a free-tier RPC, because anvil downloads the state of every earlier transaction in the block as it goes. The 20-transaction BNB Chain demo with 4 builds took about 4 hours on an Alchemy free key from Kenya. A paid or local RPC is much faster, and reruns on the same blocks can reuse anvil's cache.

## Roadmap

- Speed: run forks in parallel and reuse one fork for transactions in the same block.
- Bin pools on PancakeSwap Infinity, and Uniswap v4 on BNB Chain and Ethereum.
- v1 to v2 migration: rehearse a new version deployed at a different address by recreating its pools.
- Native BNB and ETH flows, and a check that flags integrators that pin a hook's code.
- JSON output and a GitHub Action, so a hook team can run a rehearsal on every pull request.

## License

MIT
