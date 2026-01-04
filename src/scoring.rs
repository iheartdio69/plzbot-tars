use crate::config::*;
use crate::market::market_trend;
use crate::time::now;
use crate::types::*;
use colored::*;
use std::collections::{HashMap, HashSet, VecDeque};
use std::time::Instant;

pub fn score_and_manage(
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

        if age < MIN_AGE_SECS {
            c.prev_tx_window = tx_now;
            c.prev_signers_window = signers_now;
            continue;
        }

        let (mut score, wallet_growth_pct, tx_growth_pct) =
            runner_score(signers_now, tx_now, c.prev_signers_window, c.prev_tx_window);

        c.prev_tx_window = tx_now;
        c.prev_signers_window = signers_now;

        if whales_now.beluga_count >= 1 { score += 10; }
        if whales_now.blue_count >= 1 { score += 15; }
        if whales_now.blue_count >= 2 { score += 10; }

        // market boosts
        let trend = market_trend(market, &mint);
        if trend.fdv_ok && trend.liq_ok { score += FDV_OK_BOOST; }
        if trend.price_up { score += PRICE_UP_BOOST; }

        let passes =
            score >= SCORE_TARGET && signers_now >= MIN_SIGNERS_FOR_TARGET && tx_now >= MIN_TX_FOR_TARGET;

        if passes {
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
            println!("🐋 Beluga/Blue wallets(3m): {} / {}", whales_now.beluga_count, whales_now.blue_count);
            println!("📊 Score: {}", score);

            if wallet_growth_pct >= ACCEL_WALLET_GROWTH_PCT || tx_growth_pct >= ACCEL_TX_GROWTH_PCT {
                println!("{}", "🚀 ACCELERATING (hype building)".bold().bright_green());
            }
            if score >= SCORE_STRONG {
                println!("{}", "🔥 RUNNER".bold().bright_green());
            } else {
                println!("{}", "👀 WATCH".bold().cyan());
            }
        }

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
                    if active.contains(&next) { continue; }
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

pub fn window_stats_for(events: &[Event], secs: u64) -> (usize, usize, WhaleWindow) {
    let cutoff = crate::time::now().saturating_sub(secs);

    let mut uniq = HashSet::<&str>::new();
    let mut beluga = HashSet::<&str>::new();
    let mut blue = HashSet::<&str>::new();

    let mut tx = 0usize;

    for e in events.iter().rev() {
        if e.ts < cutoff { break; }
        tx += 1;
        uniq.insert(&e.wallet);
        match e.tier {
            WhaleTier::Blue => { blue.insert(&e.wallet); }
            WhaleTier::Beluga => { beluga.insert(&e.wallet); }
            WhaleTier::None => {}
        }
    }

    (tx, uniq.len(), WhaleWindow { beluga_count: beluga.len(), blue_count: blue.len() })
}

pub fn window_wallets(events: &[Event]) -> Vec<String> {
    let cutoff = crate::time::now().saturating_sub(WINDOW_SECS);
    let mut uniq = HashSet::<String>::new();
    for e in events.iter().rev() {
        if e.ts < cutoff { break; }
        if e.wallet != "UNKNOWN" { uniq.insert(e.wallet.clone()); }
    }
    uniq.into_iter().collect()
}

pub fn window_whales(events: &[Event]) -> Vec<String> {
    let cutoff = crate::time::now().saturating_sub(WINDOW_SECS);
    let mut uniq = HashSet::<String>::new();
    for e in events.iter().rev() {
        if e.ts < cutoff { break; }
        if e.wallet != "UNKNOWN" && e.tier != WhaleTier::None {
            uniq.insert(e.wallet.clone());
        }
    }
    uniq.into_iter().collect()
}

pub fn prune_window(events: &mut Vec<Event>, keep_secs: u64) {
    let cutoff = crate::time::now().saturating_sub(keep_secs);
    while events.first().map(|e| e.ts < cutoff).unwrap_or(false) {
        events.remove(0);
    }
}

pub fn runner_score(signers_now: usize, tx_now: usize, signers_prev: usize, tx_prev: usize) -> (i32, f64, f64) {
    let wallet_growth_pct = pct_change(signers_now, signers_prev);
    let tx_growth_pct = pct_change(tx_now, tx_prev);

    let mut score: i32 = 0;
    if signers_now >= 15 { score += 10; }
    if signers_now >= 25 { score += 15; }
    if signers_now >= 40 { score += 10; }
    if signers_now >= 60 { score += 10; }
    if tx_now >= 40 { score += 10; }
    if tx_now >= 60 { score += 10; }
    if tx_now >= 120 { score += 10; }
    if wallet_growth_pct >= 0.20 { score += 10; }
    if wallet_growth_pct >= 0.50 { score += 10; }
    if tx_growth_pct >= 0.30 { score += 10; }
    if tx_growth_pct >= 0.75 { score += 10; }

    (score, wallet_growth_pct, tx_growth_pct)
}

fn pct_change(nowv: usize, prev: usize) -> f64 {
    if prev == 0 { if nowv == 0 { 0.0 } else { 1.0 } }
    else { (nowv as f64 - prev as f64) / (prev as f64) }
}