use crate::db::Db;
use anyhow::Result;

/// Update wallet reputation scores based on wallet_edges since last run.
///
/// Call this once per tick (or every N ticks).
pub fn update_wallet_reputation(db: &mut Db, now_ts: i64) -> Result<()> {
    // last time we scored up to
    let last = db.wallet_scoring_last_ts()?;

    // If never ran before, only score last ~10 minutes to avoid giant backlog spikes.
    let since = if last <= 0 {
        now_ts.saturating_sub(600)
    } else {
        last
    };

    // Nothing to do
    if now_ts <= since {
        return Ok(());
    }

    let deltas = db.wallet_scoring_compute_deltas(since, now_ts)?;

    for (wallet, delta) in deltas {
        db.wallet_score_smooth_update(&wallet, delta, now_ts)?;
    }

    db.wallet_scoring_set_last_ts(now_ts)?;
    Ok(())
}
