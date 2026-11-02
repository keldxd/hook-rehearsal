use crate::replay::Outcome;
use crate::rpc::Rpc;
use crate::util::{amount, pct, short, word, PM};
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

/// Token symbols and decimals (looked up once) and names for known addresses.
pub struct Names {
    rpc: Rpc,
    tokens: BTreeMap<String, (String, u32)>,
    labels: BTreeMap<String, String>,
}

impl Names {
    pub fn new(rpc: Rpc) -> Names {
        let mut labels = BTreeMap::new();
        labels.insert(PM.to_string(), "PoolManager".to_string());
        Names { rpc, tokens: BTreeMap::new(), labels }
    }

    pub fn label(&mut self, addr: &str, name: &str) {
        self.labels.insert(addr.to_ascii_lowercase(), name.to_string());
    }

    pub fn name(&self, addr: &str) -> String {
        self.labels.get(addr).cloned().unwrap_or_else(|| short(addr))
    }

    pub fn token(&mut self, token: &str) -> (String, u32) {
        if let Some(i) = self.tokens.get(token) {
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
        self.tokens.insert(token.to_string(), (symbol.clone(), decimals));
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
pub fn differences(user: &str, old: &Outcome, new: &Outcome, names: &mut Names) -> Vec<Finding> {
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
        let (sym, dec) = names.token(token);
        let user_d: i128 = deltas.iter().filter(|(a, _)| a == user).map(|(_, d)| *d).sum();
        let mut others: Vec<&(String, i128)> = deltas.iter().filter(|(a, _)| a != user).collect();
        others.sort_by(|x, y| y.1.unsigned_abs().cmp(&x.1.unsigned_abs()));
        let gained: Vec<String> = others
            .iter()
            .filter(|(_, d)| *d > 0)
            .take(3)
            .map(|(a, d)| format!("{} +{}", names.name(a), amount(*d, dec)))
            .collect();
        let moved: Vec<String> = others
            .iter()
            .take(3)
            .map(|(a, d)| format!("{} {}{}", names.name(a), if *d > 0 { "+" } else { "" }, amount(*d, dec)))
            .collect();
        let base = old.flows.get(&(token.clone(), user.to_string())).copied().unwrap_or(0);
        let rel = if base != 0 {
            format!(" ({})", pct(user_d as f64 * 100.0 / base.unsigned_abs() as f64))
        } else {
            String::new()
        };
        if user_d < 0 {
            out.push(finding(
                "DANGER",
                "swapper-loses",
                format!("swapper ends with {} {sym} less{rel}; gained by: {}", amount(-user_d, dec), gained.join(", ")),
            ));
        } else if user_d > 0 {
            out.push(finding(
                "REVIEW",
                "swapper-gains",
                format!("swapper ends with {} {sym} more{rel}", amount(user_d, dec)),
            ));
        } else {
            out.push(finding(
                "REVIEW",
                "flows-changed",
                format!("{sym} moves differently between other accounts: {}", moved.join(", ")),
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
            format!("gas {} -> {} ({})", old.gas, new.gas, pct((ng - og) * 100.0 / og)),
        ));
    }
    out
}

/// The results of rehearsing one build against every transaction.
#[derive(Default)]
pub struct Summary {
    pub name: String,
    pub failed: usize,
    pub verified: usize,
    same: usize,
    changed: usize,
    changed_verified: usize,
    by_kind: BTreeMap<(&'static str, &'static str), usize>,
    details: Vec<String>,
    dangers: Vec<String>,
}

impl Summary {
    pub fn new(name: &str) -> Summary {
        Summary { name: name.to_string(), ..Default::default() }
    }

    pub fn add(&mut self, hash: &str, user: &str, verified: bool, findings: Vec<Finding>) {
        self.verified += verified as usize;
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

    /// The report as text, with at most `max_details` per-transaction blocks.
    pub fn render(&self, n: usize, max_details: usize) -> String {
        let mut o = String::new();
        o.push_str(&format!("==== hook upgrade rehearsal: {} ====\n", self.name));
        o.push_str(&format!("real transactions replayed:     {n}\n"));
        o.push_str(&format!("could not replay:               {}\n", self.failed));
        o.push_str(&format!("deployed code matches mainnet:  {} / {}\n", self.verified, n - self.failed));
        o.push_str(&format!("same behaviour:                 {}\n", self.same));
        o.push_str(&format!(
            "different behaviour:            {}  ({} of them on transactions whose replay matches mainnet exactly)\n",
            self.changed, self.changed_verified
        ));
        if !self.by_kind.is_empty() {
            o.push_str("\nfindings (transactions affected):\n");
            for ((level, kind), count) in &self.by_kind {
                o.push_str(&format!("  {level:<6} {kind:<15} {count}\n"));
            }
        }
        if !self.dangers.is_empty() {
            o.push_str("\nDANGER: swappers who would lose value with the new code:\n");
            for d in &self.dangers {
                o.push_str(&format!("  {d}\n"));
            }
        }
        if !self.details.is_empty() {
            o.push_str("\ntransactions that behave differently:\n");
            for d in self.details.iter().take(max_details) {
                o.push_str(&format!("\n{d}\n"));
            }
            if self.details.len() > max_details {
                o.push_str(&format!("\n... and {} more\n", self.details.len() - max_details));
            }
        }
        o
    }
}
