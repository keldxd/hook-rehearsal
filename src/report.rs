use crate::replay::Outcome;
use crate::rpc::Rpc;
use crate::util::{amount, short, word};
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};

pub struct Finding {
    pub level: &'static str,
    pub kind: &'static str,
    pub text: String,
}

fn finding(level: &'static str, kind: &'static str, text: String) -> Finding {
    Finding { level, kind, text }
}

/// Token symbols and decimals, looked up once.
pub struct Tokens {
    rpc: Rpc,
    cache: BTreeMap<String, (String, u32)>,
}

impl Tokens {
    pub fn new(rpc: Rpc) -> Tokens {
        Tokens { rpc, cache: BTreeMap::new() }
    }

    pub fn info(&mut self, token: &str) -> (String, u32) {
        if let Some(i) = self.cache.get(token) {
            return i.clone();
        }
        let symbol = self
            .rpc
            .call("eth_call", json!([{"to": token, "data": "0x95d89b41"}, "latest"]))
            .ok()
            .and_then(|v| v.as_str().map(decode_string))
            .filter(|t| !t.is_empty())
            .unwrap_or_else(|| short(token));
        let decimals = self
            .rpc
            .call("eth_call", json!([{"to": token, "data": "0x313ce567"}, "latest"]))
            .ok()
            .and_then(|v| v.as_str().and_then(word))
            .map(|d| d.clamp(0, 36) as u32)
            .unwrap_or(18);
        self.cache.insert(token.to_string(), (symbol.clone(), decimals));
        (symbol, decimals)
    }
}

/// ABI-encoded string (or bytes32) returned by symbol().
fn decode_string(hexdata: &str) -> String {
    let h = hexdata.trim_start_matches("0x");
    let bytes: Vec<u8> = (0..h.len() / 2)
        .filter_map(|i| u8::from_str_radix(&h[i * 2..i * 2 + 2], 16).ok())
        .collect();
    let raw: Vec<u8> = if bytes.len() >= 96 {
        let len = word(&h[64..128]).unwrap_or(0).clamp(0, 64) as usize;
        bytes.get(64..64 + len).map(|b| b.to_vec()).unwrap_or_default()
    } else {
        bytes.into_iter().take_while(|b| *b != 0).collect()
    };
    String::from_utf8_lossy(&raw).chars().filter(|c| !c.is_control()).take(16).collect()
}

/// Where two outcomes' events first differ, as text.
pub fn first_log_difference(a: &Outcome, b: &Outcome) -> String {
    if a.ok != b.ok {
        return format!("status {} vs {}", a.ok, b.ok);
    }
    let n = a.logs.len().max(b.logs.len());
    match (0..n).find(|i| a.logs.get(*i) != b.logs.get(*i)) {
        Some(i) => format!(
            "{} vs {} events, first difference at #{i} (from {})",
            a.logs.len(),
            b.logs.len(),
            a.logs.get(i).or(b.logs.get(i)).map(|l| short(&l.address)).unwrap_or_default()
        ),
        None => "same events".to_string(),
    }
}

/// Everything that differs between the deployed code's run (old) and the new code's run.
pub fn differences(user: &str, old: &Outcome, new: &Outcome, tokens: &mut Tokens) -> Vec<Finding> {
    let mut out = Vec::new();
    match (old.ok, new.ok) {
        (true, false) => {
            out.push(finding("REVIEW", "swap-now-fails", "worked on the deployed code, reverts on the new code".into()));
            return out;
        }
        (false, true) => {
            out.push(finding("REVIEW", "swap-now-works", "reverted on the deployed code, works on the new code".into()));
            return out;
        }
        (false, false) => return out,
        (true, true) => {}
    }

    // 1. Value: who receives or pays a different amount of each token
    let keys: BTreeSet<&(String, String)> = old.flows.keys().chain(new.flows.keys()).collect();
    let mut change: BTreeMap<String, Vec<(String, i128)>> = BTreeMap::new();
    for k in keys {
        let d = new.flows.get(k).copied().unwrap_or(0) - old.flows.get(k).copied().unwrap_or(0);
        if d != 0 {
            change.entry(k.0.clone()).or_default().push((k.1.clone(), d));
        }
    }
    for (token, deltas) in &change {
        let (sym, dec) = tokens.info(token);
        let user_d: i128 = deltas.iter().filter(|(a, _)| a == user).map(|(_, d)| *d).sum();
        let mut gainers: Vec<&(String, i128)> = deltas.iter().filter(|(a, d)| a != user && *d > 0).collect();
        gainers.sort_by(|x, y| y.1.cmp(&x.1));
        let gained: Vec<String> = gainers
            .iter()
            .take(3)
            .map(|(a, d)| format!("{} +{}", short(a), amount(*d, dec)))
            .collect();
        let base = old.flows.get(&(token.clone(), user.to_string())).copied().unwrap_or(0);
        let pct = if base != 0 {
            format!(" ({:+.3}%)", user_d as f64 * 100.0 / base.unsigned_abs() as f64)
        } else {
            String::new()
        };
        if user_d < 0 {
            out.push(finding(
                "DANGER",
                "swapper-loses",
                format!("swapper ends with {} {sym} less{pct}; gained by: {}", amount(-user_d, dec), gained.join(", ")),
            ));
        } else if user_d > 0 {
            out.push(finding(
                "REVIEW",
                "swapper-gains",
                format!("swapper ends with {} {sym} more{pct}", amount(user_d, dec)),
            ));
        } else {
            out.push(finding(
                "REVIEW",
                "flows-changed",
                format!("{sym} moves differently between other accounts: {}", gained.join(", ")),
            ));
        }
    }

    // 2. Events (beyond the amounts above)
    if old.logs != new.logs {
        out.push(finding("REVIEW", "events-changed", first_log_difference(old, new)));
    }

    // 3. Gas
    let (og, ng) = (old.gas as f64, new.gas as f64);
    if og > 0.0 && ((ng - og) / og).abs() > 0.05 {
        out.push(finding(
            "REVIEW",
            "gas-changed",
            format!("gas {} -> {} ({:+.1}%)", old.gas, new.gas, (ng - og) * 100.0 / og),
        ));
    }
    out
}

#[derive(Default)]
pub struct Summary {
    pub failed: usize,
    same: usize,
    changed: usize,
    changed_verified: usize,
    by_kind: BTreeMap<(&'static str, &'static str), usize>,
    details: Vec<String>,
    dangers: Vec<String>,
}

impl Summary {
    pub fn add(&mut self, hash: &str, user: &str, verified: bool, findings: Vec<Finding>) {
        if findings.is_empty() {
            self.same += 1;
            return;
        }
        self.changed += 1;
        self.changed_verified += verified as usize;
        let kinds: BTreeSet<(&'static str, &'static str)> = findings.iter().map(|f| (f.level, f.kind)).collect();
        for k in kinds {
            *self.by_kind.entry(k).or_insert(0) += 1;
        }
        let baseline = if verified {
            "replay of the deployed code matches mainnet exactly"
        } else {
            "replay of the deployed code differs from mainnet, lower confidence"
        };
        let mut block = format!("{}  swapper {user}\n  ({baseline})", &hash[..18]);
        for f in &findings {
            block.push_str(&format!("\n  {:<6} {:<15} {}", f.level, f.kind, f.text));
            if f.level == "DANGER" {
                self.dangers.push(format!("{}  {}", &hash[..18], f.text));
            }
        }
        self.details.push(block);
    }

    pub fn print(&self, n: usize) {
        println!("\n==== hook upgrade rehearsal ====");
        println!("real transactions replayed:     {n}");
        println!("could not replay:               {}", self.failed);
        println!("same behaviour:                 {}", self.same);
        println!(
            "different behaviour:            {}  ({} of them on transactions whose replay matches mainnet exactly)",
            self.changed, self.changed_verified
        );
        if !self.by_kind.is_empty() {
            println!("\nfindings (transactions affected):");
            for ((level, kind), count) in &self.by_kind {
                println!("  {level:<6} {kind:<15} {count}");
            }
        }
        if !self.dangers.is_empty() {
            println!("\nDANGER: swappers who would lose value with the new code:");
            for d in self.dangers.iter().take(15) {
                println!("  {d}");
            }
        }
        if !self.details.is_empty() {
            println!("\nfirst transactions that behave differently:");
            for d in self.details.iter().take(8) {
                println!("\n{d}");
            }
        }
    }
}
