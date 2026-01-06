// config.rs
use std::env;

#[derive(Debug, Clone)]
pub struct Config {
    // Existing fields...
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

    pub avoid_bonk: bool,

    pub debug_every_n_scans: u64,
    pub debug_verbose_calls: bool,

    // New for discovery
    pub market_discovery_enabled: bool,
    pub market_discovery_queries: Vec<String>,
    pub market_discovery_every_secs: u64,
    pub market_discovery_top_n: usize,
    pub discovery_min_fdv_usd: f64,
    pub discovery_min_liq_usd: f64,
    pub discovery_min_tx_5m: u64,
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
    match getenv(name, if default { "true" } else { "false" }).to_lowercase().as_str() {
        "1" | "true" | "yes" | "y" | "on" => true,
        "0" | "false" | "no" | "n" | "off" => false,
        _ => default,
    }
}

fn get_csv(name: &str, default: &str) -> Vec<String> {
    getenv(name, default).split(',').map(|s| s.trim().to_string()).collect()
}

pub fn load_config() -> Config {
    // Existing...
    let helius_api_key = getenv("HELIUS_API_KEY", "");
    // ... (all existing)

    Config {
        // Existing fields...
        helius_api_key,
        // ...

        avoid_bonk: get_bool("AVOID_BONK", true),

        debug_every_n_scans: get_u64("DEBUG_EVERY_N_SCANS", 100),
        debug_verbose_calls: get_bool("DEBUG_VERBOSE_CALLS", false),

        // New
        market_discovery_enabled: get_bool("MARKET_DISCOVERY_ENABLED", true),
        market_discovery_queries: get_csv("MARKET_DISCOVERY_QUERIES", "pump,solana,raydium"),
        market_discovery_every_secs: get_u64("MARKET_DISCOVERY_EVERY_SECS", 60),
        market_discovery_top_n: get_usize("MARKET_DISCOVERY_TOP_N", 50),
        discovery_min_fdv_usd: get_f64("DISCOVERY_MIN_FDV_USD", 15000.0),
        discovery_min_liq_usd: get_f64("DISCOVERY_MIN_LIQ_USD", 5000.0),
        discovery_min_tx_5m: get_u64("DISCOVERY_MIN_TX_5M", 20),
    }
}