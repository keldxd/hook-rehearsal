use crate::rpc::Rpc;
use anyhow::{Context, Result, bail};
use serde_json::json;
use std::process::{Child, Command, Stdio};
use std::thread::sleep;
use std::time::Duration;

/// A local anvil fork, stopped when dropped.
pub struct Anvil {
    child: Child,
    pub rpc: Rpc,
}

impl Anvil {
    /// Forks `fork_url` at `block` (the state at the end of that block). Transactions are mined
    /// only on evm_mine, in the order they were sent.
    pub fn fork(fork_url: &str, block: u64, port: u16) -> Result<Anvil> {
        let child = Command::new("anvil")
            .args([
                "--fork-url",
                fork_url,
                "--fork-block-number",
                &block.to_string(),
                "--port",
                &port.to_string(),
                "--order",
                "fifo",
                "--no-mining",
                "--silent",
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .context("could not start anvil (is Foundry installed?)")?;
        let rpc = Rpc::new(&format!("http://127.0.0.1:{port}"), 1, 900)?;
        let mut anvil = Anvil { child, rpc };
        for _ in 0..300 {
            if anvil.rpc.call("eth_blockNumber", json!([])).is_ok() {
                return Ok(anvil);
            }
            if let Ok(Some(status)) = anvil.child.try_wait() {
                bail!("anvil exited early ({status}); is port {port} already in use?");
            }
            sleep(Duration::from_millis(200));
        }
        bail!("anvil did not start within 60 seconds")
    }
}

impl Drop for Anvil {
    /// Stops anvil with an interrupt first, so it can save the state it downloaded to its
    /// fork cache (reruns on the same blocks are then much faster); kills it if it hangs.
    fn drop(&mut self) {
        let _ = Command::new("kill")
            .args(["-INT", &self.child.id().to_string()])
            .status();
        for _ in 0..100 {
            if let Ok(Some(_)) = self.child.try_wait() {
                return;
            }
            sleep(Duration::from_millis(100));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
