use crate::fmt::fmt_i64_commas;
use crate::types::{CallRecord, WalletStats};
use colored::*;
use std::collections::HashMap;

pub fn print_wallet_stats(wallets: &HashMap<String, WalletStats>) {
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
            i + 1, w, s.score, s.wins, s.losses, samples, winrate
        );
    }
}

pub fn print_best_worst_calls(calls: &[CallRecord]) {
    let mut wins: Vec<&CallRecord> = calls.iter().filter(|c| c.outcome.as_deref() == Some("WIN")).collect();
    wins.sort_by(|a, b| b.score.cmp(&a.score));

    let mut losses: Vec<&CallRecord> = calls.iter().filter(|c| c.outcome.as_deref() == Some("LOSS")).collect();
    losses.sort_by(|a, b| b.score.cmp(&a.score));

    println!("{}", "🧾 BEST CALLS (WIN)".bold().bright_green());
    for c in wins.into_iter().take(8) {
        println!(" ✅ {} | score {}", c.mint, c.score);
    }

    println!("{}", "🧨 WORST CALLS (LOSS)".bold().red());
    for c in losses.into_iter().take(8) {
        println!(" ❌ {} | score {}", c.mint, c.score);
    }
}

pub fn print_resolve(
    call: &CallRecord,
    w5: usize,
    w15: usize,
    w_mult: f64,
    t5: usize,
    t15: usize,
    t_mult: f64,
    outcome: &str,
) {
    match outcome {
        "WIN" => println!("{}", format!("✅ RESOLVED WIN: {} (w {}→{} {:.2}x | tx {}→{} {:.2}x)", call.mint, w5, w15, w_mult, t5, t15, t_mult).bold().bright_green()),
        "MID" => println!("{}", format!("➖ RESOLVED MID: {} (w {}→{} {:.2}x | tx {}→{} {:.2}x)", call.mint, w5, w15, w_mult, t5, t15, t_mult).bright_black()),
        _ => println!("{}", format!("❌ RESOLVED LOSS: {} (w {}→{} {:.2}x | tx {}→{} {:.2}x)", call.mint, w5, w15, w_mult, t5, t15, t_mult).bold().red()),
    }
}