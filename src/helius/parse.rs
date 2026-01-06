use crate::config::Config;
use crate::helius::types::{NativeTransfer, TokenTransfer};
use crate::types::WhaleTier;
use std::collections::HashSet;

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
    if actor == "UNKNOWN" {
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

pub fn collect_mints(tts: &[TokenTransfer], cfg: &Config) -> Vec<String> {
    let mut mints: HashSet<String> = HashSet::new();
    for tt in tts {
        if let Some(m) = &tt.mint {
            if m == &cfg.sol_mint || m == &cfg.usdc_mint {
                continue;
            }
            mints.insert(m.clone());
        }
    }
    mints.into_iter().collect()
}
