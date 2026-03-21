use std::env;

#[derive(Debug, Clone)]
pub struct Config {
    pub helius_api_key: String,
    pub helius_rpc_url: String,
    pub helius_addr_url: String,
    pub pump_fun_program: String,

    pub fetch_limit: usize,

    pub main_loop_sleep: u64,
    pub market_poll_secs: u64,

    pub window_secs: u64,
    pub events_keep_secs: u64,
    pub snapshot_interval_secs: u64,

    pub min_scan_age_secs: u64,
    pub max_coin_age_secs: u64,
    pub min_age_secs: u64,

    pub min_watch_fdv_usd: f64,
    pub max_watch_fdv_usd: f64,
    pub min_call_fdv_usd: f64,
    pub max_call_fdv_usd: f64,
    pub min_liq_usd: f64,

    pub score_target: i32,
    pub score_demote: i32,
    pub demote_streak: u8,

    pub min_signers_for_target: usize,
    pub min_tx_for_target: usize,

    pub max_active_coins: usize,

    pub beluga_sol_tx: f64,
    pub blue_sol_tx: f64,

    pub sol_mint: String,
    pub usdc_mint: String,

    pub avoid_bonk: bool,

    pub debug_every_n_scans: u64,
    pub debug_verbose_calls: bool,

    pub market_discovery_enabled: bool,
    pub market_discovery_queries: Vec<String>,
    pub market_discovery_every_secs: u64,
    pub market_discovery_top_n: usize,
    pub discovery_min_fdv_usd: f64,
    pub discovery_min_liq_usd: f64,
    pub discovery_min_tx_5m: u64,

    pub resolve_t5_secs: u64,
    pub resolve_t15_secs: u64,
    pub win_wallet_mult: f64,
    pub win_tx_mult: f64,
    pub mid_wallet_mult: f64,
    pub mid_tx_mult: f64,

    // FDV velocity — % per minute to consider a coin "pumping"
    pub fdv_velocity_threshold: f64,
    // Minimum buy/sell ratio to consider bullish
    pub min_buy_sell_ratio: f64,
    // Minimum buys in 5m to care
    pub min_buys_5m: u64,
}

fn getenv(name: &str, default: &str) -> String {
    env::var(name).unwrap_or_else(|_| default.to_string())
}

fn get_u64(name: &str, default: u64) -> u64 {
    getenv(name, &default.to_string()).parse().unwrap_or(default)
}

fn get_usize(name: &str, default: usize) -> usize {
    getenv(name, &default.to_string()).parse().unwrap_or(default)
}

fn get_i32(name: &str, default: i32) -> i32 {
    getenv(name, &default.to_string()).parse().unwrap_or(default)
}

fn get_f64(name: &str, default: f64) -> f64 {
    getenv(name, &default.to_string()).parse().unwrap_or(default)
}

fn get_bool(name: &str, default: bool) -> bool {
    match getenv(name, if default { "true" } else { "false" })
        .to_lowercase()
        .as_str()
    {
        "1" | "true" | "yes" | "y" | "on" => true,
        "0" | "false" | "no" | "n" | "off" => false,
        _ => default,
    }
}

fn get_csv(name: &str, default: &str) -> Vec<String> {
    getenv(name, default)
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

pub fn load_config() -> Config {
    Config {
        helius_api_key: getenv("HELIUS_API_KEY", ""),
        helius_rpc_url: getenv("HELIUS_RPC_URL", ""),
        helius_addr_url: getenv(
            "HELIUS_ADDR_URL",
            "https://api.helius.xyz/v0/addresses",
        ),
        pump_fun_program: getenv(
            "PUMP_FUN_PROGRAM",
            "6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P",
        ),

        fetch_limit: get_usize("FETCH_LIMIT", 75),

        main_loop_sleep: get_u64("MAIN_LOOP_SLEEP", 1),
        market_poll_secs: get_u64("MARKET_POLL_SECS", 15),

        window_secs: get_u64("WINDOW_SECS", 300),
        events_keep_secs: get_u64("EVENTS_KEEP_SECS", 600),
        snapshot_interval_secs: get_u64("SNAPSHOT_INTERVAL_SECS", 30),

        min_scan_age_secs: get_u64("MIN_SCAN_AGE_SECS", 0),
        max_coin_age_secs: get_u64("DISCOVERY_MAX_AGE_SECS", 10000),
        min_age_secs: get_u64("MIN_AGE_SECS", 60),

        min_watch_fdv_usd: get_f64("MIN_WATCH_FDV_USD", 5000.0),
        max_watch_fdv_usd: get_f64("MAX_WATCH_FDV_USD", 10_000_000.0),
        min_call_fdv_usd: get_f64("MIN_CALL_FDV_USD", 10000.0),
        max_call_fdv_usd: get_f64("MAX_CALL_FDV_USD", 500000.0),
        min_liq_usd: get_f64("DISCOVERY_MIN_LIQ_USD", 0.0),

        score_target: get_i32("SCORE_TARGET", 40),
        score_demote: get_i32("SCORE_DEMOTE", -10),
        demote_streak: get_u64("DEMOTE_STREAK", 10) as u8,

        min_signers_for_target: get_usize("MIN_SIGNERS_FOR_TARGET", 6),
        min_tx_for_target: get_usize("MIN_TX_FOR_TARGET", 10),

        max_active_coins: get_usize("MAX_ACTIVE_COINS", 10),

        beluga_sol_tx: get_f64("BELUGA_SOL_TX", 2.0),
        blue_sol_tx: get_f64("BLUE_SOL_TX", 5.0),

        sol_mint: "So11111111111111111111111111111111111111112".to_string(),
        usdc_mint: "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v".to_string(),

        avoid_bonk: get_bool("AVOID_BONK", true),

        debug_every_n_scans: get_u64("DEBUG_EVERY_N_SCANS", 100),
        debug_verbose_calls: get_bool("DEBUG_VERBOSE_CALLS", false),

        market_discovery_enabled: get_bool("MARKET_DISCOVERY_ENABLED", true),
        market_discovery_queries: get_csv("MARKET_DISCOVERY_QUERIES", "pump,solana,raydium"),
        market_discovery_every_secs: get_u64("MARKET_DISCOVERY_EVERY_SECS", 60),
        market_discovery_top_n: get_usize("MARKET_DISCOVERY_TOP_N", 50),
        discovery_min_fdv_usd: get_f64("DISCOVERY_MIN_FDV_USD", 15000.0),
        discovery_min_liq_usd: get_f64("DISCOVERY_MIN_LIQ_USD", 0.0),
        discovery_min_tx_5m: get_u64("DISCOVERY_MIN_TX_5M", 0),

        resolve_t5_secs: get_u64("RESOLVE_T5_SECS", 300),
        resolve_t15_secs: get_u64("RESOLVE_T15_SECS", 900),
        win_wallet_mult: get_f64("WIN_WALLET_MULT", 1.5),
        win_tx_mult: get_f64("WIN_TX_MULT", 2.0),
        mid_wallet_mult: get_f64("MID_WALLET_MULT", 1.1),
        mid_tx_mult: get_f64("MID_TX_MULT", 1.3),

        fdv_velocity_threshold: get_f64("FDV_VELOCITY_THRESHOLD", 2.0), // 2% per minute
        min_buy_sell_ratio: get_f64("MIN_BUY_SELL_RATIO", 1.5),
        min_buys_5m: get_u64("MIN_BUYS_5M", 10),
    }
}
