use crate::types::WhaleTier;

#[inline]
fn is_ignored_mint(m: &str) -> bool {
    matches!(
        m,
        "So11111111111111111111111111111111111111112" | // wSOL
        "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v" | // USDC
        "Es9vMFrzaCERmJfrF4H2FYD4KCoNkY11McCe8BenwNYB" // USDT (common)
    )
}

pub fn classify_tier(sol: f64, beluga_sol_tx: f64, blue_sol_tx: f64) -> WhaleTier {
    if sol >= blue_sol_tx {
        WhaleTier::Blue
    } else if sol >= beluga_sol_tx {
        WhaleTier::Beluga
    } else {
        WhaleTier::None
    }
}

pub fn filter_mints(mints: Vec<String>) -> Vec<String> {
    mints
        .into_iter()
        .filter(|m| {
            let mm = m.trim();
            !mm.is_empty() && !is_ignored_mint(mm)
        })
        .collect()
}
