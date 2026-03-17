use crate::config::Config;
use crate::helius::types::{NativeTransfer, TokenTransfer};
use crate::types::WhaleTier;
use std::collections::HashSet;

// Known mints (Solana mainnet)
const SOL_MINT: &str = "So11111111111111111111111111111111111111112";
const USDC_MINT: &str = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";

pub fn classify_tier(sol_out: f64, cfg: &Config) -> WhaleTier {
    if sol_out >= cfg.blue_sol_tx {
        WhaleTier::Blue
    } else if sol_out >= cfg.beluga_sol_tx {
        WhaleTier::Beluga
    } else {
        WhaleTier::None
    }
}

pub fn estimate_sol_outflow(native: &[NativeTransfer], actor: &str) -> f64 {
    if actor.is_empty() || actor == "UNKNOWN" {
        return 0.0;
    }
    let mut lamports_out: u64 = 0;
    for nt in native {
        let from = nt.from_user_account.as_deref().unwrap_or("");
        if from == actor {
            lamports_out = lamports_out.saturating_add(nt.amount);
        }
    }
    (lamports_out as f64) / 1_000_000_000.0
}

pub fn collect_mints(tts: &[TokenTransfer]) -> Vec<String> {
    let mut mints: HashSet<String> = HashSet::new();
    for tt in tts {
        let Some(m) = tt.mint.as_ref() else {
            continue;
        };
        let m = m.trim();
        if m.is_empty() {
            continue;
        }
        if m == SOL_MINT || m == USDC_MINT {
            continue;
        }
        mints.insert(m.to_string());
    }
    mints.into_iter().collect()
}
