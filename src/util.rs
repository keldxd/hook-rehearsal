use anyhow::{anyhow, Result};
use serde_json::Value;

/// Uniswap v4 PoolManager on Base
pub const PM: &str = "0x498581ff718922c3f8e6a244956af099b2652b2b";
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
    u64::from_str_radix(t.trim_start_matches("0x"), 16).map_err(|e| anyhow!("bad hex number {t}: {e}"))
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

/// A raw token amount with its decimals, e.g. 113798858 with 6 -> "113.798858".
pub fn amount(v: i128, decimals: u32) -> String {
    let sign = if v < 0 { "-" } else { "" };
    let a = v.unsigned_abs();
    if decimals == 0 {
        return format!("{sign}{a}");
    }
    let base = 10u128.pow(decimals.min(38));
    let whole = a / base;
    let frac = format!("{:0width$}", a % base, width = decimals as usize);
    let frac = frac.trim_end_matches('0');
    let frac: String = frac.chars().take(8).collect();
    if frac.is_empty() {
        format!("{sign}{whole}")
    } else {
        format!("{sign}{whole}.{frac}")
    }
}
