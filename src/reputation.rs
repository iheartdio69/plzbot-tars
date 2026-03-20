// reputation.rs
// Loads wallet reputation from ranked CSV and avoid list into the scoring engine's
// WALLET_REPUTATION and RUG_WALLETS maps.

use lazy_static::lazy_static;
use std::collections::{HashMap, HashSet};
use std::sync::Mutex;

lazy_static! {
    pub static ref WALLET_REPUTATION: Mutex<HashMap<String, f64>> = Mutex::new(HashMap::new());
    pub static ref RUG_WALLETS: Mutex<HashSet<String>> = Mutex::new(HashSet::new());
}

pub fn load_reputation() {
    load_ranked();
    load_avoid();
    load_whales();

    let rep = WALLET_REPUTATION.lock().unwrap();
    let rug = RUG_WALLETS.lock().unwrap();
    println!(
        "📚 Reputation loaded: {} wallets scored, {} rug-flagged",
        rep.len(),
        rug.len()
    );
}

fn load_ranked() {
    let path = "reports/archive/2026-01-06/wallets_ranked.csv";
    let Ok(content) = std::fs::read_to_string(path) else {
        println!("⚠️  wallets_ranked.csv not found at {}", path);
        return;
    };

    let mut rep = WALLET_REPUTATION.lock().unwrap();
    let mut loaded = 0usize;

    for line in content.lines().skip(1) {
        // wallet,score,wins,losses,samples,winrate,...
        let cols: Vec<&str> = line.split(',').collect();
        if cols.len() < 2 { continue; }
        let wallet = cols[0].trim().to_string();
        let score: f64 = cols[1].trim().parse().unwrap_or(0.0);
        if wallet.is_empty() { continue; }
        rep.insert(wallet, score);
        loaded += 1;
    }

    println!("  ✅ Loaded {} ranked wallets", loaded);
}

fn load_avoid() {
    let path = "reports/archive/2026-01-06/wallets_avoid.csv";
    let Ok(content) = std::fs::read_to_string(path) else {
        println!("⚠️  wallets_avoid.csv not found at {}", path);
        return;
    };

    let mut rug = RUG_WALLETS.lock().unwrap();
    let mut loaded = 0usize;

    for line in content.lines().skip(1) {
        let wallet = line.split(',').next().unwrap_or("").trim().to_string();
        if wallet.is_empty() { continue; }
        rug.insert(wallet);
        loaded += 1;
    }

    println!("  🚩 Loaded {} rug-flagged wallets", loaded);
}

fn load_whales() {
    let path = "whales.json";
    let Ok(content) = std::fs::read_to_string(path) else { return; };
    let Ok(map): Result<serde_json::Value, _> = serde_json::from_str(&content) else { return; };

    let mut rep = WALLET_REPUTATION.lock().unwrap();
    let mut loaded = 0usize;

    if let Some(obj) = map.as_object() {
        for (wallet, data) in obj {
            let score = data.get("score").and_then(|s| s.as_f64()).unwrap_or(0.0);
            let wins = data.get("wins").and_then(|w| w.as_f64()).unwrap_or(0.0);
            // Boost whales that have wins
            let whale_rep = score + (wins * 2.0);
            rep.entry(wallet.clone())
                .and_modify(|e| *e += whale_rep)
                .or_insert(whale_rep);
            loaded += 1;
        }
    }

    println!("  🐋 Loaded {} whale scores", loaded);
}

pub fn save_reputation() {
    // Save any runtime-updated reputation back to wallets.json
    let rep = WALLET_REPUTATION.lock().unwrap();
    let map: HashMap<String, serde_json::Value> = rep.iter().map(|(k, v)| {
        (k.clone(), serde_json::json!({ "score": v, "wins": 0, "losses": 0, "seen": 1 }))
    }).collect();

    if let Ok(s) = serde_json::to_string_pretty(&map) {
        let _ = std::fs::write("wallets.json", s);
    }
}
