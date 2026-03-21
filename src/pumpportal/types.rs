#[derive(Debug, Clone, Default)]
pub struct PumpMintMeta {
    pub name: Option<String>,
    pub symbol: Option<String>,
    pub description: Option<String>,
    pub twitter: Option<String>,
    pub telegram: Option<String>,
    pub website: Option<String>,
    pub image_uri: Option<String>,
}

impl PumpMintMeta {
    /// Quality score: how much social/marketing prep did this project have?
    /// Prepped projects (website + twitter at launch) are far more likely to run.
    pub fn social_score(&self) -> i32 {
        let mut s = 0i32;
        if self.twitter.as_ref().map(|x| !x.is_empty()).unwrap_or(false) { s += 15; }
        if self.telegram.as_ref().map(|x| !x.is_empty()).unwrap_or(false) { s += 10; }
        if self.website.as_ref().map(|x| !x.is_empty()).unwrap_or(false) { s += 15; }
        // Has a real description (not just a ticker)
        if self.description.as_ref().map(|x| x.len() > 20).unwrap_or(false) { s += 5; }
        s
    }

    pub fn has_socials(&self) -> bool {
        self.twitter.as_ref().map(|x| !x.is_empty()).unwrap_or(false)
            || self.telegram.as_ref().map(|x| !x.is_empty()).unwrap_or(false)
            || self.website.as_ref().map(|x| !x.is_empty()).unwrap_or(false)
    }
}

#[derive(Debug, Clone)]
pub struct PumpMint {
    pub mint: String,
    pub market_cap_sol: Option<f64>,
    pub v_sol_in_bonding_curve: Option<f64>,
    pub v_tokens_in_bonding_curve: Option<f64>,
    pub creator: Option<String>,
    pub meta: PumpMintMeta,
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
