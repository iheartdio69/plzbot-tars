use crate::config::Config;
use crate::db::Db;
use crate::governor::Governor;
use crate::types::{CoinState, Event, WhaleTier};
use anyhow::Result;
use reqwest::Client;
use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct HeliusTx {
    pub signature: Option<String>,
    pub timestamp: Option<u64>,
    pub fee_payer: Option<String>,

    #[serde(default)]
    pub native_transfers: Vec<NativeTransfer>,
    #[serde(default)]
    pub token_transfers: Vec<TokenTransfer>,

    #[serde(default)]
    pub transaction_error: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct NativeTransfer {
    pub from_user_account: Option<String>,
    pub to_user_account: Option<String>,
    pub amount: u64, // lamports
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct TokenTransfer {
    #[serde(default)]
    pub mint: String,
    pub from_user_account: Option<String>,
    pub to_user_account: Option<String>,
    pub token_amount: Option<f64>,
}

fn lamports_to_sol(l: u64) -> f64 {
    (l as f64) / 1_000_000_000.0
}

// ---------------- helpers shared with per_coin.rs ----------------

#[inline]
pub(crate) fn is_ignored_mint(m: &str) -> bool {
    matches!(
        m,
        "So11111111111111111111111111111111111111112" | // wSOL
        "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v" | // USDC
        "Es9vMFrzaCERmJfrF4H2FYD4KCoNkY11McCe8BenwNYB" // USDT
    )
}

// Blue > Beluga (Blue threshold higher than Beluga)
pub(crate) fn classify_tier(sol: f64, cfg: &Config) -> WhaleTier {
    if sol >= cfg.blue_sol_tx {
        WhaleTier::Blue
    } else if sol >= cfg.beluga_sol_tx {
        WhaleTier::Beluga
    } else {
        WhaleTier::None
    }
}

pub(crate) fn estimate_sol_out(native: &[NativeTransfer], actor: &str) -> f64 {
    let mut lamports_out: u64 = 0;
    for nt in native {
        if nt.from_user_account.as_deref().unwrap_or("") == actor {
            lamports_out = lamports_out.saturating_add(nt.amount);
        }
    }
    lamports_to_sol(lamports_out)
}

pub(crate) fn estimate_sol_in(native: &[NativeTransfer], actor: &str) -> f64 {
    let mut lamports_in: u64 = 0;
    for nt in native {
        if nt.to_user_account.as_deref().unwrap_or("") == actor {
            lamports_in = lamports_in.saturating_add(nt.amount);
        }
    }
    lamports_to_sol(lamports_in)
}

pub(crate) fn estimate_sol_mag(native: &[NativeTransfer], actor: &str) -> f64 {
    let out = estimate_sol_out(native, actor);
    let inp = estimate_sol_in(native, actor);
    out.abs().max(inp.abs())
}

pub(crate) fn tx_signers(tx: &HeliusTx, fee_payer: &str) -> HashSet<String> {
    let mut signers: HashSet<String> = HashSet::new();
    if !fee_payer.trim().is_empty() {
        signers.insert(fee_payer.to_string());
    }

    for nt in &tx.native_transfers {
        if let Some(f) = &nt.from_user_account {
            if !f.is_empty() {
                signers.insert(f.clone());
            }
        }
        if let Some(t) = &nt.to_user_account {
            if !t.is_empty() {
                signers.insert(t.clone());
            }
        }
    }

    for tt in &tx.token_transfers {
        if let Some(f) = &tt.from_user_account {
            if !f.is_empty() {
                signers.insert(f.clone());
            }
        }
        if let Some(t) = &tt.to_user_account {
            if !t.is_empty() {
                signers.insert(t.clone());
            }
        }
    }

    signers
}

fn collect_mints(token: &[TokenTransfer]) -> Vec<String> {
    let mut set = HashSet::new();
    for tt in token {
        let m = tt.mint.trim();
        if m.is_empty() || is_ignored_mint(m) {
            continue;
        }
        set.insert(m.to_string());
    }
    set.into_iter().collect()
}

// ---------------- main ingest ----------------

/// Fetch transactions for a single wallet from Helius.
/// Handles 429 retry and per-request timeout. Returns Err on any failure.
async fn fetch_wallet_transactions(
    cfg: &Config,
    gov: &Arc<Governor>,
    wallet: &str,
) -> Result<Vec<HeliusTx>> {
    let client = Client::new();

    let raw = cfg.helius_addr_url.trim().trim_end_matches('/');
    let base = if raw.ends_with("/v0/addresses") {
        raw.to_string()
    } else {
        format!("{}/v0/addresses", raw)
    };

    let url = format!(
        "{}/{}/transactions?api-key={}&limit={}",
        base,
        wallet,
        cfg.helius_api_key.trim(),
        cfg.fetch_limit
    );

    let permit = gov.acquire_enhanced().await;

    // ---------- first attempt ----------
    let mut resp = match tokio::time::timeout(
        std::time::Duration::from_secs(5),
        client.get(&url).send(),
    ).await {
        Ok(Ok(r)) => r,
        Ok(Err(e)) => return Err(anyhow::anyhow!("request error: {}", e)),
        Err(_) => return Err(anyhow::anyhow!("timeout")),
    };

    // ---------- 429 handling (one retry) ----------
    if resp.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
        let body = resp.text().await.unwrap_or_default();
        eprintln!(
            "DBG helius ingest: 429 wallet={} body_snip={}",
            wallet,
            body.chars().take(200).collect::<String>()
        );
        gov.on_429_enhanced().await;
        drop(permit);
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;

        let permit2 = gov.acquire_enhanced().await;
        resp = match tokio::time::timeout(
            std::time::Duration::from_secs(5),
            client.get(&url).send(),
        ).await {
            Ok(Ok(r)) => r,
            Ok(Err(e)) => return Err(anyhow::anyhow!("retry error: {}", e)),
            Err(_) => return Err(anyhow::anyhow!("retry timeout")),
        };

        if resp.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
            gov.on_429_enhanced().await;
            return Err(anyhow::anyhow!("429 after retry"));
        }
        if !resp.status().is_success() {
            return Err(anyhow::anyhow!("non-200 after retry: {}", resp.status()));
        }
        gov.on_success(permit2.lane()).await;
    } else {
        if !resp.status().is_success() {
            return Err(anyhow::anyhow!("non-200: {}", resp.status()));
        }
        gov.on_success(permit.lane()).await;
    }

    let txs: Vec<HeliusTx> = resp.json().await
        .map_err(|e| anyhow::anyhow!("json error: {}", e))?;

    Ok(txs)
}

/// Process a batch of fetched transactions into coins/db for a single wallet.
fn process_wallet_transactions(
    db: &mut Db,
    coins: &mut HashMap<String, CoinState>,
    cfg: &Config,
    wallet: &str,
    txs: Vec<HeliusTx>,
) -> Result<()> {
    for tx in txs {
        if tx.transaction_error.is_some() {
            continue;
        }

        let sig = match tx.signature.as_deref() {
            Some(s) if !s.is_empty() => s,
            _ => continue,
        };

        let ts = tx.timestamp.unwrap_or(0);
        if ts == 0 {
            continue;
        }

        if db.seen_sig(sig)? {
            continue;
        }
        db.mark_sig(ts as i64, sig)?;

        let fee_payer = tx.fee_payer.clone().unwrap_or_else(|| wallet.to_string());
        let sol_mag = estimate_sol_mag(&tx.native_transfers, &fee_payer);
        let signers = tx_signers(&tx, &fee_payer);

        // Dust ignore
        if sol_mag < 0.01 && signers.len() <= 1 {
            continue;
        }

        let tier = classify_tier(sol_mag, cfg);
        let mints = collect_mints(&tx.token_transfers);
        if mints.is_empty() {
            continue;
        }

        for mint in mints {
            coins.entry(mint.clone()).or_insert_with(CoinState::new);

            if let Some(st) = coins.get_mut(&mint) {
                if ts > 0 && (st.first_seen == 0 || ts < st.first_seen) {
                    st.first_seen = ts;
                }
                for w in signers.iter() {
                    st.events.push(Event { wallet: w.clone(), ts, sol: sol_mag, tier });
                }
                st.last_activity_ts = st.last_activity_ts.max(ts);
                if st.events.len() > 50_000 {
                    st.events.drain(0..10_000);
                }
            }

            for w in signers.iter() {
                let _ = db.insert_wallet_edge(
                    ts as i64, w, None, Some(mint.as_str()),
                    "helius_tx", Some(sol_mag), Some(sig),
                );
            }

            for tt in &tx.token_transfers {
                let tm = tt.mint.as_str().trim();
                if tm.is_empty() || is_ignored_mint(tm) { continue; }
                let from = tt.from_user_account.as_deref().unwrap_or("").trim();
                let to   = tt.to_user_account.as_deref().unwrap_or("").trim();
                if from.is_empty() || to.is_empty() { continue; }
                let _ = db.insert_wallet_edge(
                    ts as i64, from, Some(to), Some(tm),
                    "token_transfer", None, Some(sig),
                );
            }
        }
    }
    Ok(())
}

/// Wallet-driven ingest — fetches all wallets concurrently, processes results sequentially.
pub async fn ingest_wallet_activity(
    cfg: &Config,
    db: &mut Db,
    coins: &mut HashMap<String, CoinState>,
    wallets: &[String],
    gov: Arc<Governor>,
    shutdown: &tokio_util::sync::CancellationToken,
) -> Result<()> {
    if wallets.is_empty() || shutdown.is_cancelled() {
        return Ok(());
    }
    if cfg.helius_api_key.trim().is_empty() {
        eprintln!("DBG helius ingest: HELIUS_API_KEY empty -> skipping");
        return Ok(());
    }

    eprintln!(
        "DBG helius ingest: api_key_len={} wallets={} (concurrent)",
        cfg.helius_api_key.trim().len(),
        wallets.len()
    );

    // Fetch all wallets concurrently with a hard 10s global timeout
    let futures: Vec<_> = wallets.iter().map(|wallet| {
        let cfg = cfg.clone();
        let gov = gov.clone();
        let wallet = wallet.clone();
        async move { fetch_wallet_transactions(&cfg, &gov, &wallet).await }
    }).collect();

    let results = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        futures_util::future::join_all(futures),
    ).await.unwrap_or_default();

    // Process results sequentially (db + coins need exclusive access)
    for (wallet, result) in wallets.iter().zip(results.into_iter()) {
        match result {
            Ok(txs) => {
                process_wallet_transactions(db, coins, cfg, wallet, txs)?;
            }
            Err(e) => {
                let w = wallet.as_str();
                let snip = if w.len() >= 8 { &w[..8] } else { w };
                eprintln!("DBG wallet fetch ERR wallet={} err={:?}", snip, e);
            }
        }
    }

    Ok(())
}
