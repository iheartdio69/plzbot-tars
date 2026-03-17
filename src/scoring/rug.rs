pub struct RugScan {
    pub rug_score: i32,
    pub freeze_auth: bool,
    pub mint_auth: bool,
    pub liq_usd: f64,
    pub liq_to_fdv: f64,
    pub reasons: Vec<&'static str>,
}