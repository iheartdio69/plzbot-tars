// ================= CONFIG =================

pub const HELIUS_API_KEY: &str = "PUT_YOUR_KEY_HERE";
pub const HELIUS_ADDR_URL: &str = "https://api-mainnet.helius-rpc.com/v0/addresses";
pub const PUMP_FUN_PROGRAM: &str = "6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P";

pub const MAX_ACTIVE_COINS: usize = 3;

// NOTE: your file said "3m" but was 400s. Make it explicit:
pub const WINDOW_SECS: u64 = 180; // 3 minutes (change if you want)
pub const LOOP_SLEEP_SECS: u64 = 5;
pub const FETCH_LIMIT: usize = 100;

pub const DAILY_CAP: u64 = 33_000;

pub const SNAPSHOT_INTERVAL_SECS: u64 = 90;

pub const MIN_AGE_SECS: u64 = 60;
pub const MIN_SIGNERS_FOR_TARGET: usize = 20;
pub const MIN_TX_FOR_TARGET: usize = 40;

pub const SCORE_TARGET: i32 = 70;
pub const SCORE_STRONG: i32 = 85;
pub const SCORE_DEMOTE: i32 = 45;
pub const DEMOTE_STREAK: u8 = 4;

pub const ACCEL_WALLET_GROWTH_PCT: f64 = 0.50;
pub const ACCEL_TX_GROWTH_PCT: f64 = 0.75;

// Whale tiers
pub const BELUGA_SOL_TX: f64 = 2.0;
pub const BLUE_SOL_TX: f64 = 5.0;

// Noise mints
pub const SOL_MINT: &str = "So11111111111111111111111111111111111111112";
pub const USDC_MINT: &str = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";

// Dedup
pub const SEEN_SIG_CAP: usize = 10_000;

// Resolver timing
pub const RESOLVE_T5_SECS: u64 = 5 * 60;
pub const RESOLVE_T15_SECS: u64 = 15 * 60;
pub const RESOLVE_CHECK_EVERY_SECS: u64 = 30;

// Keep enough history for resolver + buffer
pub const EVENTS_KEEP_SECS: u64 = RESOLVE_T15_SECS + 5 * 60;

// Win/Loss rules
pub const WIN_WALLET_MULT: f64 = 1.70;
pub const WIN_TX_MULT: f64 = 2.20;
pub const MID_WALLET_MULT: f64 = 1.25;
pub const MID_TX_MULT: f64 = 1.35;

// Cadence
pub const SAVE_EVERY_SECS: u64 = 60;
pub const PRINT_TOP_WALLETS_EVERY_SECS: u64 = 180;

// ================= MARKET (DEXSCREENER) =================
pub const MARKET_POLL_SECS: u64 = 20;

pub const MAX_FDV_USD: f64 = 250_000.0;
pub const MIN_LIQ_USD: f64 = 5_000.0;
pub const PRICE_UP_BOOST: i32 = 10;
pub const FDV_OK_BOOST: i32 = 5;