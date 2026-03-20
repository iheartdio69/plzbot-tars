#[derive(Debug, Clone)]
pub struct PumpMint {
    pub mint: String,
    pub market_cap_sol: Option<f64>,
    pub v_sol_in_bonding_curve: Option<f64>,
    pub v_tokens_in_bonding_curve: Option<f64>,
    pub creator: Option<String>,
}
