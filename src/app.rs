use crate::config::*;
use crate::helius::fetch_latest_program_txs;
use crate::io::{load_json, load_usage, save_json, save_usage};
use crate::market::{cache_push, fetch_dexscreener_snap};
use crate::printing::{print_best_worst_calls, print_wallet_stats};
use crate::resolver::resolver_tick;
use crate::scoring::{prune_window, score_and_manage};
use crate::time::{day_number_now, now};
use crate::types::*;
use colored::*;
use reqwest::Client;
use std::collections::{HashMap, HashSet, VecDeque};
use std::time::{Duration, Instant};
use tokio::time::sleep;

fn classify_tier(sol_out: f64) -> WhaleTier {
    if sol_out >= BLUE_SOL_TX { WhaleTier::Blue }
    else if sol_out >= BELUGA_SOL_TX { WhaleTier::Beluga }
    else { WhaleTier::None }
}

fn estimate_sol_outflow(native: &[NativeTransfer], actor: &str) -> f64 {
    if actor == "UNKNOWN" { return 0.0; }
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
            if m == SOL_MINT || m == USDC_MINT { continue; }
            mints.insert(m.clone());
        }
    }
    mints.into_iter().collect()
}

pub async fn run() {
    println!("{}", "🧠 Ishmael Runner Radar (modular rebuild)".bold().green());

    let client = Client::new();

    let mut coins: HashMap<String, CoinState> = HashMap::new();
    let mut active: Vec<String> = vec![];
    let mut queue: VecDeque<String> = VecDeque::new();

    let mut seen_sigs: VecDeque<String> = VecDeque::new();
    let mut seen_set: HashSet<String> = HashSet::new();

    let mut wallets: HashMap<String, WalletStats> = load_json("wallets.json");
    let mut whales: HashMap<String, WhalePerf> = load_json("whales.json");
    let mut calls: Vec<CallRecord> = load_json("calls.json");

    let usage_path = "usage.json";
    let mut usage = load_usage(usage_path);

    let mut market: MarketCache = HashMap::new();
    let mut last_market_poll = Instant::now().checked_sub(Duration::from_secs(999)).unwrap_or(Instant::now());

    let mut last_save = Instant::now().checked_sub(Duration::from_secs(999)).unwrap_or(Instant::now());
    let mut last_print_top = Instant::now().checked_sub(Duration::from_secs(999)).unwrap_or(Instant::now());
    let mut last_resolve_tick = Instant::now().checked_sub(Duration::from_secs(999)).unwrap_or(Instant::now());

    loop {
        // day reset
        let today = day_number_now();
        if usage.day != today {
            usage.day = today;
            usage.requests = 0;
            save_usage(usage_path, &usage);
        }

        // cap check
        if usage.requests >= DAILY_CAP {
            println!("{}", format!("🛑 DAILY CAP HIT: {}/{}. Sleeping 60s…", usage.requests, DAILY_CAP).bold().yellow());
            save_usage(usage_path, &usage);
            sleep(Duration::from_secs(60)).await;
            continue;
        }

        // market poll (free)
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

        // helius fetch (costs)
        usage.requests = usage.requests.saturating_add(1);
        save_usage(usage_path, &usage);

        match fetch_latest_program_txs(&client).await {
            Ok(txs) => {
                let fetched = txs.len();
                let mut new_count = 0usize;

                for tx in txs {
                    if seen_set.contains(&tx.signature) { continue; }
                    new_count += 1;

                    seen_set.insert(tx.signature.clone());
                    seen_sigs.push_back(tx.signature.clone());
                    while seen_sigs.len() > SEEN_SIG_CAP {
                        if let Some(old) = seen_sigs.pop_front() {
                            seen_set.remove(&old);
                        }
                    }

                    let actor = tx.fee_payer.clone().unwrap_or_else(|| "UNKNOWN".to_string());
                    let ts = tx.timestamp;

                    let sol_out = estimate_sol_outflow(&tx.native_transfers, &actor);
                    let tier = classify_tier(sol_out);

                    let mints = collect_mints(&tx.token_transfers);
                    if mints.is_empty() { continue; }

                    let sol_per_mint = if sol_out > 0.0 { sol_out / (mints.len() as f64) } else { 0.0 };

                    if actor != "UNKNOWN" && tier != WhaleTier::None {
                        let wp = whales.entry(actor.clone()).or_default();
                        wp.last_seen_ts = ts;
                        wp.seen = wp.seen.saturating_add(1);
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
                        entry.events.push(Event { wallet: actor.clone(), ts, sol: sol_per_mint, tier });
                        prune_window(&mut entry.events, EVENTS_KEEP_SECS);
                    }
                }

                println!("{}", format!("📡 fetched {} txs | new {} | used today {}/{}", fetched, new_count, usage.requests, DAILY_CAP).bright_black());

                score_and_manage(&mut coins, &mut active, &mut queue, &mut calls, &market);

                if last_resolve_tick.elapsed().as_secs() >= RESOLVE_CHECK_EVERY_SECS {
                    resolver_tick(&coins, &mut calls, &mut wallets, &mut whales);
                    last_resolve_tick = Instant::now();
                }

                if last_print_top.elapsed().as_secs() >= PRINT_TOP_WALLETS_EVERY_SECS {
                    print_wallet_stats(&wallets);
                    print_best_worst_calls(&calls);
                    last_print_top = Instant::now();
                }

                if last_save.elapsed().as_secs() >= SAVE_EVERY_SECS {
                    save_json("wallets.json", &wallets);
                    save_json("whales.json", &whales);
                    save_json("calls.json", &calls);
                    last_save = Instant::now();
                }
            }
            Err(_) => println!("{}", "⚠️ fetch failed (network/rate-limit). retrying…".yellow()),
        }

        sleep(Duration::from_secs(LOOP_SLEEP_SECS)).await;
    }
}