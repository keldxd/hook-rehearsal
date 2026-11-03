use crate::rpc::Rpc;
use crate::util::{
    TRANSFER, ZERO, all_earlier, chain, hex, is_hub, is_manager, num, s, topic_addr,
};
use anyhow::Result;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;

pub const CACHE: &str = "cache/txs.json";

/// What a log touches: the pool for pool manager logs, the two accounts for token
/// transfers (not the token itself, everyone uses WETH, and not the pool manager or vault),
/// otherwise the emitting contract.
pub fn touch(log: &Value) -> Vec<String> {
    let addr = s(&log["address"]);
    let topics: Vec<String> = log["topics"]
        .as_array()
        .map(|a| a.iter().map(s).collect())
        .unwrap_or_default();
    if is_manager(&addr) {
        vec![format!(
            "pool:{}",
            topics.get(1).cloned().unwrap_or_default()
        )]
    } else if topics.first().map(String::as_str) == Some(TRANSFER) && topics.len() >= 3 {
        topics[1..3]
            .iter()
            .map(|t| topic_addr(t))
            .filter(|a| !is_hub(a) && a != ZERO)
            .collect()
    } else {
        vec![addr]
    }
}

/// The hook of a pool, from the pool manager's poolIdToPoolKey(bytes32) (PancakeSwap Infinity).
fn pool_hook(rpc: &Rpc, manager: &str, pool: &str) -> Result<String> {
    if pool.len() != 66 {
        return Ok(String::new());
    }
    // selector of poolIdToPoolKey(bytes32)
    let data = format!("0x0e2d484a{}", &pool[2..]);
    let out = s(&rpc.call("eth_call", json!([{"to": manager, "data": data}, "latest"]))?);
    // (currency0, currency1, hooks, ...): hooks is the third 32-byte word
    if out.len() < 2 + 64 * 3 {
        return Ok(String::new());
    }
    Ok(format!("0x{}", &out[2 + 64 * 2 + 24..2 + 64 * 3]))
}

pub fn fetch(hook: &str, n: usize) -> Result<()> {
    let rpc = Rpc::remote()?;
    let hook = hook.to_ascii_lowercase();
    let latest = num(&rpc.call("eth_blockNumber", json!([]))?)?;
    println!(
        "{}: finding the last {n} transactions that touched {hook} (latest block {latest})",
        chain().name
    );

    // 1. newest first, 10 blocks per call (free-tier limit). Where the chain allows it, the
    // hook's transactions are found through the pool manager's events for pools that use the
    // hook, so hooks that emit no events of their own are covered too.
    let lookup = chain().key_lookup;
    let source = match lookup {
        Some(m) => m.to_string(),
        None => hook.clone(),
    };
    let mut hook_of: BTreeMap<String, String> = BTreeMap::new();
    let mut hashes: Vec<String> = Vec::new();
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut to = latest;
    let mut scanned = 0u64;
    while hashes.len() < n && scanned < 200_000 {
        let from = to.saturating_sub(9);
        let logs = rpc.call(
            "eth_getLogs",
            json!([{"address": source, "fromBlock": hex(from), "toBlock": hex(to)}]),
        )?;
        let mut window: Vec<(u64, u64, String)> = Vec::new();
        for l in logs.as_array().into_iter().flatten() {
            if let Some(manager) = lookup {
                let pool = l["topics"].get(1).map(s).unwrap_or_default();
                if !hook_of.contains_key(&pool) {
                    let h = pool_hook(&rpc, manager, &pool)?;
                    hook_of.insert(pool.clone(), h);
                }
                if hook_of[&pool] != hook {
                    continue;
                }
            }
            window.push((
                num(&l["blockNumber"])?,
                num(&l["transactionIndex"])?,
                s(&l["transactionHash"]),
            ));
        }
        window.sort();
        window.reverse();
        for (_, _, h) in window {
            if seen.insert(h.clone()) {
                hashes.push(h);
            }
        }
        scanned += 10;
        to = from.saturating_sub(1);
        if scanned % 1000 == 0 {
            println!("  scanned {scanned} blocks, found {}", hashes.len());
        }
    }
    hashes.truncate(n);
    hashes.reverse();
    println!(
        "found {}; downloading them and the related earlier transactions in their blocks",
        hashes.len()
    );

    // 2. each transaction, its block, and the earlier transactions in the block that are
    // replayed first: all of them, or only those that touched the same things
    let all = all_earlier();
    if all {
        println!("replaying every earlier transaction in each block (exact block order)");
    }
    let mut blocks: BTreeMap<u64, (Value, Value)> = BTreeMap::new();
    let mut out = Vec::new();
    for (i, h) in hashes.iter().enumerate() {
        let tx = rpc.call("eth_getTransactionByHash", json!([h]))?;
        let receipt = rpc.call("eth_getTransactionReceipt", json!([h]))?;
        let block = num(&tx["blockNumber"])?;
        let index = num(&tx["transactionIndex"])?;
        if !blocks.contains_key(&block) {
            // with all = true the block comes with its full transactions
            let header = rpc.call("eth_getBlockByNumber", json!([hex(block), all]))?;
            let logs = if all {
                Value::Null
            } else {
                rpc.call(
                    "eth_getLogs",
                    json!([{"fromBlock": hex(block), "toBlock": hex(block)}]),
                )?
            };
            blocks.insert(block, (header, logs));
        }
        let (header, block_logs) = &blocks[&block];

        let mut related_txs = Vec::new();
        if all {
            for t in header["transactions"].as_array().into_iter().flatten() {
                if num(&t["transactionIndex"])? < index {
                    related_txs.push(t.clone());
                }
            }
        } else {
            let want: BTreeSet<String> = receipt["logs"]
                .as_array()
                .into_iter()
                .flatten()
                .flat_map(touch)
                .collect();
            let mut related: BTreeMap<u64, String> = BTreeMap::new();
            for l in block_logs.as_array().into_iter().flatten() {
                let li = num(&l["transactionIndex"])?;
                if li < index && touch(l).iter().any(|k| want.contains(k)) {
                    related.insert(li, s(&l["transactionHash"]));
                }
            }
            for rh in related.values() {
                related_txs.push(rpc.call("eth_getTransactionByHash", json!([rh]))?);
            }
        }

        println!(
            "  {}/{} {} block {block} index {index}: {} related earlier",
            i + 1,
            hashes.len(),
            &h[..12],
            related_txs.len()
        );
        out.push(json!({
            "hash": h,
            "hook": hook,
            "block": block,
            "index": index,
            "timestamp": num(&header["timestamp"])?,
            "base_fee": header["baseFeePerGas"].clone(),
            "tx": tx,
            "receipt": receipt,
            "related": related_txs,
        }));
    }
    fs::create_dir_all("cache")?;
    fs::write(CACHE, serde_json::to_string(&out)?)?;
    println!("saved {} transactions to {CACHE}", out.len());
    Ok(())
}
