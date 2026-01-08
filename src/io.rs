use crate::types::{RugSet, WalletRepMap};
use serde_json;
use std::fs;

static WALLETS_FILE: &str = "wallets.json";
static RUGS_FILE: &str = "rug_wallets.json";

static mut WALLET_REP: Option<WalletRepMap> = None;
static mut RUGS: Option<RugSet> = None;

pub fn wallet_rep_mut() -> &'static mut WalletRepMap {
    unsafe { WALLET_REP.get_or_insert_with(WalletRepMap::default) }
}
pub fn rugs_mut() -> &'static mut RugSet {
    unsafe { RUGS.get_or_insert_with(RugSet::default) }
}

pub fn load_reputation() -> Result<(), String> {
    // wallets
    if let Ok(s) = fs::read_to_string(WALLETS_FILE) {
        if let Ok(map) = serde_json::from_str::<WalletRepMap>(&s) {
            let n = map.len();
            *wallet_rep_mut() = map;
            println!(
                "✅ Ishmael: Loaded {} wallet reputations from {}",
                n, WALLETS_FILE
            );
        }
    } else {
        println!(
            "✅ Ishmael: Loaded 0 wallet reputations from {}",
            WALLETS_FILE
        );
    }

    // rugs
    if let Ok(s) = fs::read_to_string(RUGS_FILE) {
        if let Ok(set) = serde_json::from_str::<RugSet>(&s) {
            let n = set.len();
            *rugs_mut() = set;
            println!(
                "🚩 Ishmael: Loaded {} known rug wallets from {}",
                n, RUGS_FILE
            );
        }
    } else {
        println!("🚩 Ishmael: Loaded 0 known rug wallets from {}", RUGS_FILE);
    }

    Ok(())
}

pub fn save_reputation() -> Result<(), String> {
    let wallets = serde_json::to_string_pretty(wallet_rep_mut()).map_err(|e| e.to_string())?;
    fs::write(WALLETS_FILE, wallets).map_err(|e| e.to_string())?;
    println!(
        "💾 Ishmael: Saved {} wallet reputations",
        wallet_rep_mut().len()
    );

    let rugs = serde_json::to_string_pretty(rugs_mut()).map_err(|e| e.to_string())?;
    fs::write(RUGS_FILE, rugs).map_err(|e| e.to_string())?;
    println!("🚨 Ishmael: Saved {} rug wallets", rugs_mut().len());

    Ok(())
}
