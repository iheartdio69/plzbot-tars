// rugcheck.rs
// Fetches safety and holder data from rugcheck.xyz
// Free API, no auth needed.

use serde::Deserialize;
use std::collections::HashMap;

const RUGCHECK_API: &str = "https://api.rugcheck.xyz/v1/tokens";

#[derive(Debug, Clone, Default)]
pub struct RugReport {
    pub score: u64,                    // lower = safer. >1000 = danger
    pub mint_authority_revoked: bool,  // true = safe (can't mint more)
    pub freeze_authority_revoked: bool,// true = safe (can't freeze wallets)
    pub top5_holder_pct: f64,          // % held by top 5 wallets
    pub top_holder_pct: f64,           // % held by single largest wallet
    pub total_holders: u64,            // total unique holders
    pub creator_pct: f64,              // % the creator/dev still holds
    pub is_rugged: bool,               // already rugged flag
    pub risk_count: usize,             // number of risk flags
    pub high_risk_count: usize,        // number of HIGH severity risks
    pub has_insider_network: bool,     // coordinated wallet network detected
    pub lp_providers: u64,             // number of LP providers
    pub risks: Vec<String>,            // human readable risk list
    pub fetched: bool,                 // did we get data?
}

impl RugReport {
    pub fn is_safe(&self) -> bool {
        if !self.fetched { return true; } // no data = don't block
        if self.is_rugged { return false; }
        if !self.mint_authority_revoked { return false; }
        if self.top_holder_pct > 30.0 { return false; } // one wallet holds 30%+ = danger
        if self.high_risk_count >= 2 { return false; }
        if self.score > 2000 { return false; }
        true
    }

    pub fn score_modifier(&self) -> i32 {
        if !self.fetched { return 0; }

        let mut modifier = 0i32;

        // Positive signals
        if self.mint_authority_revoked { modifier += 10; }
        if self.freeze_authority_revoked { modifier += 5; }
        if self.total_holders >= 100 { modifier += 10; }
        if self.total_holders >= 500 { modifier += 15; }
        if self.total_holders >= 1000 { modifier += 20; }
        if self.top5_holder_pct < 20.0 { modifier += 10; } // well distributed
        if self.lp_providers >= 5 { modifier += 5; }
        if self.creator_pct < 5.0 { modifier += 10; } // dev not holding much

        // Negative signals
        if self.top_holder_pct > 20.0 { modifier -= 20; }
        if self.top_holder_pct > 10.0 { modifier -= 10; }
        if self.top5_holder_pct > 40.0 { modifier -= 20; }
        if self.has_insider_network { modifier -= 30; }
        if self.high_risk_count > 0 { modifier -= 15 * self.high_risk_count as i32; }
        if self.risk_count > 3 { modifier -= 10; }
        if self.creator_pct > 10.0 { modifier -= 15; } // dev holding a lot
        if self.creator_pct > 20.0 { modifier -= 20; }
        if self.score > 1000 { modifier -= 20; }

        modifier
    }
}

#[derive(Debug, Deserialize)]
struct RawReport {
    score: Option<u64>,
    rugged: Option<bool>,
    #[serde(rename = "topHolders")]
    top_holders: Option<Vec<HolderEntry>>,
    #[serde(rename = "totalHolders")]
    total_holders: Option<u64>,
    #[serde(rename = "totalLPProviders")]
    total_lp_providers: Option<u64>,
    token: Option<TokenInfo>,
    risks: Option<Vec<RiskEntry>>,
    #[serde(rename = "graphInsidersDetected")]
    graph_insiders_detected: Option<bool>,
    creator: Option<String>,
    #[serde(rename = "creatorBalance")]
    creator_balance: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct HolderEntry {
    pct: Option<f64>,
    owner: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TokenInfo {
    #[serde(rename = "mintAuthority")]
    mint_authority: Option<serde_json::Value>,
    #[serde(rename = "freezeAuthority")]
    freeze_authority: Option<serde_json::Value>,
    supply: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct RiskEntry {
    name: Option<String>,
    level: Option<String>,
}

pub async fn fetch_rug_report(mint: &str) -> RugReport {
    let url = format!("{}/{}/report", RUGCHECK_API, mint);
    let resp = match reqwest::get(&url).await {
        Ok(r) => r,
        Err(_) => return RugReport::default(),
    };
    if !resp.status().is_success() {
        return RugReport::default();
    }
    let raw: RawReport = match resp.json().await {
        Ok(r) => r,
        Err(_) => return RugReport::default(),
    };

    let holders = raw.top_holders.unwrap_or_default();
    let top5_pct: f64 = holders.iter().take(5).filter_map(|h| h.pct).sum();
    let top1_pct: f64 = holders.first().and_then(|h| h.pct).unwrap_or(0.0);

    // Figure out creator % — find creator in holder list
    let creator_addr = raw.creator.unwrap_or_default();
    let creator_pct = holders.iter()
        .find(|h| h.owner.as_deref() == Some(&creator_addr))
        .and_then(|h| h.pct)
        .unwrap_or(0.0);

    let token = raw.token.unwrap_or(TokenInfo {
        mint_authority: None,
        freeze_authority: None,
        supply: None,
    });

    let mint_revoked = token.mint_authority.is_none()
        || token.mint_authority == Some(serde_json::Value::Null);
    let freeze_revoked = token.freeze_authority.is_none()
        || token.freeze_authority == Some(serde_json::Value::Null);

    let risks = raw.risks.unwrap_or_default();
    let risk_names: Vec<String> = risks.iter()
        .filter_map(|r| r.name.clone())
        .collect();
    let high_risk_count = risks.iter()
        .filter(|r| r.level.as_deref() == Some("danger"))
        .count();

    RugReport {
        score: raw.score.unwrap_or(0),
        mint_authority_revoked: mint_revoked,
        freeze_authority_revoked: freeze_revoked,
        top5_holder_pct: top5_pct,
        top_holder_pct: top1_pct,
        total_holders: raw.total_holders.unwrap_or(0),
        creator_pct,
        is_rugged: raw.rugged.unwrap_or(false),
        risk_count: risks.len(),
        high_risk_count,
        has_insider_network: raw.graph_insiders_detected.unwrap_or(false),
        lp_providers: raw.total_lp_providers.unwrap_or(0),
        risks: risk_names,
        fetched: true,
    }
}
