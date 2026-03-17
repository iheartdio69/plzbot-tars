// src/printing.rs
use crate::types::CallRecord;
use crate::{db::Db, fmt, market::cache::MarketCache};
use anyhow::Result;

/// Minimal call print (works with current CallRecord: mint/score/ts).
pub fn print_call(c: &CallRecord) {
    // Keep this cheap + always valid:
    // - mint is mint-green bold
    // - score uses score bands
    println!(
        "{} {} score={} ts={}",
        fmt::pink("📣 CALL:"),
        fmt::mint(c.mint.as_str()),
        fmt::score_fmt(c.score),
        c.ts
    );
}

/// Rich call print (use from process_calls where you already have these values).
/// `tags` is like "GAMBOL|WPLUS" or "REVIVE|RUNNER|WNEG".
pub fn print_call_rich(
    mint: &str,
    fdv: f64,
    score: i32,
    tx_5m: u64,
    signers: usize,
    events: usize,
    tags: &str,
) {
    let line = if tags.is_empty() {
        fmt::call_line(mint, fdv, score, tx_5m, signers, events)
    } else {
        fmt::call_line_tagged(mint, fdv, score, tx_5m, signers, events, tags)
    };

    println!("{line}");
    println!(
        "   🕒 local={}",
        chrono::Local::now().format("%-I:%M:%S %p")
    );
}

/// Throttled "tick":
/// - grade call outcomes
/// - update wallet outcomes
/// - print bot perf (only if new grading happened)
pub fn tick_outcomes_and_perf(
    db: &mut Db,
    market: &MarketCache,
    last_eval_ts: &mut i64,
    now_ts: i64,
    every_secs: i64,
) -> Result<()> {
    if now_ts - *last_eval_ts < every_secs {
        return Ok(());
    }
    *last_eval_ts = now_ts;

    let inserted = match crate::scoring::call_outcomes::evaluate_call_outcomes(db, market, now_ts) {
        Ok(n) => n,
        Err(e) => {
            eprintln!("{}DBG outcomes eval ERR={:?}{}", fmt::RED, e, fmt::RESET);
            0
        }
    };

    if let Err(e) = crate::scoring::wallet_outcomes::update_from_call_outcomes(db, now_ts) {
        eprintln!("{}DBG wallet_outcomes ERR={:?}{}", fmt::RED, e, fmt::RESET);
    }

    // Only print perf when we graded something new
    if inserted > 0 {
        match db.bot_perf_lifetime() {
            Ok(life) => {
                let last50 = db.bot_perf_last_n(50).unwrap_or_else(|_| life.clone());
                let last10 = db.bot_perf_last_n(10).unwrap_or_else(|_| life.clone());

                eprintln!(
                    "{}",
                    fmt::perf_line(
                        "lifetime",
                        life.total,
                        life.wins,
                        life.losses,
                        life.win_rate,
                        life.avg_mult
                    )
                );
                eprintln!(
                    "{}",
                    fmt::perf_line(
                        "last50",
                        last50.total,
                        last50.wins,
                        last50.losses,
                        last50.win_rate,
                        last50.avg_mult
                    )
                );
                eprintln!(
                    "{}",
                    fmt::perf_line(
                        "last10",
                        last10.total,
                        last10.wins,
                        last10.losses,
                        last10.win_rate,
                        last10.avg_mult
                    )
                );
            }
            Err(e) => eprintln!("{}DBG perf ERR={:?}{}", fmt::RED, e, fmt::RESET),
        }
    }

    Ok(())
}
