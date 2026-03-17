use std::env;

#[derive(Debug, Clone)]
pub struct Config {
    // PumpPortal
    pub pumpportal_enabled: bool,
    pub pumpportal_wss: String,
    pub pumpportal_api_key: String,
    pub pumpportal_channel: String,

    // Core loop
    pub main_loop_sleep: u64,

    // Discovery tuning
    pub market_discovery_enabled: bool,
    pub market_discovery_every_secs: u64,
    pub market_discovery_top_n: usize,
    pub discovery_min_fdv_usd: f64,
    pub discovery_min_liq_usd: f64,
    pub discovery_min_tx_5m: u64,
    pub discovery_max_age_secs: u64,

    // Active / scoring tuning
    pub min_watch_fdv_usd: f64,
    pub max_watch_fdv_usd: f64,
    pub min_liq_usd: f64,
    pub min_tx_for_target: usize,
    pub min_signers_for_target: usize,
    pub score_target: i32,
    pub score_demote: i32,
    pub demote_streak: u32,
    pub max_active_coins: usize,

    // Call gate
    pub min_call_fdv_usd: f64,
    pub max_call_fdv_usd: f64,

    // Whale tiers
    pub beluga_sol_tx: f64,
    pub blue_sol_tx: f64,

    // Helius
    pub helius_api_key: String,
    pub helius_addr_url: String,
    pub helius_rpc_url: String,
    pub fetch_limit: usize,
    pub helius_wallets: String,

    // QUEUE
    pub queue_score_min: i32,

    // DB
    pub sqlite_path: String,

    // Discovery queries
    pub market_discovery_queries: Vec<String>,
}

pub fn load_config() -> Config {
    dotenvy::dotenv().ok();

    Config {
        // PumpPortal
        pumpportal_enabled: env_bool("PUMPPORTAL_ENABLED", true),
        pumpportal_wss: env_str("PUMPPORTAL_WSS", "wss://pumpportal.fun/api/data"),
        pumpportal_api_key: env_str("PUMPPORTAL_API_KEY", ""),
        pumpportal_channel: env_str("PUMPPORTAL_CHANNEL", "subscribeNewToken"),

        // Core
        main_loop_sleep: env_u64("MAIN_LOOP_SLEEP", 5),

        // Discovery
        market_discovery_enabled: env_bool("MARKET_DISCOVERY_ENABLED", true),
        market_discovery_every_secs: env_u64("MARKET_DISCOVERY_EVERY_SECS", 15),
        market_discovery_top_n: env_usize("MARKET_DISCOVERY_TOP_N", 50),
        discovery_min_fdv_usd: env_f64("DISCOVERY_MIN_FDV_USD", 5_000.0),
        discovery_min_liq_usd: env_f64("DISCOVERY_MIN_LIQ_USD", 0.0),
        discovery_min_tx_5m: env_u64("DISCOVERY_MIN_TX_5M", 0),
        discovery_max_age_secs: env_u64("DISCOVERY_MAX_AGE_SECS", 86_400),

        // Active / scoring
        min_watch_fdv_usd: env_f64("MIN_WATCH_FDV_USD", 10_000.0),
        max_watch_fdv_usd: env_f64("MAX_WATCH_FDV_USD", 2_000_000.0),
        min_liq_usd: env_f64("MIN_LIQ_USD", 8_000.0),
        min_tx_for_target: env_usize("MIN_TX_FOR_TARGET", 10),
        min_signers_for_target: env_usize("MIN_SIGNERS_FOR_TARGET", 6),
        score_target: env_i32("SCORE_TARGET", 40),
        score_demote: env_i32("SCORE_DEMOTE", -10),
        demote_streak: env_u32("DEMOTE_STREAK", 3),
        max_active_coins: env_usize("MAX_ACTIVE_COINS", 30),

        // Call gate
        min_call_fdv_usd: env_f64("MIN_CALL_FDV_USD", 15_000.0),
        max_call_fdv_usd: env_f64("MAX_CALL_FDV_USD", 55_000.0),

        // Whale tiers (Blue > Beluga)
        beluga_sol_tx: env_f64("BELUGA_SOL_TX", 2.0),
        blue_sol_tx: env_f64("BLUE_SOL_TX", 5.0),

        // Helius
        helius_api_key: env_str("HELIUS_API_KEY", ""),
        helius_addr_url: env_str("HELIUS_ADDR_URL", "https://api.helius.xyz"),
        helius_rpc_url: env_str("HELIUS_RPC_URL", ""),
        fetch_limit: env_usize("FETCH_LIMIT", 50),
        helius_wallets: env_str("HELIUS_WALLETS", ""),

        // QUEUE
        queue_score_min: env_i32("QUEUE_SCORE_MIN", 20),

        // DB
        sqlite_path: env_str("SQLITE_PATH", "data/solana_meme.sqlite"),

        // Queries
        market_discovery_queries: env_list(
            "MARKET_DISCOVERY_QUERIES",
            vec!["pump".to_string(), "pumpfun".to_string()],
        ),
    }
}

/* ---------------- helpers ---------------- */

fn env_str(name: &str, default: &str) -> String {
    env::var(name).unwrap_or_else(|_| default.to_string())
}

fn env_bool(name: &str, default: bool) -> bool {
    env::var(name)
        .ok()
        .map(|v| matches!(v.to_lowercase().as_str(), "1" | "true" | "yes" | "y" | "on"))
        .unwrap_or(default)
}

fn env_u64(name: &str, default: u64) -> u64 {
    env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn env_u32(name: &str, default: u32) -> u32 {
    env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn env_usize(name: &str, default: usize) -> usize {
    env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn env_i32(name: &str, default: i32) -> i32 {
    env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn env_f64(name: &str, default: f64) -> f64 {
    env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn env_list(name: &str, default: Vec<String>) -> Vec<String> {
    let v = env::var(name).unwrap_or_default();
    if v.trim().is_empty() {
        return default;
    }
    v.split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}
