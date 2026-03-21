#[derive(Debug, Clone)]
pub struct PumpMint {
    pub mint: String,
    pub market_cap_sol: Option<f64>,
    pub v_sol_in_bonding_curve: Option<f64>,
    pub v_tokens_in_bonding_curve: Option<f64>,
    pub creator: Option<String>,
}

/// A single trade event from the pump.fun trade stream (subscribeTokenTrade).
/// This gives us real wallet addresses in real-time — no Helius needed for pre-grad coins.
#[derive(Debug, Clone)]
pub struct PumpTrade {
    pub mint: String,
    pub trader: String,      // wallet that made the trade
    pub sol_amount: f64,     // SOL spent/received
    pub is_buy: bool,        // true = buy, false = sell
    pub market_cap_sol: Option<f64>,
    pub ts: u64,             // unix seconds
}
