#!/usr/bin/env bash
set -euo pipefail

echo "🔧 Patching src/helius/client.rs (replace fetch_onchain_events) ..."
python3 - <<'PY'
from pathlib import Path
import re

path = Path("src/helius/client.rs")
s = path.read_text()

# Replace the entire fetch_onchain_events(...) function with a known-good version.
# This avoids brace-mismatch and the "for loop returns ()" issue.
pat = re.compile(r'pub\s+async\s+fn\s+fetch_onchain_events\s*\([\s\S]*?\n\}\n', re.MULTILINE)

replacement = r'''pub async fn fetch_onchain_events(
    cfg: &crate::config::Config,
    db: &mut crate::db::Db,
    coins: &mut std::collections::HashMap<String, crate::types::CoinState>,
    tracked: &[String],
) -> Vec<String> {
    use crate::helius::types::HeliusTx;
    use crate::helius::utils::{collect_mints, estimate_sol_outflow};
    use crate::types::Event;
    use std::collections::HashSet;

    if cfg.helius_api_key.trim().is_empty() {
        return vec![];
    }

    let base = if cfg.helius_api_base.trim().is_empty() {
        "https://api.helius.xyz".to_string()
    } else {
        cfg.helius_api_base.clone()
    };

    let client = reqwest::Client::new();
    let mut discovered: HashSet<String> = HashSet::new();

    for addr in tracked {
        let url = format!(
            "{}/v0/addresses/{}/transactions?api-key={}&limit={}",
            base,
            addr,
            cfg.helius_api_key,
            cfg.helius_tx_limit
        );

        let resp = match client.get(&url).send().await {
            Ok(r) => r,
            Err(_) => continue,
        };
        if !resp.status().is_success() {
            continue;
        }

        let txs: Vec<HeliusTx> = match resp.json().await {
            Ok(v) => v,
            Err(_) => continue,
        };

        // If we have a coin state for this address (sometimes you track mints, sometimes wallets),
        // we'll attach events to it. If not, we'll just record discovery.
        let maybe_state = coins.get_mut(addr);

        for tx in txs.iter() {
            let ts = tx.timestamp.unwrap_or(0) as i64;
            let actor = tx.fee_payer.clone().unwrap_or_else(|| addr.clone());
            let sig = tx.signature.as_deref();

            // Discover mints from token transfers
            for m in collect_mints(&tx.token_transfers) {
                discovered.insert(m.clone());

                // Record a wallet_edge observation (best-effort)
                // insert_wallet_edge(ts, src_wallet, dst_wallet, mint, action, sol, sig)
                let _ = db.insert_wallet_edge(
                    ts,
                    &actor,
                    None,
                    Some(m.as_str()),
                    Some("token_transfer"),
                    None,
                    sig,
                );

                // Also attach an event if we have state
                if let Some(state) = maybe_state.as_ref() {
                    // no-op; we can’t mut-borrow twice here
                    let _ = state;
                }
            }

            // Estimate SOL outflow from native transfers
            let sol_out = estimate_sol_outflow(&tx.native_transfers, &actor);
            if sol_out > 0.0 {
                let _ = db.insert_wallet_edge(
                    ts,
                    &actor,
                    None,
                    None,
                    Some("sol_outflow"),
                    Some(sol_out),
                    sig,
                );
            }

            // Attach a lightweight event if we do have a state for this tracked address
            if let Some(state) = coins.get_mut(addr) {
                state.events.push(Event {
                    ts: ts as u64,
                    wallet: actor.clone(),
                    mint: None,
                    action: "helius_tx".to_string(),
                    sol: Some(sol_out),
                    sig: sig.map(|x| x.to_string()),
                });
            }
        }
    }

    discovered.into_iter().collect::<Vec<String>>()
}
'''

m = pat.search(s)
if not m:
    raise SystemExit("❌ Could not find `pub async fn fetch_onchain_events(...)` in src/helius/client.rs")

s2 = pat.sub(replacement + "\n", s, count=1)
path.write_text(s2)
print("✅ Replaced fetch_onchain_events() in src/helius/client.rs")
PY


echo "🔧 Patching src/config.rs (force helius_rpc_url in Config initializer) ..."
python3 - <<'PY'
from pathlib import Path
import re

p = Path("src/config.rs")
s = p.read_text()

# Find the first "Config {" initializer block inside from_env/new/default and ensure helius_rpc_url exists.
# We'll insert after helius_api_key if present, else right after "Config {"
start = s.find("Config {")
if start == -1:
    raise SystemExit("❌ No `Config {` initializer found in src/config.rs")

# Walk brace depth to find the initializer end
i = start
depth = 0
end = None
while i < len(s):
    if s[i] == "{":
        depth += 1
    elif s[i] == "}":
        depth -= 1
        if depth == 0:
            end = i
            break
    i += 1

if end is None:
    raise SystemExit("❌ Could not parse Config initializer braces in src/config.rs")

block = s[start:end+1]
if "helius_rpc_url" in block:
    print("ℹ️ helius_rpc_url already present in Config initializer")
    raise SystemExit(0)

ins = '        helius_rpc_url: env_str("HELIUS_RPC_URL", ""),\n'

m = re.search(r'(helius_api_key\s*:\s*env_str\([^\)]*\)\s*,\s*\n)', block)
if m:
    block2 = block[:m.end(1)] + ins + block[m.end(1):]
else:
    block2 = block.replace("Config {\n", "Config {\n" + ins, 1)

s2 = s[:start] + block2 + s[end+1:]
p.write_text(s2)
print("✅ Added helius_rpc_url to Config initializer in src/config.rs")
PY

echo
echo "✅ Done. Now run: cargo build"
