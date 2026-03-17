use crate::config::Config;
use crate::db::Db;
use crate::governor::Governor;
use crate::types::CoinState;
use std::collections::HashMap;
use std::sync::Arc;

/// Fetch onchain events for tracked pair addresses.
/// - Governed via Governor
/// - `fetch_address_txs` does the real ingestion (writes DB + updates CoinState.events)
/// - After ingestion, we "refresh" CoinState.last_activity_ts from DB heartbeat
/// - Returns newly discovered mints
pub async fn fetch_onchain_events(
    cfg: &Config,
    db: &mut Db,
    coins: &mut HashMap<String, CoinState>,
    tracked_pairs: &[String],
    gov: Arc<Governor>,
) -> Vec<String> {
    if tracked_pairs.is_empty() {
        return Vec::new();
    }

    // 1) Ingest (this is where events/edges get written)
    let discovered_mints =
        crate::helius::client::fetch_address_txs(cfg, db, coins, tracked_pairs, gov).await;

    // 2) Refresh per-mint "last activity" for actives/rotator logic
    // IMPORTANT: last_activity_ts should mean "real onchain activity", not "we polled".
    // We derive it from DB (events/sigs or snapshots), so it stays consistent.
    let now_u64: u64 = crate::time::now();
    let now_ts: i64 = now_u64 as i64;

    // Update activity for:
    // - any newly discovered mint
    // - any existing mint whose pair we are tracking this tick
    //
    // NOTE: we can't map pair->mint here without extra bookkeeping, so we update only
    // the discovered mints (most important). If you want perfect updates for *all*
    // tracked mints, see the note below.
    for mint in discovered_mints.iter() {
        // Prefer sig/event-based heartbeat if you have it; otherwise snapshot heartbeat.
        let mut last_seen: i64 = db
            .mint_last_seen_ts(mint.as_str())
            .ok()
            .flatten()
            .unwrap_or(0);

        // Optional: if you also track snapshots as heartbeat, take the max.
        let last_snap: i64 = db
            .mint_last_seen_ts(mint.as_str())
            .ok()
            .flatten()
            .unwrap_or(0);

        last_seen = last_seen.max(last_snap);

        // clamp bad/future timestamps
        if last_seen > now_ts {
            last_seen = now_ts;
        }

        if last_seen > 0 {
            if let Some(st) = coins.get_mut(mint) {
                st.last_activity_ts = last_seen as u64;
            }
        }
    }

    discovered_mints
}
