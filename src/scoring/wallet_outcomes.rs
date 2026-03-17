use crate::db::Db;
use anyhow::Result;

/// Attribute graded call outcomes to wallets (based on call_top_wallets).
/// Safe to run every tick; it only processes calls newer than last seen.
pub fn update_from_call_outcomes(db: &mut Db, _now_ts: i64) -> Result<()> {
    let last_done_call_ts = db.wallet_outcomes_last_call_ts()?;

    // Pull newly-graded calls
    let graded = db.graded_calls_since(last_done_call_ts, 500)?;

    for (mint, call_ts, outcome_ts, peak_fdv, result, call_fdv) in graded {
        // Attribute to wallets we recorded at call time
        let wallets = db.wallets_for_call(&mint, call_ts, 25)?;

        if wallets.is_empty() {
            continue;
        }

        // We'll use this to bump watchlist W/L
        let mut wallet_ids: Vec<String> = Vec::with_capacity(wallets.len());

        for (wallet, edges) in wallets {
            wallet_ids.push(wallet.clone());

            db.insert_wallet_outcome(
                &wallet, &mint, call_ts, outcome_ts, &result, call_fdv, peak_fdv, edges,
            )?;
        }

        // --- also attribute this call outcome to watchlist_wallets (W/L) ---
        let is_win = result == "winner";
        let bumped = db
            .bump_watchlist_wl_for_wallets(&wallet_ids, is_win)
            .unwrap_or(0);

        if bumped > 0 {
            eprintln!(
                "DBG watchlist wl bump: mint={} call_ts={} result={} wallets_updated={}",
                mint, call_ts, result, bumped
            );
        }
    }

    Ok(())
}
