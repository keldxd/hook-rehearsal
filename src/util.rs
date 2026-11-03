use anyhow::{Result, anyhow};
use serde_json::Value;
use std::sync::OnceLock;

/// The chain and DEX being rehearsed, chosen with the CHAIN environment variable.
pub struct Chain {
    pub name: &'static str,
    /// Environment variable that holds the archive RPC URL
    pub rpc_env: &'static str,
    /// Pool managers: their first indexed topic is the pool id
    pub managers: &'static [(&'static str, &'static str)],
    /// Contracts that hold everyone's tokens (pool managers, vaults): ignored when
    /// deciding which transactions are related
    pub hubs: &'static [(&'static str, &'static str)],
    /// Pool manager with poolIdToPoolKey(bytes32), used to find a hook's transactions
    /// through its pools' events when the hook emits no events of its own
    pub key_lookup: Option<&'static str>,
    /// Replay every earlier transaction in the block before the target (exact, and cheap on
    /// chains with small blocks), instead of only the ones that touched the same things
    pub all_earlier: bool,
}

static BASE: Chain = Chain {
    name: "Base (Uniswap v4)",
    rpc_env: "BASE_RPC_URL",
    managers: &[("0x498581ff718922c3f8e6a244956af099b2652b2b", "PoolManager")],
    hubs: &[("0x498581ff718922c3f8e6a244956af099b2652b2b", "PoolManager")],
    key_lookup: None,
    all_earlier: false,
};

static BSC: Chain = Chain {
    name: "BNB Chain (PancakeSwap Infinity)",
    rpc_env: "BSC_RPC_URL",
    managers: &[
        (
            "0xa0ffb9c1ce1fe56963b0321b32e7a0302114058b",
            "CLPoolManager",
        ),
        (
            "0xc697d2898e0d09264376196696c51d7abbbaa4a9",
            "BinPoolManager",
        ),
    ],
    hubs: &[
        (
            "0xa0ffb9c1ce1fe56963b0321b32e7a0302114058b",
            "CLPoolManager",
        ),
        (
            "0xc697d2898e0d09264376196696c51d7abbbaa4a9",
            "BinPoolManager",
        ),
        ("0x238a358808379702088667322f80ac48bad5e6c4", "Vault"),
    ],
    key_lookup: Some("0xa0ffb9c1ce1fe56963b0321b32e7a0302114058b"),
    all_earlier: true,
};

/// CHAIN=base (default) or CHAIN=bsc.
pub fn chain() -> &'static Chain {
    static C: OnceLock<&'static Chain> = OnceLock::new();
    C.get_or_init(|| {
        match std::env::var("CHAIN")
            .unwrap_or_default()
            .to_ascii_lowercase()
            .as_str()
        {
            "bsc" | "bnb" => &BSC,
            _ => &BASE,
        }
    })
}

/// RELATED=all or RELATED=touched overrides the chain's default.
pub fn all_earlier() -> bool {
    match std::env::var("RELATED").unwrap_or_default().as_str() {
        "all" => true,
        "touched" => false,
        _ => chain().all_earlier,
    }
}

pub fn is_manager(a: &str) -> bool {
    chain().managers.iter().any(|(m, _)| *m == a)
}

pub fn is_hub(a: &str) -> bool {
    chain().hubs.iter().any(|(m, _)| *m == a)
}
/// keccak256("Transfer(address,address,uint256)")
pub const TRANSFER: &str = "0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef";
pub const ZERO: &str = "0x0000000000000000000000000000000000000000";

/// A JSON string, lowercased ("" if missing).
pub fn s(v: &Value) -> String {
    v.as_str().unwrap_or("").to_ascii_lowercase()
}

/// A number given as JSON number or "0x.." hex string.
pub fn num(v: &Value) -> Result<u64> {
    if let Some(n) = v.as_u64() {
        return Ok(n);
    }
    let t = v.as_str().ok_or_else(|| anyhow!("not a number: {v}"))?;
    u64::from_str_radix(t.trim_start_matches("0x"), 16)
        .map_err(|e| anyhow!("bad hex number {t}: {e}"))
}

pub fn hex(n: u64) -> String {
    format!("0x{n:x}")
}

/// An unsigned 256-bit word (hex) as i128; None if it does not fit.
pub fn word(w: &str) -> Option<i128> {
    let t = w.trim_start_matches("0x").trim_start_matches('0');
    if t.is_empty() {
        return Some(0);
    }
    if t.len() > 32 {
        return None;
    }
    let v = u128::from_str_radix(t, 16).ok()?;
    i128::try_from(v).ok()
}

/// The address inside a 32-byte topic.
pub fn topic_addr(t: &str) -> String {
    let t = t.to_ascii_lowercase();
    format!("0x{}", &t[t.len().saturating_sub(40)..])
}

/// 0x4200..0006 style short address.
pub fn short(a: &str) -> String {
    if a.len() > 14 {
        format!("{}..{}", &a[..6], &a[a.len() - 4..])
    } else {
        a.to_string()
    }
}

/// A raw token amount with its decimals: 8 decimals for amounts of 1 or more,
/// otherwise 6 significant digits (0.000000203 rather than 0.00000000).
pub fn amount(v: i128, decimals: u32) -> String {
    let sign = if v < 0 { "-" } else { "" };
    let a = v.unsigned_abs();
    let decimals = decimals.min(38);
    if decimals == 0 {
        return format!("{sign}{a}");
    }
    let base = 10u128.pow(decimals);
    let whole = a / base;
    let frac = format!("{:0width$}", a % base, width = decimals as usize);
    let keep = if whole > 0 {
        8
    } else {
        frac.find(|c: char| c != '0').map(|i| i + 6).unwrap_or(0)
    };
    let frac: String = frac.chars().take(keep.min(decimals as usize)).collect();
    let frac = frac.trim_end_matches('0');
    if frac.is_empty() {
        format!("{sign}{whole}")
    } else {
        format!("{sign}{whole}.{frac}")
    }
}

/// A percentage with enough decimals to show small changes.
pub fn pct(x: f64) -> String {
    if x.abs() >= 0.01 {
        format!("{x:+.3}%")
    } else {
        format!("{x:+.6}%")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn amounts() {
        assert_eq!(amount(113_798_858, 6), "113.798858");
        assert_eq!(amount(203_000_000_000, 18), "0.000000203");
        assert_eq!(amount(-1_500_000_000_000_000_000, 18), "-1.5");
        assert_eq!(amount(0, 18), "0");
    }

    #[test]
    fn words() {
        assert_eq!(
            word("0x0000000000000000000000000000000000000000000000000000000006c86eca"),
            Some(113_798_858)
        );
        assert_eq!(
            word("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"),
            None
        );
    }
}
