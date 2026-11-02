use crate::rpc::Rpc;
use crate::util::{hex, num, s, topic_addr, PM, TRANSFER, ZERO};
use anyhow::Result;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;

pub const CACHE: &str = "cache/txs.json";

/// What a log touches: the v4 pool for PoolManager logs, the two accounts for token
/// transfers (not the token itself, everyone uses WETH), otherwise the emitting contract.
pub fn touch(log: &Value) -> Vec<String> {
    let addr = s(&log["address"]);
    let topics: Vec<String> = log["topics"]
        .as_array()
        .map(|a| a.iter().map(s).collect())
        .unwrap_or_default();
    if addr == PM {
        vec![format!("pool:{}", topics.get(1).cloned().unwrap_or_default())]
    } else if topics.first().map(String::as_str) == Some(TRANSFER) && topics.len() >= 3 {
        topics[1..3]
            .iter()
            .map(|t| topic_addr(t))
            .filter(|a| a != PM && a != ZERO)
            .collect()
    } else {
        vec![addr]
    }
}

pub fn fetch(hook: &str, n: usize) -> Result<()> {
    let rpc = Rpc::remote()?;
    let hook = hook.to_ascii_lowercase();
    let latest = num(&rpc.call("eth_blockNumber", json!([]))?)?;
    println!("finding the last {n} transactions that touched {hook} (latest block {latest})");

    // 1. newest first, 10 blocks per call (free-tier limit)
    let mut hashes: Vec<String> = Vec::new();
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut to = latest;
    let mut scanned = 0u64;
    while hashes.len() < n && scanned < 200_000 {
        let from = to.saturating_sub(9);
        let logs = rpc.call(
            "eth_getLogs",
            json!([{"address": hook, "fromBlock": hex(from), "toBlock": hex(to)}]),
        )?;
        let mut window: Vec<(u64, u64, String)> = Vec::new();
        for l in logs.as_array().into_iter().flatten() {
            window.push((num(&l["blockNumber"])?, num(&l["transactionIndex"])?, s(&l["transactionHash"])));
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
    println!("found {}; downloading them and the related earlier transactions in their blocks", hashes.len());

    // 2. each transaction, its block, and the earlier transactions that touched the same things
    let mut blocks: BTreeMap<u64, (Value, Value)> = BTreeMap::new();
    let mut out = Vec::new();
    for (i, h) in hashes.iter().enumerate() {
        let tx = rpc.call("eth_getTransactionByHash", json!([h]))?;
        let receipt = rpc.call("eth_getTransactionReceipt", json!([h]))?;
        let block = num(&tx["blockNumber"])?;
        let index = num(&tx["transactionIndex"])?;
        if !blocks.contains_key(&block) {
            let header = rpc.call("eth_getBlockByNumber", json!([hex(block), false]))?;
            let logs = rpc.call("eth_getLogs", json!([{"fromBlock": hex(block), "toBlock": hex(block)}]))?;
            blocks.insert(block, (header, logs));
        }
        let (header, block_logs) = &blocks[&block];

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
        let mut related_txs = Vec::new();
        for rh in related.values() {
            related_txs.push(rpc.call("eth_getTransactionByHash", json!([rh]))?);
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
