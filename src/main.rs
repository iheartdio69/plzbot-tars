use std::{
    collections::{HashMap, HashSet, VecDeque},
    fs,
    time::{Duration, Instant},
};

use colored::*;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use tokio::time::sleep;

/* ================= CONFIG ================= */

const HELIUS_API_KEY: &str = "3eed6b62-9f1a-4093-ac9e-92226a323815"; // <-- put your key
const HELIUS_ADDR_URL: &str = "https://api-mainnet.helius-rpc.com/v0/addresses";
const PUMP_FUN_PROGRAM: &str = "6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P";

const MAX_ACTIVE_COINS: usize = 3;
const WINDOW_SECS: u64 = 400; // 3m

const LOOP_SLEEP_SECS: u64 = 5;
const FETCH_LIMIT: usize = 100;

const DAILY_CAP: u64 = 33_000; // helius credits cap/day

const SNAPSHOT_INTERVAL_SECS: u64 = 90;

const MIN_AGE_SECS: u64 = 60;
const MIN_SIGNERS_FOR_TARGET: usize = 20;
const MIN_TX_FOR_TARGET: usize = 40;

const SCORE_TARGET: i32 = 70;
const SCORE_STRONG: i32 = 85;
const SCORE_DEMOTE: i32 = 45;
const DEMOTE_STREAK: u8 = 4;

const ACCEL_WALLET_GROWTH_PCT: f64 = 0.50;
const ACCEL_TX_GROWTH_PCT: f64 = 0.75;

// Whale tiers (per-tx SOL outflow estimate)
const BELUGA_SOL_TX: f64 = 2.0;
const BLUE_SOL_TX: f64 = 5.0;

const SOL_MINT: &str = "So11111111111111111111111111111111111111112";
const USDC_MINT: &str = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";

const SEEN_SIG_CAP: usize = 10_000;

// Resolver timing
const RESOLVE_T5_SECS: u64 = 5 * 60;
const RESOLVE_T15_SECS: u64 = 15 * 60;
const RESOLVE_CHECK_EVERY_SECS: u64 = 30;
// keep enough history for resolver (15m) + buffer
const EVENTS_KEEP_SECS: u64 = RESOLVE_T15_SECS + 5 * 60; // 20 minutes
// Win/Loss rules
const WIN_WALLET_MULT: f64 = 1.70;
const WIN_TX_MULT: f64 = 2.20;
const MID_WALLET_MULT: f64 = 1.25;
const MID_TX_MULT: f64 = 1.35;

// Printing / saving cadence
const SAVE_EVERY_SECS: u64 = 60;
const PRINT_TOP_WALLETS_EVERY_SECS: u64 = 180;

/* ================= MARKET (DEXSCREENER) ================= */

// Poll Dexscreener for active mints + top queue
const MARKET_POLL_SECS: u64 = 20;

// Lightweight filters/boosts.
// fdv on Dexscreener is “FDV” (often used as MC proxy); sometimes it’s missing.
const MAX_FDV_USD: f64 = 250_000.0; // <-- tweak later
const MIN_LIQ_USD: f64 = 5_000.0;   // <-- tweak later
const PRICE_UP_BOOST: i32 = 10;     // score boost if price rising
const FDV_OK_BOOST: i32 = 5;        // score boost if FDV under cap + liq ok

type MarketCache = HashMap<String, VecDeque<MarketSnap>>;

#[derive(Debug, Clone)]
struct MarketSnap {
    ts: u64,
    price_usd: Option<f64>,
    fdv: Option<f64>,
    liquidity_usd: Option<f64>,
    vol_h24: Option<f64>,
}

fn cache_push(cache: &mut MarketCache, mint: &str, snap: MarketSnap) {
    let q = cache.entry(mint.to_string()).or_insert_with(VecDeque::new);
    q.push_back(snap);
    while q.len() > 30 {
        q.pop_front();
    }
}

fn market_trend(cache: &MarketCache, mint: &str) -> MarketTrend {
    let Some(q) = cache.get(mint) else {
        return MarketTrend::default();
    };
    if q.len() < 2 {
        return MarketTrend::default();
    }
    let first = &q[0];
    let last = &q[q.len() - 1];

    let price_up = match (first.price_usd, last.price_usd) {
        (Some(a), Some(b)) if a > 0.0 && b > a => true,
        _ => false,
    };

    let fdv_ok = match last.fdv {
        Some(fdv) => fdv > 0.0 && fdv <= MAX_FDV_USD,
        None => false,
    };

    let liq_ok = match last.liquidity_usd {
        Some(l) => l >= MIN_LIQ_USD,
        None => false,
    };

    MarketTrend {
        price_up,
        fdv_ok,
        liq_ok,
        last_price: last.price_usd,
        last_fdv: last.fdv,
        last_liq: last.liquidity_usd,
    }
}

#[derive(Default, Debug, Clone)]
struct MarketTrend {
    price_up: bool,
    fdv_ok: bool,
    liq_ok: bool,
    last_price: Option<f64>,
    last_fdv: Option<f64>,
    last_liq: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct DexSearchResp {
    pairs: Vec<DexPair>,
}

#[derive(Debug, Deserialize)]
struct DexPair {
    chainId: Option<String>,
    dexId: Option<String>,
    priceUsd: Option<String>,
    fdv: Option<f64>,
    liquidity: Option<DexLiquidity>,
    volume: Option<DexVolume>,
    baseToken: Option<DexToken>,
    quoteToken: Option<DexToken>,
}

#[derive(Debug, Deserialize)]
struct DexLiquidity {
    usd: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct DexVolume {
    h24: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct DexToken {
    address: Option<String>,
    symbol: Option<String>,
}

async fn fetch_dexscreener_snap(client: &Client, mint: &str) -> Option<MarketSnap> {
    // Search endpoint: returns pairs; we pick first Solana pair where baseToken matches mint
    let url = format!("https://api.dexscreener.com/latest/dex/search?q={}", mint);
    let res = client.get(url).send().await.ok()?;
    let parsed = res.json::<DexSearchResp>().await.ok()?;

    let mut best: Option<&DexPair> = None;
    for p in parsed.pairs.iter() {
        let chain_ok = p.chainId.as_deref().unwrap_or("").eq_ignore_ascii_case("solana");
        if !chain_ok {
            continue;
        }
        let base_addr = p
            .baseToken
            .as_ref()
            .and_then(|t| t.address.as_deref())
            .unwrap_or("");
        if base_addr != mint {
            continue;
        }
        best = Some(p);
        break;
    }

    let p = best?;

    let price_usd = p
        .priceUsd
        .as_ref()
        .and_then(|s| s.parse::<f64>().ok());

    let liquidity_usd = p.liquidity.as_ref().and_then(|l| l.usd);
    let vol_h24 = p.volume.as_ref().and_then(|v| v.h24);

    Some(MarketSnap {
        ts: now(),
        price_usd,
        fdv: p.fdv,
        liquidity_usd,
        vol_h24,
    })
}

/* ================= HELIUS TYPES ================= */

#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct HeliusTx {
    signature: String,
    timestamp: u64,

    #[serde(default)]
    fee_payer: Option<String>,

    #[serde(default)]
    token_transfers: Vec<TokenTransfer>,

    #[serde(default)]
    native_transfers: Vec<NativeTransfer>,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct TokenTransfer {
    #[serde(default)]
    mint: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct NativeTransfer {
    #[serde(default)]
    from_user_account: Option<String>,
    #[serde(default)]
    amount: u64, // lamports
}

/* ================= DATA ================= */

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum WhaleTier {
    None,
    Beluga,
    Blue,
}

#[derive(Debug, Clone)]
struct Event {
    wallet: String,
    ts: u64,
    sol: f64,
    tier: WhaleTier,
}

#[derive(Debug)]
struct CoinState {
    first_seen: Instant,
    last_update: Instant,
    events: Vec<Event>,
    active: bool,

    last_snapshot: Instant,
    prev_tx_window: usize,
    prev_signers_window: usize,
    low_score_streak: u8,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
struct WalletStats {
    seen: u32,
    wins: u32,
    losses: u32,
    score: i32,
    last_seen_ts: u64,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
struct WhalePerf {
    seen: u32,
    beluga_txs: u32,
    blue_txs: u32,
    beluga_sol: f64,
    blue_sol: f64,
    wins: u32,
    losses: u32,
    score: f64,
    last_seen_ts: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CallRecord {
    mint: String,
    call_ts: u64,
    score: i32,

    t5_ts: Option<u64>,
    wallets_t5: Option<usize>,
    tx_t5: Option<usize>,

    t15_ts: Option<u64>,
    wallets_t15: Option<usize>,
    tx_t15: Option<usize>,

    outcome: Option<String>, // "WIN" | "MID" | "LOSS"

    wallets_involved: Vec<String>,
    whales_involved: Vec<String>,
}

/* ================= USAGE CAP ================= */

#[derive(Serialize, Deserialize, Default, Debug, Clone)]
struct Usage {
    day: u64,
    requests: u64,
}

fn day_number_now() -> u64 {
    now() / 86_400
}

fn load_usage(path: &str) -> Usage {
    let Ok(s) = fs::read_to_string(path) else {
        return Usage {
            day: day_number_now(),
            requests: 0,
        };
    };
    serde_json::from_str(&s).unwrap_or_else(|_| Usage {
        day: day_number_now(),
        requests: 0,
    })
}

fn save_usage(path: &str, u: &Usage) {
    if let Ok(s) = serde_json::to_string_pretty(u) {
        let _ = fs::write(path, s);
    }
}

fn fmt_i64_commas(n: i64) -> String {
    let mut s = n.abs().to_string();
    let mut out = String::new();
    while s.len() > 3 {
        let chunk = s.split_off(s.len() - 3);
        if out.is_empty() {
            out = chunk;
        } else {
            out = format!("{},{}", chunk, out);
        }
    }
    if out.is_empty() {
        out = s;
    } else {
        out = format!("{},{}", s, out);
    }
    if n < 0 { format!("-{}", out) } else { out }
}

fn fmt_f64_0_commas(x: f64) -> String {
    fmt_i64_commas(x.round() as i64)
}
/* ================= MAIN ================= */

#[tokio::main]
async fn main() {
    println!(
        "{}",
        "🧠 Ishmael Runner Radar (RESOLVER + whales.json + wallets.json)"
            .bold()
            .green()
    );
    println!(
        "{}",
        format!(
            "Heartbeat enabled. {} slots. Beluga ≥ {} SOL, Blue ≥ {} SOL. Resolver: 5m→15m.",
            MAX_ACTIVE_COINS, BELUGA_SOL_TX, BLUE_SOL_TX
        )
        .bright_black()
    );

    let client = Client::new();

    let mut coins: HashMap<String, CoinState> = HashMap::new();
    let mut active: Vec<String> = vec![];
    let mut queue: VecDeque<String> = VecDeque::new();

    let mut seen_sigs: VecDeque<String> = VecDeque::new();
    let mut seen_set: HashSet<String> = HashSet::new();

    let mut wallets: HashMap<String, WalletStats> = load_json("wallets.json").unwrap_or_default();
    let mut whales: HashMap<String, WhalePerf> = load_json("whales.json").unwrap_or_default();
    let mut calls: Vec<CallRecord> = load_json("calls.json").unwrap_or_default();

    // ===== usage cap state =====
    let usage_path = "usage.json";
    let mut usage = load_usage(usage_path);

    // ===== market cache state =====
    let mut market: MarketCache = HashMap::new();
    let mut last_market_poll = Instant::now()
        .checked_sub(Duration::from_secs(999))
        .unwrap_or(Instant::now());

    // cadence
    let mut last_save = Instant::now()
        .checked_sub(Duration::from_secs(999))
        .unwrap_or(Instant::now());
    let mut last_print_top = Instant::now()
        .checked_sub(Duration::from_secs(999))
        .unwrap_or(Instant::now());
    let mut last_resolve_tick = Instant::now()
        .checked_sub(Duration::from_secs(999))
        .unwrap_or(Instant::now());

    loop {
        // ===== day reset =====
        let today = day_number_now();
        if usage.day != today {
            usage.day = today;
            usage.requests = 0;
            save_usage(usage_path, &usage);
        }

        // ===== cap check =====
        if usage.requests >= DAILY_CAP {
            println!(
                "{}",
                format!(
                    "🛑 DAILY CAP HIT: {}/{} requests. Sleeping 60s…",
                    usage.requests, DAILY_CAP
                )
                .bold()
                .yellow()
            );
            save_usage(usage_path, &usage);
            sleep(Duration::from_secs(60)).await;
            continue;
        }

        // ===== MARKET POLL (does NOT consume helius credits) =====
        if last_market_poll.elapsed().as_secs() >= MARKET_POLL_SECS {
            for mint in active.iter() {
                if let Some(snap) = fetch_dexscreener_snap(&client, mint).await {
                    cache_push(&mut market, mint, snap);
                }
            }
            for mint in queue.iter().take(3) {
                if let Some(snap) = fetch_dexscreener_snap(&client, mint).await {
                    cache_push(&mut market, mint, snap);
                }
            }
            last_market_poll = Instant::now();
        }

        // ===== HELIUS FETCH (consumes helius credits) =====
        usage.requests = usage.requests.saturating_add(1);
        save_usage(usage_path, &usage);

        match fetch_latest_program_txs(&client).await {
            Ok(txs) => {
                let fetched = txs.len();
                let mut new_count = 0usize;

                for tx in txs {
                    if seen_set.contains(&tx.signature) {
                        continue;
                    }
                    new_count += 1;

                    seen_set.insert(tx.signature.clone());
                    seen_sigs.push_back(tx.signature.clone());
                    while seen_sigs.len() > SEEN_SIG_CAP {
                        if let Some(old) = seen_sigs.pop_front() {
                            seen_set.remove(&old);
                        }
                    }

                    let actor = tx
                        .fee_payer
                        .clone()
                        .unwrap_or_else(|| "UNKNOWN".to_string());
                    let ts = tx.timestamp;

                    let sol_out = estimate_sol_outflow(&tx.native_transfers, &actor);
                    let tier = classify_tier(sol_out);

                    let mints = collect_mints(&tx.token_transfers);
                    if mints.is_empty() {
                        continue;
                    }

                    let sol_per_mint = if sol_out > 0.0 {
                        sol_out / (mints.len() as f64)
                    } else {
                        0.0
                    };

                    if actor != "UNKNOWN" && tier != WhaleTier::None {
                        let wp = whales.entry(actor.clone()).or_default();
                        wp.last_seen_ts = ts;
                        wp.seen = wp.seen.saturating_add(1);

                        match tier {
                            WhaleTier::Blue => {
                                wp.blue_txs = wp.blue_txs.saturating_add(1);
                                wp.blue_sol += sol_out;
                            }
                            WhaleTier::Beluga => {
                                wp.beluga_txs = wp.beluga_txs.saturating_add(1);
                                wp.beluga_sol += sol_out;
                            }
                            WhaleTier::None => {}
                        }
                    }

                    if actor != "UNKNOWN" {
                        let ws = wallets.entry(actor.clone()).or_default();
                        ws.last_seen_ts = ts;
                        ws.seen = ws.seen.saturating_add(1);
                    }

                    for mint in mints {
                        let entry = coins.entry(mint.clone()).or_insert_with(|| CoinState {
                            first_seen: Instant::now(),
                            last_update: Instant::now(),
                            events: vec![],
                            active: false,
                            last_snapshot: Instant::now(),
                            prev_tx_window: 0,
                            prev_signers_window: 0,
                            low_score_streak: 0,
                        });

                        entry.last_update = Instant::now();
                        entry.events.push(Event {
                            wallet: actor.clone(),
                            ts,
                            sol: sol_per_mint,
                            tier,
                        });
                        prune_window(&mut entry.events, EVENTS_KEEP_SECS);
                    }
                }

                println!(
                    "{}",
                    format!(
                        "📡 fetched {} txs | new {} | used today {}/{}",
                        fetched, new_count, usage.requests, DAILY_CAP
                    )
                    .bright_black()
                );

                // scoring + calls
                score_and_manage(
                    &mut coins,
                    &mut active,
                    &mut queue,
                    &mut calls,
                    &market,
                );

                // resolver tick
                if last_resolve_tick.elapsed().as_secs() >= RESOLVE_CHECK_EVERY_SECS {
                    resolver_tick(&coins, &mut calls, &mut wallets, &mut whales);
                    last_resolve_tick = Instant::now();
                }

                // print summaries
                if last_print_top.elapsed().as_secs() >= PRINT_TOP_WALLETS_EVERY_SECS {
                    print_wallet_stats(&wallets);
                    print_best_worst_calls(&calls);
                    last_print_top = Instant::now();
                }

                // save periodically
                if last_save.elapsed().as_secs() >= SAVE_EVERY_SECS {
                    let _ = save_json("wallets.json", &wallets);
                    let _ = save_json("whales.json", &whales);
                    let _ = save_json("calls.json", &calls);
                    last_save = Instant::now();
                }
            }
            Err(_) => {
                println!("{}", "⚠️ fetch failed (network/rate-limit). retrying…".yellow());
            }
        }

        sleep(Duration::from_secs(LOOP_SLEEP_SECS)).await;
    }
}

/* ================= SCORING / MANAGEMENT ================= */

fn score_and_manage(
    coins: &mut HashMap<String, CoinState>,
    active: &mut Vec<String>,
    queue: &mut VecDeque<String>,
    calls: &mut Vec<CallRecord>,
    market: &MarketCache,
) {
    let keys: Vec<String> = coins.keys().cloned().collect();

    for mint in keys {
        let Some(c) = coins.get_mut(&mint) else { continue; };

        if c.last_snapshot.elapsed().as_secs() < SNAPSHOT_INTERVAL_SECS {
            continue;
        }
        c.last_snapshot = Instant::now();

        let age = c.first_seen.elapsed().as_secs();
        let (tx_now, signers_now, whales_now) = window_stats_for(&c.events, WINDOW_SECS);

        // update prev even if young
        if age < MIN_AGE_SECS {
            c.prev_tx_window = tx_now;
            c.prev_signers_window = signers_now;
            continue;
        }

        let (mut score, wallet_growth_pct, tx_growth_pct) =
            runner_score(signers_now, tx_now, c.prev_signers_window, c.prev_tx_window);

        c.prev_tx_window = tx_now;
        c.prev_signers_window = signers_now;

        // whale boosts
        if whales_now.beluga_count >= 1 { score += 10; }
        if whales_now.blue_count >= 1 { score += 15; }
        if whales_now.blue_count >= 2 { score += 10; }

        // ===== market filter/boost =====
        let trend = market_trend(market, &mint);
        // If we have market data and it looks bad, you can soften score or reject calls.
        if trend.fdv_ok && trend.liq_ok {
            score += FDV_OK_BOOST;
        }
        if trend.price_up {
            score += PRICE_UP_BOOST;
        }

        // promote/queue + create call record
        let passes_activity =
            score >= SCORE_TARGET && signers_now >= MIN_SIGNERS_FOR_TARGET && tx_now >= MIN_TX_FOR_TARGET;

        // Optional: if you want to REQUIRE market confirmation for targets:
        // let market_ok = !market.get(&mint).is_some() || (trend.fdv_ok && trend.liq_ok && trend.price_up);
        // if passes_activity && market_ok { ... }
        // For now: we do NOT hard-require it, only boost.

        if passes_activity {
            if !c.active {
                if active.len() < MAX_ACTIVE_COINS && !active.contains(&mint) {
                    c.active = true;
                    active.push(mint.clone());
                    println!("{}", format!("🎯 TARGET → {} (score {})", mint, score).bold().green());

                    let involved_wallets = window_wallets(&c.events);
                    let involved_whales = window_whales(&c.events);
                    calls.push(CallRecord {
                        mint: mint.clone(),
                        call_ts: now(),
                        score,
                        t5_ts: None,
                        wallets_t5: None,
                        tx_t5: None,
                        t15_ts: None,
                        wallets_t15: None,
                        tx_t15: None,
                        outcome: None,
                        wallets_involved: involved_wallets,
                        whales_involved: involved_whales,
                    });
                } else if !queue.contains(&mint) {
                    queue.push_back(mint.clone());
                    println!("{}", format!("📥 QUEUED → {} (score {})", mint, score).yellow());
                }
            }

            println!("{}", "==============================".bright_black());
            println!("🪙 Mint: {}", mint.green());
            println!("⏱️ Age: {}s", age);
            println!("👥 Unique wallets(3m): {}", signers_now);
            println!("🧾 Tx count(3m): {}", tx_now);
            println!(
                "🐋 Beluga/Blue wallets(3m): {} / {}",
                whales_now.beluga_count, whales_now.blue_count
            );
            println!("📊 Score: {}", score);

            if let Some(fdv) = trend.last_fdv {
                println!("💰 FDV: ${}", fmt_f64_0_commas(fdv));
            }
            if let Some(liq) = trend.last_liq {
                println!("💧 Liq: ${}", fmt_f64_0_commas(liq));
            }
            if let Some(px) = trend.last_price {
                println!("💵 Price: ${:.8}", px);
            }
            if trend.price_up {
                println!("{}", "📈 Price rising (market confirm)".bright_green());
            }

            if wallet_growth_pct >= ACCEL_WALLET_GROWTH_PCT || tx_growth_pct >= ACCEL_TX_GROWTH_PCT {
                println!("{}", "🚀 ACCELERATING (hype building)".bold().bright_green());
            }
            if score >= SCORE_STRONG {
                println!("{}", "🔥 RUNNER".bold().bright_green());
            } else {
                println!("{}", "👀 WATCH".bold().cyan());
            }
        }

        // demote
        if c.active {
            if score < SCORE_DEMOTE {
                c.low_score_streak = c.low_score_streak.saturating_add(1);
            } else {
                c.low_score_streak = 0;
            }

            if c.low_score_streak >= DEMOTE_STREAK {
                c.active = false;
                c.low_score_streak = 0;
                active.retain(|m| m != &mint);
                println!("{}", format!("📉 DEMOTE → {} (score {})", mint, score).bold().yellow());

                while let Some(next) = queue.pop_front() {
                    if active.contains(&next) {
                        continue;
                    }
                    if let Some(nc) = coins.get_mut(&next) {
                        nc.active = true;
                        active.push(next.clone());
                        println!("{}", format!("🧠 FOCUS ADD → {} (from queue)", next).bold().green());

                        let involved_wallets = window_wallets(&nc.events);
                        let involved_whales = window_whales(&nc.events);
                        calls.push(CallRecord {
                            mint: next.clone(),
                            call_ts: now(),
                            score: SCORE_TARGET,
                            t5_ts: None,
                            wallets_t5: None,
                            tx_t5: None,
                            t15_ts: None,
                            wallets_t15: None,
                            tx_t15: None,
                            outcome: None,
                            wallets_involved: involved_wallets,
                            whales_involved: involved_whales,
                        });

                        break;
                    }
                }
            }
        }
    }
}

#[derive(Debug, Default, Clone)]
struct WhaleWindow {
    beluga_count: usize,
    blue_count: usize,
}

fn window_stats_for(events: &[Event], secs: u64) -> (usize, usize, WhaleWindow) {
    let cutoff = now().saturating_sub(secs);

    let mut uniq = HashSet::<&str>::new();
    let mut beluga = HashSet::<&str>::new();
    let mut blue = HashSet::<&str>::new();

    let mut tx = 0usize;

    for e in events.iter().rev() {
        if e.ts < cutoff {
            break;
        }
        tx += 1;
        uniq.insert(&e.wallet);

        match e.tier {
            WhaleTier::Blue => {
                blue.insert(&e.wallet);
            }
            WhaleTier::Beluga => {
                beluga.insert(&e.wallet);
            }
            WhaleTier::None => {}
        }
    }

    (
        tx,
        uniq.len(),
        WhaleWindow {
            beluga_count: beluga.len(),
            blue_count: blue.len(),
        },
    )
}

fn window_wallets(events: &[Event]) -> Vec<String> {
    let cutoff = now().saturating_sub(WINDOW_SECS);
    let mut uniq = HashSet::<String>::new();
    for e in events.iter().rev() {
        if e.ts < cutoff {
            break;
        }
        if e.wallet != "UNKNOWN" {
            uniq.insert(e.wallet.clone());
        }
    }
    uniq.into_iter().collect()
}

fn window_whales(events: &[Event]) -> Vec<String> {
    let cutoff = now().saturating_sub(WINDOW_SECS);
    let mut uniq = HashSet::<String>::new();
    for e in events.iter().rev() {
        if e.ts < cutoff {
            break;
        }
        if e.wallet != "UNKNOWN" && e.tier != WhaleTier::None {
            uniq.insert(e.wallet.clone());
        }
    }
    uniq.into_iter().collect()
}

fn prune_window(events: &mut Vec<Event>, window_secs: u64) {
    let cutoff = now().saturating_sub(window_secs);
    while events.first().map(|e| e.ts < cutoff).unwrap_or(false) {
        events.remove(0);
    }
}

fn pct_change(nowv: usize, prev: usize) -> f64 {
    if prev == 0 {
        if nowv == 0 { 0.0 } else { 1.0 }
    } else {
        (nowv as f64 - prev as f64) / (prev as f64)
    }
}

fn runner_score(
    signers_now: usize,
    tx_now: usize,
    signers_prev: usize,
    tx_prev: usize,
) -> (i32, f64, f64) {
    let wallet_growth_pct = pct_change(signers_now, signers_prev);
    let tx_growth_pct = pct_change(tx_now, tx_prev);

    let mut score: i32 = 0;

    // Breadth
    if signers_now >= 15 { score += 10; }
    if signers_now >= 25 { score += 15; }
    if signers_now >= 40 { score += 10; }
    if signers_now >= 60 { score += 10; }

    // Activity
    if tx_now >= 40 { score += 10; }
    if tx_now >= 60 { score += 10; }
    if tx_now >= 120 { score += 10; }

    // Acceleration
    if wallet_growth_pct >= 0.20 { score += 10; }
    if wallet_growth_pct >= 0.50 { score += 10; }
    if tx_growth_pct >= 0.30 { score += 10; }
    if tx_growth_pct >= 0.75 { score += 10; }

    (score, wallet_growth_pct, tx_growth_pct)
}

/* ================= RESOLVER ================= */

fn resolver_tick(
    coins: &HashMap<String, CoinState>,
    calls: &mut Vec<CallRecord>,
    wallets: &mut HashMap<String, WalletStats>,
    whales: &mut HashMap<String, WhalePerf>,
) {
    let now_ts = now();

    for call in calls.iter_mut() {
        if call.outcome.is_some() {
            continue;
        }

        let elapsed = now_ts.saturating_sub(call.call_ts);

        // baseline at ~5m (use last WINDOW_SECS at that moment)
        if call.t5_ts.is_none() && elapsed >= RESOLVE_T5_SECS {
            if let Some(c) = coins.get(&call.mint) {
                let (tx_now, signers_now, _) = window_stats_for(&c.events, WINDOW_SECS);
                call.t5_ts = Some(now_ts);
                call.wallets_t5 = Some(signers_now);
                call.tx_t5 = Some(tx_now);
            }
        }
        fn stats_since(events: &[Event], since_ts: u64) -> (usize, usize, WhaleWindow) {
         let mut uniq = HashSet::<&str>::new();
         let mut beluga = HashSet::<&str>::new();
         let mut blue = HashSet::<&str>::new();
         let mut tx = 0usize;

         for e in events.iter() {
             if e.ts < since_ts {
                 continue;
            }
            tx += 1;
            uniq.insert(&e.wallet);

            match e.tier {
              WhaleTier::Blue => { blue.insert(&e.wallet); }
              WhaleTier::Beluga => { beluga.insert(&e.wallet); }
              WhaleTier::None => {}
             }
        }

        (
             tx,
            uniq.len(),
            WhaleWindow { beluga_count: beluga.len(), blue_count: blue.len() },
        )
    }

        // finalize at ~15m
        if elapsed >= RESOLVE_T15_SECS {
            if let Some(c) = coins.get(&call.mint) {
                let (tx_now, signers_now, _) = stats_since(&c.events, call.call_ts);                call.t15_ts = Some(now_ts);
                call.wallets_t15 = Some(signers_now);
                call.tx_t15 = Some(tx_now);

                let w5 = call.wallets_t5.unwrap_or(0).max(1);
                let t5 = call.tx_t5.unwrap_or(0).max(1);

                let w_mult = (signers_now as f64) / (w5 as f64);
                let t_mult = (tx_now as f64) / (t5 as f64);

                let outcome = if w_mult >= WIN_WALLET_MULT || t_mult >= WIN_TX_MULT {
                    "WIN"
                } else if w_mult >= MID_WALLET_MULT || t_mult >= MID_TX_MULT {
                    "MID"
                } else {
                    "LOSS"
                };

                call.outcome = Some(outcome.to_string());

                match outcome {
                    "WIN" => {
                        println!(
                            "{}",
                            format!(
                                "✅ RESOLVED WIN: {}  (w {}→{} {:.2}x | tx {}→{} {:.2}x)",
                                call.mint, w5, signers_now, w_mult, t5, tx_now, t_mult
                            )
                            .bold()
                            .bright_green()
                        );
                    }
                    "MID" => {
                        println!(
                            "{}",
                            format!(
                                "➖ RESOLVED MID: {}  (w {}→{} {:.2}x | tx {}→{} {:.2}x)",
                                call.mint, w5, signers_now, w_mult, t5, tx_now, t_mult
                            )
                            .bright_black()
                        );
                    }
                    _ => {
                        println!(
                            "{}",
                            format!(
                                "❌ RESOLVED LOSS: {}  (w {}→{} {:.2}x | tx {}→{} {:.2}x)",
                                call.mint, w5, signers_now, w_mult, t5, tx_now, t_mult
                            )
                            .bold()
                            .red()
                        );
                    }
                }

                if outcome == "WIN" || outcome == "LOSS" {
                    for w in call.wallets_involved.iter() {
                        let ws = wallets.entry(w.clone()).or_default();
                        if outcome == "WIN" {
                            ws.wins = ws.wins.saturating_add(1);
                            ws.score += 6;
                        } else {
                            ws.losses = ws.losses.saturating_add(1);
                            ws.score -= 2;
                        }
                    }

                    for w in call.whales_involved.iter() {
                        let wp = whales.entry(w.clone()).or_default();
                        if outcome == "WIN" {
                            wp.wins = wp.wins.saturating_add(1);
                            wp.score += 1.0;
                        } else {
                            wp.losses = wp.losses.saturating_add(1);
                            wp.score -= 1.0;
                        }
                    }
                }
            } else {
                call.outcome = Some("LOSS".to_string());
            }
        }
    }
}

/* ================= PRINTING ================= */

fn print_wallet_stats(wallets: &HashMap<String, WalletStats>) {
    println!("{}", "=== WALLET STATS ===".bold().bright_white());
    println!("Wallets tracked: {}", fmt_i64_commas(wallets.len() as i64));
    let mut v: Vec<(&String, &WalletStats)> = wallets.iter().collect();
    v.sort_by(|a, b| {
        b.1.score
            .cmp(&a.1.score)
            .then_with(|| (b.1.wins + b.1.losses).cmp(&(a.1.wins + a.1.losses)))
    });

    println!("{}", "Top wallets (by score):".bright_black());
    for (i, (w, s)) in v.into_iter().take(15).enumerate() {
        let samples = (s.wins + s.losses).max(1);
        let winrate = (s.wins as f64) * 100.0 / (samples as f64);
        println!(
            " {:>2}. {} | score {} | W {} / L {} | samples {} | winrate {:.1}%",
            i + 1,
            w,
            s.score,
            s.wins,
            s.losses,
            samples,
            winrate
        );
    }
}

fn print_best_worst_calls(calls: &[CallRecord]) {
    let mut wins: Vec<&CallRecord> = calls
        .iter()
        .filter(|c| c.outcome.as_deref() == Some("WIN"))
        .collect();
    wins.sort_by(|a, b| b.score.cmp(&a.score));

    let mut losses: Vec<&CallRecord> = calls
        .iter()
        .filter(|c| c.outcome.as_deref() == Some("LOSS"))
        .collect();
    losses.sort_by(|a, b| b.score.cmp(&a.score));

    println!("{}", "🧾 BEST CALLS (WIN)".bold().bright_green());
    for c in wins.into_iter().take(8) {
        let wmult = if let (Some(a), Some(b)) = (c.wallets_t5, c.wallets_t15) {
            Some(format!("{:.2}x", (b as f64) / (a.max(1) as f64)))
        } else {
            None
        };
        let tmult = if let (Some(a), Some(b)) = (c.tx_t5, c.tx_t15) {
            Some(format!("{:.2}x", (b as f64) / (a.max(1) as f64)))
        } else {
            None
        };
        println!(
            " ✅ {} | score {} | w5→15 {:?} | tx5→15 {:?}",
            c.mint, c.score, wmult, tmult
        );
    }

    println!("{}", "🧨 WORST CALLS (high-score LOSS)".bold().red());
    for c in losses.into_iter().take(8) {
        println!(
            " ❌ {} | score {} | wallets {:?}→{:?} | tx {:?}→{:?}",
            c.mint, c.score, c.wallets_t5, c.wallets_t15, c.tx_t5, c.tx_t15
        );
    }
}

/* ================= WHALE HELPERS ================= */

fn classify_tier(sol_out: f64) -> WhaleTier {
    if sol_out >= BLUE_SOL_TX {
        WhaleTier::Blue
    } else if sol_out >= BELUGA_SOL_TX {
        WhaleTier::Beluga
    } else {
        WhaleTier::None
    }
}

fn estimate_sol_outflow(native: &[NativeTransfer], actor: &str) -> f64 {
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

fn collect_mints(tts: &[TokenTransfer]) -> Vec<String> {
    let mut mints: HashSet<String> = HashSet::new();
    for tt in tts {
        if let Some(m) = &tt.mint {
            if m == SOL_MINT || m == USDC_MINT {
                continue;
            }
            mints.insert(m.clone());
        }
    }
    mints.into_iter().collect()
}

/* ================= JSON IO ================= */

fn load_json<T: for<'de> Deserialize<'de> + Default>(path: &str) -> Option<T> {
    let Ok(s) = fs::read_to_string(path) else { return None; };
    serde_json::from_str(&s).ok().or(Some(T::default()))
}

fn save_json<T: Serialize>(path: &str, value: &T) -> std::io::Result<()> {
    let s = serde_json::to_string_pretty(value).unwrap_or_else(|_| "{}".to_string());
    fs::write(path, s)
}

/* ================= HELIUS FETCH ================= */

async fn fetch_latest_program_txs(client: &Client) -> Result<Vec<HeliusTx>, ()> {
    let url = format!(
        "{}/{}/transactions?api-key={}&limit={}",
        HELIUS_ADDR_URL,
        PUMP_FUN_PROGRAM,
        HELIUS_API_KEY,
        FETCH_LIMIT
    );

    let res = client.get(url).send().await.map_err(|_| ())?;
    res.json::<Vec<HeliusTx>>().await.map_err(|_| ())
}

/* ================= TIME ================= */

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
}