use crate::db::Db;
use crate::market::cache::MarketCache;
use anyhow::Result;

pub fn evaluate_call_outcomes(db: &mut Db, _market: &MarketCache, now_ts: i64) -> Result<i64> {
    eprintln!("DBG outcomes: now_ts={}", now_ts);

    // settings (tune later)
    let outcome_delay_sec: i64 = 60 * 20; // wait 20m before grading
    let peak_window_sec: i64 = 60 * 60; // look 60m after call
    let win_mult: f64 = 1.4; // win if peak >= call_fdv * 1.4
    let give_up_sec: i64 = 60 * 60 * 6; // give up 6h after call if no snapshot data

    // only grade calls older than delay
    let older_than_ts = now_ts - outcome_delay_sec;

    // pull a small batch per tick so we don't stall the loop
    let pending = db.calls_missing_outcomes(older_than_ts, 200)?;
    eprintln!("DBG outcomes: pending={}", pending.len());

    let mut inserted_total: i64 = 0;

    for (mint, call_ts, call_fdv) in pending {
        let start = call_ts;
        let end = call_ts + peak_window_sec;
        let give_up_ts = call_ts + give_up_sec;

        let peak_opt = db.peak_fdv_for_mint_window(&mint, start, end)?;

        let Some(peak_fdv) = peak_opt else {
            // no data yet in snapshots for that window
            if now_ts >= give_up_ts {
                // close it out as loss so it stops clogging the queue
                inserted_total +=
                    db.insert_call_outcome(&mint, call_ts, end, call_fdv.max(0.0), "loss")? as i64;
            }
            continue;
        };

        let result = if call_fdv > 0.0 && peak_fdv >= call_fdv * win_mult {
            "win"
        } else {
            "loss"
        };

        inserted_total += db.insert_call_outcome(&mint, call_ts, end, peak_fdv, result)? as i64;
    }

    Ok(inserted_total)
}
