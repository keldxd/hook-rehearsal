use anyhow::{Result, anyhow, bail};
use serde_json::{Value, json};
use std::thread::sleep;
use std::time::Duration;

pub struct Rpc {
    url: String,
    client: reqwest::blocking::Client,
    tries: u64,
}

impl Rpc {
    /// `tries` attempts per call, each waiting up to `timeout_secs` for an answer.
    pub fn new(url: &str, tries: u64, timeout_secs: u64) -> Result<Rpc> {
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(timeout_secs))
            .build()?;
        Ok(Rpc {
            url: url.to_string(),
            client,
            tries,
        })
    }

    /// The archive RPC for the chosen chain (BASE_RPC_URL or BSC_RPC_URL).
    pub fn remote() -> Result<Rpc> {
        let c = crate::util::chain();
        let url = std::env::var(c.rpc_env).map_err(|_| {
            anyhow!(
                "set {} to an archive RPC endpoint for {}",
                c.rpc_env,
                c.name
            )
        })?;
        Rpc::new(&url, 6, 180)
    }

    pub fn url(&self) -> &str {
        &self.url
    }

    /// One JSON-RPC call, retried on rate limits and network errors.
    pub fn call(&self, method: &str, params: Value) -> Result<Value> {
        let body = json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params});
        let mut last = anyhow!("{method}: no attempt made");
        for attempt in 0..self.tries {
            if attempt > 0 {
                sleep(Duration::from_millis(400 * attempt));
            }
            let resp = match self.client.post(&self.url).json(&body).send() {
                Ok(r) => r,
                Err(e) => {
                    last = anyhow!("{method}: {}", e.without_url());
                    continue;
                }
            };
            if resp.status().as_u16() == 429 {
                last = anyhow!("{method}: rate limited");
                continue;
            }
            let v: Value = match resp.json() {
                Ok(v) => v,
                Err(e) => {
                    last = anyhow!("{method}: bad response: {}", e.without_url());
                    continue;
                }
            };
            if let Some(e) = v.get("error") {
                let msg = e
                    .get("message")
                    .and_then(|m| m.as_str())
                    .unwrap_or("")
                    .to_string();
                let lower = msg.to_ascii_lowercase();
                if lower.contains("rate")
                    || lower.contains("exceeded")
                    || lower.contains("capacity")
                {
                    last = anyhow!("{method}: {msg}");
                    continue;
                }
                bail!("{method}: {msg}");
            }
            return Ok(v.get("result").cloned().unwrap_or(Value::Null));
        }
        Err(last)
    }
}
