mod anvil;
mod fetch;
mod replay;
mod report;
mod rpc;
mod util;

use anyhow::{Context, Result, bail};

const USAGE: &str = "usage:
  hook-rehearsal fetch <hook address> [count]
  hook-rehearsal replay
  hook-rehearsal rehearse <constructor args hex | 0x> <artifact.json> [more artifacts...]";

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("fetch") => {
            let hook = args.get(2).context(USAGE)?;
            let n = match args.get(3) {
                Some(v) => v.parse().context("count must be a number")?,
                None => 100,
            };
            fetch::fetch(hook, n)
        }
        Some("replay") => replay::fidelity(),
        Some("rehearse") => {
            let ctor = args.get(2).context(USAGE)?;
            if args.len() < 4 {
                bail!(USAGE);
            }
            replay::rehearse(ctor, &args[3..])
        }
        _ => bail!(USAGE),
    }
}
