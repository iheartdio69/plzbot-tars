use crate::config::*;
use crate::printing::print_resolve;
use crate::types::*;
use std::collections::HashMap;

fn stats_since(events: &[Event], since_ts: u64) -> (usize, usize, WhaleWindow) {
    use std::collections::HashSet;

    let mut uniq = HashSet::<&str>::new();
    let mut beluga = HashSet::<&str>::new();
    let mut blue = HashSet::<&str>::new();
    let mut tx = 0usize;

    for e in events.iter() {
        if e.ts < since_ts { continue; }
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

pub fn resolver_tick(
    coins: &HashMap<String, CoinState>,
    calls: &mut Vec<CallRecord>,
    wallets: &mut HashMap<String, WalletStats>,
    whales: &mut HashMap<String, WhalePerf>,
) {
    let now_ts = crate::time::now();

    for call in calls.iter_mut() {
        if call.outcome.is_some() { continue; }

        let elapsed = now_ts.saturating_sub(call.call_ts);

        // baseline at ~5m since call
        if call.t5_ts.is_none() && elapsed >= RESOLVE_T5_SECS {
            if let Some(c) = coins.get(&call.mint) {
                let (tx_now, signers_now, _) = stats_since(&c.events, call.call_ts);
                call.t5_ts = Some(now_ts);
                call.wallets_t5 = Some(signers_now);
                call.tx_t5 = Some(tx_now);
            }
        }

        // finalize at ~15m since call
        if elapsed >= RESOLVE_T15_SECS {
            if let Some(c) = coins.get(&call.mint) {
                let (tx_now, signers_now, _) = stats_since(&c.events, call.call_ts);
                call.t15_ts = Some(now_ts);
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
                print_resolve(call, w5, signers_now, w_mult, t5, tx_now, t_mult, outcome);

                if outcome == "WIN" || outcome == "LOSS" {
                    for w in call.wallets_involved.iter() {
                        let ws = wallets.entry(w.clone()).or_default();
                        if outcome == "WIN" { ws.wins += 1; ws.score += 6; }
                        else { ws.losses += 1; ws.score -= 2; }
                    }
                    for w in call.whales_involved.iter() {
                        let wp = whales.entry(w.clone()).or_default();
                        if outcome == "WIN" { wp.wins += 1; wp.score += 1.0; }
                        else { wp.losses += 1; wp.score -= 1.0; }
                    }
                }
            } else {
                call.outcome = Some("LOSS".to_string());
            }
        }
    }
}