mod anvil;
mod fetch;
mod replay;
mod report;
mod rpc;
mod util;

use anyhow::{bail, Context, Result};

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("fetch") => {
            let hook = args.get(2).context("usage: fetch <hook address> [count]")?;
            let n = match args.get(3) {
                Some(v) => v.parse().context("count must be a number")?,
                None => 100,
            };
            fetch::fetch(hook, n)
        }
        Some("replay") => replay::fidelity(),
        Some("rehearse") => {
            let artifact = args
                .get(2)
                .context("usage: rehearse <foundry artifact .json> [constructor args as hex]")?;
            let ctor = args.get(3).map(String::as_str).unwrap_or("0x");
            replay::rehearse(artifact, ctor)
        }
        _ => bail!(
            "usage: hook-rehearsal fetch <hook> [count] | replay | rehearse <artifact.json> [constructor args hex]"
        ),
    }
}
