// rug_tracker.rs
// Dynamically tracks wallets involved in losing calls.
// On LOSS: increments strike count for each wallet involved.
// On WIN: decrements (they might just be degen traders, not rugs).
// Feeds back into reputation at startup.

use crate::reputation::{RUG_WALLETS, WALLET_REPUTATION};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

const RUG_PATH: &str = "data/rug_wallets.json";
const STRIKES_TO_FLAG: u32 = 2; // 2 losses = flagged

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct WalletStrike {
    pub strikes: u32,
    pub assists: u32, // wins they were involved in
    pub flagged: bool,
}

pub fn load_rug_tracker() -> HashMap<String, WalletStrike> {
    let Ok(s) = std::fs::read_to_string(RUG_PATH) else {
        return HashMap::new();
    };
    serde_json::from_str(&s).unwrap_or_default()
}

pub fn save_rug_tracker(tracker: &HashMap<String, WalletStrike>) {
    if let Ok(s) = serde_json::to_string_pretty(tracker) {
        let _ = std::fs::write(RUG_PATH, s);
    }
}

pub fn record_loss(tracker: &mut HashMap<String, WalletStrike>, wallets: &[String]) {
    let mut newly_flagged = vec![];

    for wallet in wallets {
        let entry = tracker.entry(wallet.clone()).or_default();
        entry.strikes += 1;

        if !entry.flagged && entry.strikes >= STRIKES_TO_FLAG {
            entry.flagged = true;
            newly_flagged.push(wallet.clone());
        }
    }

    // Apply to live reputation immediately
    if !newly_flagged.is_empty() {
        let mut rug = RUG_WALLETS.lock().unwrap();
        let mut rep = WALLET_REPUTATION.lock().unwrap();

        for wallet in &newly_flagged {
            rug.insert(wallet.clone());
            rep.insert(wallet.clone(), -50.0); // hard negative
            println!("🚩 RUG FLAGGED: {} (earned {} strikes)", &wallet[..20.min(wallet.len())], STRIKES_TO_FLAG);
        }
    }

    // Give soft negative to all loss-associated wallets
    {
        let mut rep = WALLET_REPUTATION.lock().unwrap();
        for wallet in wallets {
            let score = rep.entry(wallet.clone()).or_insert(0.0);
            *score -= 3.0;
        }
    }
}

pub fn record_win(tracker: &mut HashMap<String, WalletStrike>, wallets: &[String]) {
    let mut rep = WALLET_REPUTATION.lock().unwrap();
    for wallet in wallets {
        let entry = tracker.entry(wallet.clone()).or_default();
        entry.assists += 1;

        // Boost reputation for wallets on winning coins
        let score = rep.entry(wallet.clone()).or_insert(0.0);
        *score += 5.0;
    }
}

pub fn apply_to_reputation(tracker: &HashMap<String, WalletStrike>) {
    let mut rug = RUG_WALLETS.lock().unwrap();
    let mut rep = WALLET_REPUTATION.lock().unwrap();

    let mut flagged = 0usize;
    for (wallet, data) in tracker {
        if data.flagged {
            rug.insert(wallet.clone());
            rep.insert(wallet.clone(), -50.0);
            flagged += 1;
        }
    }
    if flagged > 0 {
        println!("🚩 Loaded {} dynamically flagged rug wallets", flagged);
    }
}
