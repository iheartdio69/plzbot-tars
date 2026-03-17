// src/resolver.rs
use crate::config::Config;
use crate::db::Db;
use crate::market::cache::MarketCache;
use anyhow::Result;

pub fn resolve_calls(_cfg: &Config, db: &mut Db, market: &MarketCache, now_ts: i64) -> Result<()> {
    let inserted = crate::scoring::call_outcomes::evaluate_call_outcomes(db, market, now_ts)?;

    if inserted > 0 {
        // optional: also update wallet outcomes here if you want it tied to grading
        let _ = crate::scoring::wallet_outcomes::update_from_call_outcomes(db, now_ts);

        // perf printouts
        let life = db.bot_perf_lifetime()?;
        let last50 = db.bot_perf_last_n(50)?;
        let last10 = db.bot_perf_last_n(10)?;

        eprintln!(
            "{}",
            crate::fmt::perf_line(
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
            crate::fmt::perf_line(
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
            crate::fmt::perf_line(
                "last10",
                last10.total,
                last10.wins,
                last10.losses,
                last10.win_rate,
                last10.avg_mult
            )
        );
    }

    Ok(())
}
