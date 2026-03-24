use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Position {
    pub mint: String,
    pub entry_price_usd: f64,
    pub entry_fdv: f64,
    pub sol_invested: f64,
    pub tokens_held: f64,
    pub entry_ts: u64,
    pub tp1_mult: f64,       // e.g. 1.5 = 50% gain
    pub tp2_mult: f64,       // e.g. 2.0 = 100% gain
    pub sl_pct: f64,         // e.g. 0.30 = -30% stop loss
    pub tp1_triggered: bool,
    pub tp2_triggered: bool,
    pub status: PositionStatus,
    pub outcome: Option<String>,
    pub peak_fdv: f64,
    pub peak_mult: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum PositionStatus {
    Open,
    PartialExit, // TP1 hit, still holding rest
    Closed,
}

impl Position {
    pub fn new(mint: String, entry_price_usd: f64, entry_fdv: f64, sol_invested: f64) -> Self {
        let tp1_mult: f64 = std::env::var("TARS_TP1_MULT")
            .unwrap_or_else(|_| "1.5".into())
            .parse()
            .unwrap_or(1.5);
        let tp2_mult: f64 = std::env::var("TARS_TP2_MULT")
            .unwrap_or_else(|_| "2.0".into())
            .parse()
            .unwrap_or(2.0);
        let sl_pct: f64 = std::env::var("TARS_SL_PCT")
            .unwrap_or_else(|_| "0.30".into())
            .parse()
            .unwrap_or(0.30);

        Self {
            mint,
            entry_price_usd,
            entry_fdv,
            sol_invested,
            tokens_held: 0.0,
            entry_ts: crate::time::now_ts(),
            tp1_mult,
            tp2_mult,
            sl_pct,
            tp1_triggered: false,
            tp2_triggered: false,
            status: PositionStatus::Open,
            outcome: None,
            peak_fdv: entry_fdv,
            peak_mult: 1.0,
        }
    }

    /// Returns the action to take given current FDV
    pub fn check_thresholds(&self, current_fdv: f64) -> PositionAction {
        if self.status == PositionStatus::Closed {
            return PositionAction::Hold;
        }

        let mult = current_fdv / self.entry_fdv;
        let down_pct = (self.entry_fdv - current_fdv) / self.entry_fdv;

        // Stop loss
        if down_pct >= self.sl_pct {
            return PositionAction::ExitFull(format!("-{:.0}% SL hit", down_pct * 100.0));
        }

        // TP2 — full exit
        if mult >= self.tp2_mult && self.tp1_triggered {
            return PositionAction::ExitFull(format!("{:.1}x TP2 hit", mult));
        }

        // TP1 — sell 50%, keep moonbag
        if mult >= self.tp1_mult && !self.tp1_triggered {
            return PositionAction::ExitPartial(50.0, format!("{:.1}x TP1 hit", mult));
        }

        PositionAction::Hold
    }
}

#[derive(Debug)]
pub enum PositionAction {
    Hold,
    ExitPartial(f64, String), // pct to sell, reason
    ExitFull(String),          // reason
}

/// Persist positions to disk
pub fn save_positions(positions: &[Position]) {
    let path = "data/positions.json";
    if let Ok(s) = serde_json::to_string_pretty(positions) {
        let _ = std::fs::write(path, s);
    }
}

pub fn load_positions() -> Vec<Position> {
    let path = "data/positions.json";
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}
