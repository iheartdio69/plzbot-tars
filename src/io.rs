use crate::types::{RugSet, WalletRepMap};
use serde_json;
use std::fs;

static WALLETS_FILE: &str = "wallets.json";
static RUGS_FILE: &str = "rug_wallets.json";

static mut WALLET_REP: Option<WalletRepMap> = None;
static mut RUGS: Option<RugSet> = None;

pub fn export_wallets_json_from_db(db_path: &str) -> Result<(), String> {
    use rusqlite::Connection;
    use std::fs;

    let conn = Connection::open(db_path).map_err(|e| e.to_string())?;

    // You MUST adjust these column names after you show me `.schema wallets`
    let mut stmt = conn
        .prepare("SELECT wallet, score, hits, rugs FROM wallets")
        .map_err(|e| e.to_string())?;

    let mut out: std::collections::HashMap<String, crate::types::WalletReputation> =
        std::collections::HashMap::new();

    let rows = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                crate::types::WalletReputation {
                    score: r.get::<_, i64>(1)? as i32,
                    hits: r.get::<_, i64>(2)? as u64,
                    rugs: r.get::<_, i64>(3)? as u64,
                },
            ))
        })
        .map_err(|e| e.to_string())?;

    for row in rows {
        let (w, rep) = row.map_err(|e| e.to_string())?;
        out.insert(w, rep);
    }

    let json = serde_json::to_string_pretty(&out).map_err(|e| e.to_string())?;
    fs::write("wallets.json", json).map_err(|e| e.to_string())?;
    Ok(())
}

pub fn wallet_rep_mut() -> &'static mut WalletRepMap {
    unsafe { WALLET_REP.get_or_insert_with(WalletRepMap::default) }
}
pub fn rugs_mut() -> &'static mut RugSet {
    unsafe { RUGS.get_or_insert_with(RugSet::default) }
}

pub fn load_reputation() -> Result<(), String> {
    let cwd = std::env::current_dir()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| "<unknown>".to_string());

    println!(
        "DBG Ishmael: load_reputation cwd={} wallets_file={} rugs_file={}",
        cwd, WALLETS_FILE, RUGS_FILE
    );

    // wallets.json
    match fs::read_to_string(WALLETS_FILE) {
        Ok(s) => match serde_json::from_str::<WalletRepMap>(&s) {
            Ok(map) => {
                let n = map.len();
                *wallet_rep_mut() = map;
                println!(
                    "✅ Ishmael: Loaded {} wallet reputations from {}",
                    n, WALLETS_FILE
                );
            }
            Err(e) => {
                return Err(format!(
                    "Failed to parse {} as WalletRepMap JSON: {}",
                    WALLETS_FILE, e
                ));
            }
        },
        Err(e) => {
            println!(
                "⚠️ Ishmael: Could not read {} (will start empty). err={}",
                WALLETS_FILE, e
            );
            let _ = wallet_rep_mut();
        }
    }

    // rug_wallets.json
    match fs::read_to_string(RUGS_FILE) {
        Ok(s) => match serde_json::from_str::<RugSet>(&s) {
            Ok(set) => {
                let n = set.len();
                *rugs_mut() = set;
                println!(
                    "🚩 Ishmael: Loaded {} known rug wallets from {}",
                    n, RUGS_FILE
                );
            }
            Err(e) => {
                return Err(format!(
                    "Failed to parse {} as RugSet JSON: {}",
                    RUGS_FILE, e
                ));
            }
        },
        Err(e) => {
            println!(
                "⚠️ Ishmael: Could not read {} (will start empty). err={}",
                RUGS_FILE, e
            );
            let _ = rugs_mut();
        }
    }

    println!(
        "DBG Ishmael: rep_state reps={} rugs={}",
        wallet_rep_mut().len(),
        rugs_mut().len()
    );

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
