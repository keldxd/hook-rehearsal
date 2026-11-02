use crate::anvil::Anvil;
use crate::fetch::CACHE;
use crate::report::{self, Tokens};
use crate::rpc::Rpc;
use crate::util::{num, s, short, topic_addr, word, TRANSFER};
use anyhow::{bail, Context, Result};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::fs;

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Log {
    pub address: String,
    pub topics: Vec<String>,
    pub data: String,
}

/// What one transaction did.
pub struct Outcome {
    pub ok: bool,
    pub gas: u64,
    pub logs: Vec<Log>,
    /// (token, account) -> net ERC20 amount received (negative = sent)
    pub flows: BTreeMap<(String, String), i128>,
}

impl Outcome {
    pub fn from_receipt(r: &Value) -> Result<Outcome> {
        let ok = num(&r["status"])? == 1;
        let gas = num(&r["gasUsed"])?;
        let logs: Vec<Log> = r["logs"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|l| Log {
                address: s(&l["address"]),
                topics: l["topics"].as_array().map(|a| a.iter().map(s).collect()).unwrap_or_default(),
                data: s(&l["data"]),
            })
            .collect();
        let mut flows: BTreeMap<(String, String), i128> = BTreeMap::new();
        for l in &logs {
            if l.topics.len() == 3 && l.topics[0] == TRANSFER {
                if let Some(v) = word(&l.data) {
                    *flows.entry((l.address.clone(), topic_addr(&l.topics[1]))).or_insert(0) -= v;
                    *flows.entry((l.address.clone(), topic_addr(&l.topics[2]))).or_insert(0) += v;
                }
            }
        }
        flows.retain(|_, v| *v != 0);
        Ok(Outcome { ok, gas, logs, flows })
    }

    /// Same status and identical events as the other outcome.
    pub fn same_as(&self, other: &Outcome) -> bool {
        self.ok == other.ok && self.logs == other.logs
    }
}

pub fn load() -> Result<Vec<Value>> {
    let raw = fs::read_to_string(CACHE).context("cache/txs.json not found, run fetch first")?;
    Ok(serde_json::from_str(&raw)?)
}

fn port() -> u16 {
    std::env::var("ANVIL_PORT").ok().and_then(|p| p.parse().ok()).unwrap_or(8546)
}

/// Queues a real transaction on the fork as its real sender, with its real fees and nonce.
fn send(a: &Rpc, t: &Value) -> Result<String> {
    a.call("anvil_impersonateAccount", json!([t["from"]]))?;
    let mut o = json!({
        "from": t["from"], "to": t["to"], "data": t["input"],
        "value": t["value"], "gas": t["gas"], "nonce": t["nonce"],
    });
    if !t["maxFeePerGas"].is_null() {
        o["maxFeePerGas"] = t["maxFeePerGas"].clone();
        o["maxPriorityFeePerGas"] = t["maxPriorityFeePerGas"].clone();
    } else {
        o["gasPrice"] = t["gasPrice"].clone();
    }
    Ok(s(&a.call("eth_sendTransaction", json!([o]))?))
}

/// Mines the related earlier transactions and then the target in one block with the real
/// base fee and timestamp, optionally with different code at the hook address.
pub fn run_block(anvil: &Anvil, e: &Value, code: Option<&str>) -> Result<Outcome> {
    let a = &anvil.rpc;
    if let Some(c) = code {
        a.call("anvil_setCode", json!([e["hook"], c]))?;
    }
    a.call("anvil_setNextBlockBaseFeePerGas", json!([e["base_fee"]]))?;
    for t in e["related"].as_array().into_iter().flatten() {
        let _ = send(a, t);
    }
    let h = send(a, &e["tx"])?;
    a.call("evm_setNextBlockTimestamp", json!([e["timestamp"]]))?;
    a.call("evm_mine", json!([]))?;
    let r = a.call("eth_getTransactionReceipt", json!([h]))?;
    if r.is_null() {
        bail!("the transaction was not mined");
    }
    Outcome::from_receipt(&r)
}

/// `replay`: the deployed code on rebuilt state, compared with what mainnet recorded.
pub fn fidelity() -> Result<()> {
    let remote = Rpc::remote()?;
    let entries = load()?;
    println!("replaying {} transactions on the deployed code", entries.len());
    let (mut same, mut same_gas, mut failed) = (0usize, 0usize, 0usize);
    let mut diffs: Vec<String> = Vec::new();
    for (i, e) in entries.iter().enumerate() {
        let hash = s(&e["hash"]);
        let block = num(&e["block"])?;
        let result = Anvil::fork(remote.url(), block - 1, port()).and_then(|a| run_block(&a, e, None));
        match result {
            Ok(out) => {
                let real = Outcome::from_receipt(&e["receipt"])?;
                let ok = out.same_as(&real);
                same += ok as usize;
                same_gas += (out.gas == real.gas) as usize;
                println!(
                    "  {}/{} {}: {}",
                    i + 1,
                    entries.len(),
                    &hash[..12],
                    if ok { "identical" } else { "DIFFERENT" }
                );
                if !ok {
                    diffs.push(format!("{hash}: {}", report::first_log_difference(&real, &out)));
                }
            }
            Err(err) => {
                failed += 1;
                println!("  {}/{} {}: could not replay: {err}", i + 1, entries.len(), &hash[..12]);
            }
        }
    }
    let n = entries.len();
    println!("\n==== replay fidelity (deployed code vs what mainnet recorded) ====");
    println!("transactions:                     {n}");
    println!("could not replay:                 {failed}");
    println!("identical result and events:      {same} / {}", n - failed);
    println!("identical gas used:               {same_gas} / {}", n - failed);
    if !diffs.is_empty() {
        println!("\nfirst differences:");
        for d in diffs.iter().take(10) {
            println!("  {d}");
        }
    }
    Ok(())
}

/// Builds the runtime code of the new hook by running its constructor at the hook's own
/// address on a fork, so immutables and the hook address permission check are real.
fn build_runtime(remote: &Rpc, e: &Value, artifact: &str, ctor: &str) -> Result<String> {
    let art: Value = serde_json::from_str(
        &fs::read_to_string(artifact).with_context(|| format!("cannot read {artifact}"))?,
    )?;
    let init = art["bytecode"]["object"]
        .as_str()
        .context("no bytecode.object in the artifact (use Foundry's out/<File>.sol/<Contract>.json)")?;
    if init.contains("__") {
        bail!("the artifact needs library linking, which v0.1 does not support");
    }
    let init = format!("0x{}{}", init.trim_start_matches("0x"), ctor.trim_start_matches("0x"));
    let anvil = Anvil::fork(remote.url(), num(&e["block"])? - 1, port())?;
    let hook = &e["hook"];
    let deployed = s(&anvil.rpc.call("eth_getCode", json!([hook, "latest"]))?);
    anvil.rpc.call("anvil_setCode", json!([hook, init]))?;
    let runtime = s(&anvil.rpc.call("eth_call", json!([{"to": hook, "data": "0x"}, "latest"]))?);
    if runtime.len() <= 2 {
        bail!("the constructor returned no code: check the constructor arguments");
    }
    println!(
        "new code: {} bytes (deployed: {} bytes, {})",
        (runtime.len() - 2) / 2,
        (deployed.len() - 2) / 2,
        if runtime == deployed { "identical bytes" } else { "different bytes" }
    );
    Ok(runtime)
}

/// `rehearse`: every transaction on the deployed code and on the new code, on identical state.
pub fn rehearse(artifact: &str, ctor: &str) -> Result<()> {
    let remote = Rpc::remote()?;
    let entries = load()?;
    let first = entries.first().context("no transactions in the cache")?;
    println!("rehearsing an upgrade of {}", s(&first["hook"]));
    let runtime = build_runtime(&remote, first, artifact, ctor)?;
    let mut tokens = Tokens::new(Rpc::remote()?);
    println!("replaying {} real transactions on the deployed code and on the new code\n", entries.len());

    let mut summary = report::Summary::default();
    for (i, e) in entries.iter().enumerate() {
        let hash = s(&e["hash"]);
        let user = s(&e["tx"]["from"]);
        let block = num(&e["block"])?;
        let step = (|| -> Result<(Outcome, Outcome)> {
            let anvil = Anvil::fork(remote.url(), block - 1, port())?;
            let snap = anvil.rpc.call("evm_snapshot", json!([]))?;
            let old = run_block(&anvil, e, None)?;
            anvil.rpc.call("evm_revert", json!([snap]))?;
            let new = run_block(&anvil, e, Some(&runtime))?;
            Ok((old, new))
        })();
        match step {
            Ok((old, new)) => {
                let real = Outcome::from_receipt(&e["receipt"])?;
                let verified = old.same_as(&real);
                let findings = report::differences(&user, &old, &new, &mut tokens);
                println!(
                    "  {}/{} {}: {}",
                    i + 1,
                    entries.len(),
                    &hash[..12],
                    if findings.is_empty() { "same behaviour".to_string() } else { format!("{} finding(s)", findings.len()) }
                );
                summary.add(&hash, &short(&user), verified, findings);
            }
            Err(err) => {
                println!("  {}/{} {}: could not replay: {err}", i + 1, entries.len(), &hash[..12]);
                summary.failed += 1;
            }
        }
    }
    summary.print(entries.len());
    Ok(())
}
