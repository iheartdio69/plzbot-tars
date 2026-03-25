/// Paper trading wallets — 5 strategy profiles running in parallel
///
/// Each wallet independently tracks calls using its own entry size,
/// stop loss, and take profit rules. After enough calls, we compare
/// which strategy performs best and tune the live bot accordingly.
///
/// To disable: remove the `paper_wallets` mod from trading/mod.rs.

use serde::{Deserialize, Serialize};
use crate::time::now_ts;

// ── Strategy definitions ──────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WalletStrategy {
    pub name: &'static str,
    pub sol_size: f64,
    pub stop_loss_pct: Option<f64>,
    pub time_stop_mins: Option<u64>,
    pub tp_levels: Vec<f64>,
    pub tp_exit_pcts: Vec<f64>,
    pub max_entry_fdv: Option<f64>,
    pub min_entry_fdv: Option<f64>,
    /// Trailing profit lock: (trigger_mult, floor_mult, sell_pct)
    /// When coin hits trigger_mult, set a floor at floor_mult.
    /// If price drops back to floor, sell sell_pct% to lock profit.
    /// Example: (3.0, 2.0, 40.0) = hit 3x → protect 2x → sell 40% if it drops to 2x
    pub trailing_locks: Vec<(f64, f64, f64)>,
}

pub fn all_strategies() -> Vec<WalletStrategy> {
    vec![
        // ══════════════════════════════════════════════════════════════
        // V3 STRATEGIES — Based on v2 data analysis
        // Key insight: stop losses kill meme coin profits. No stops = wins.
        // ══════════════════════════════════════════════════════════════

        // 🌙 GUT_MOON — Shoot for the big x's. Never sells early.
        // Half the position rides to 10x/20x — this is the moonshot wallet.
        // When it hits, it more than pays for all the losses.
        // ── MINIMUM EXIT: 2.2x ─────────────────────────────────────────
        // At 0.1 SOL position size, fees eat ~5% of small profits.
        // Never sell below 2.2x — let it ride or die.
        // ──────────────────────────────────────────────────────────────

        // 🌙 GUT_MOON — Moonshot. First floor at 2.5x, protects from there.
        WalletStrategy {
            name: "GUT_MOON",
            sol_size: 0.125,
            stop_loss_pct: None,
            time_stop_mins: None,
            tp_levels: vec![10.0, 20.0],
            tp_exit_pcts: vec![50.0, 50.0],
            max_entry_fdv: None,
            min_entry_fdv: None,
            trailing_locks: vec![
                (4.0,  2.5, 30.0),  // hit 4x → protect 2.5x → sell 30% on reversal
                (7.0,  4.0, 30.0),  // hit 7x → protect 4x
                (12.0, 7.0, 25.0),  // hit 12x → protect 7x
            ],
        },

        // 💰 GUT_LOCK — Profit lock. Min first TP raised to 2.2x.
        WalletStrategy {
            name: "GUT_LOCK",
            sol_size: 0.125,
            stop_loss_pct: None,
            time_stop_mins: None,
            tp_levels: vec![2.2, 5.0, 15.0],  // min 2.2x before any sale
            tp_exit_pcts: vec![40.0, 35.0, 25.0],
            max_entry_fdv: None,
            min_entry_fdv: None,
            trailing_locks: vec![
                (3.0, 2.2, 0.0),   // hit 3x → floor at 2.2x (TP handles sell)
                (5.0, 3.0, 20.0),  // hit 5x → if drops to 3x, sell extra 20%
            ],
        },

        // 🎯 GUT_STRICT — Tight entry, 2.2x minimum floor
        WalletStrategy {
            name: "GUT_STRICT",
            sol_size: 0.25,
            stop_loss_pct: None,
            time_stop_mins: None,
            tp_levels: vec![10.0, 20.0],
            tp_exit_pcts: vec![50.0, 50.0],
            max_entry_fdv: Some(40_000.0),
            min_entry_fdv: None,
            trailing_locks: vec![
                (4.0,  2.5, 30.0),
                (7.0,  4.0, 30.0),
                (12.0, 7.0, 25.0),
            ],
        },

        // 🐳 WHALE — First TP at 5x, floor locks from 2.5x up
        WalletStrategy {
            name: "WHALE",
            sol_size: 0.5,
            stop_loss_pct: None,
            time_stop_mins: None,
            tp_levels: vec![5.0, 15.0],
            tp_exit_pcts: vec![50.0, 50.0],
            max_entry_fdv: Some(30_000.0),
            min_entry_fdv: None,
            trailing_locks: vec![
                (3.0,  2.2, 20.0),  // hit 3x → protect 2.2x minimum
                (5.0,  3.0, 20.0),  // hit 5x → protect 3x (TP fires at 5x anyway)
                (10.0, 6.0, 20.0),  // hit 10x → protect 6x
            ],
        },

        // 🎟️ MOONBAG — Pure lottery, no sales before 5x reversal
        WalletStrategy {
            name: "MOONBAG",
            sol_size: 0.05,
            stop_loss_pct: None,
            time_stop_mins: None,
            tp_levels: vec![20.0, 50.0],
            tp_exit_pcts: vec![50.0, 50.0],
            max_entry_fdv: None,
            min_entry_fdv: None,
            trailing_locks: vec![
                (5.0,  3.0, 20.0),
                (10.0, 6.0, 20.0),
                (20.0, 12.0, 0.0),  // TP fires at 20x
            ],
        },

        // 🔫 SNIPER_V2 — Sub-$10k, first exit at 3x minimum
        WalletStrategy {
            name: "SNIPER_V2",
            sol_size: 0.5,
            stop_loss_pct: None,
            time_stop_mins: Some(120),
            tp_levels: vec![3.0, 10.0],
            tp_exit_pcts: vec![50.0, 50.0],
            max_entry_fdv: Some(10_000.0),
            min_entry_fdv: None,
            trailing_locks: vec![
                (3.0, 2.2, 0.0),   // TP fires at 3x
                (5.0, 3.0, 25.0),  // hit 5x → if drops to 3x, sell 25% extra
            ],
        },
    ]
}

// ── Paper trade record ────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaperTrade {
    pub strategy: String,
    pub mint: String,
    pub entry_fdv: f64,
    pub entry_ts: u64,
    pub sol_in: f64,
    pub peak_mult: f64,
    pub exit_mult: f64,       // actual exit multiplier (weighted avg of TP exits)
    pub sol_out: f64,         // SOL returned (paper)
    pub pnl_sol: f64,         // sol_out - sol_in
    pub status: TradeStatus,
    pub exit_reason: String,
    pub tps_hit: Vec<f64>,    // which TP levels were triggered
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum TradeStatus {
    Open,
    Closed,
}

impl PaperTrade {
    pub fn new(strategy: &str, mint: &str, entry_fdv: f64, sol_in: f64) -> Self {
        Self {
            strategy: strategy.to_string(),
            mint: mint.to_string(),
            entry_fdv,
            entry_ts: now_ts(),
            sol_in,
            peak_mult: 1.0,
            exit_mult: 1.0,
            sol_out: 0.0,
            pnl_sol: 0.0,
            status: TradeStatus::Open,
            exit_reason: String::new(),
            tps_hit: Vec::new(),
        }
    }
}

// ── Strategy dashboard ────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct StrategyStats {
    pub name: String,
    pub total_trades: usize,
    pub open_trades: usize,
    pub wins: usize,
    pub losses: usize,
    pub total_pnl_sol: f64,
    pub best_trade_mult: f64,
    pub worst_trade_mult: f64,
}

impl StrategyStats {
    pub fn win_rate(&self) -> f64 {
        let resolved = self.wins + self.losses;
        if resolved == 0 { return 0.0; }
        self.wins as f64 / resolved as f64 * 100.0
    }
}

// ── Paper wallet manager ──────────────────────────────────────────────

const TRADES_PATH: &str = "data/paper_wallet_trades.json";

pub fn load_trades() -> Vec<PaperTrade> {
    std::fs::read_to_string(TRADES_PATH)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save_trades(trades: &[PaperTrade]) {
    if let Ok(s) = serde_json::to_string_pretty(trades) {
        let _ = std::fs::create_dir_all("data");
        let _ = std::fs::write(TRADES_PATH, s);
    }
}

/// Called when a new CALL is made — opens a paper trade for every strategy
pub fn open_paper_trades(mint: &str, entry_fdv: f64) {
    let mut trades = load_trades();
    for strat in all_strategies() {
        // Don't re-open if already tracking this mint for this strategy
        if trades.iter().any(|t| t.mint == mint && t.strategy == strat.name) {
            continue;
        }
        // FDV filter — skip if coin doesn't meet strategy's entry criteria
        if let Some(max) = strat.max_entry_fdv {
            if entry_fdv > max { continue; }
        }
        if let Some(min) = strat.min_entry_fdv {
            if entry_fdv < min { continue; }
        }
        trades.push(PaperTrade::new(strat.name, mint, entry_fdv, strat.sol_size));
    }
    save_trades(&trades);
}

/// Called on every market tick — updates open trades with current price
pub fn update_paper_trades(mint: &str, current_fdv: f64) {
    let mut trades = load_trades();
    let now = now_ts();
    let mut changed = false;

    for trade in trades.iter_mut() {
        if trade.mint != mint || trade.status == TradeStatus::Closed {
            continue;
        }

        let mult = if trade.entry_fdv > 0.0 { current_fdv / trade.entry_fdv } else { 1.0 };
        if mult > trade.peak_mult {
            trade.peak_mult = mult;
        }

        let strategy = all_strategies().into_iter().find(|s| s.name == trade.strategy);
        let Some(strat) = strategy else { continue; };

        // Time stop check (SNIPER strategy)
        if let Some(time_stop) = strat.time_stop_mins {
            let age_mins = (now - trade.entry_ts) / 60;
            if age_mins >= time_stop {
                close_trade(trade, mult, "TIME_STOP");
                changed = true;
                continue;
            }
        }

        // Stop loss check
        if let Some(sl) = strat.stop_loss_pct {
            if mult <= 1.0 - sl {
                close_trade(trade, mult, "STOP_LOSS");
                changed = true;
                continue;
            }
        }

        // TP checks — partial exits
        for (i, &tp) in strat.tp_levels.iter().enumerate() {
            if mult >= tp && !trade.tps_hit.contains(&tp) {
                trade.tps_hit.push(tp);
                let exit_pct = strat.tp_exit_pcts.get(i).copied().unwrap_or(100.0) / 100.0;
                trade.sol_out += trade.sol_in * exit_pct * mult;
                changed = true;

                if trade.tps_hit.len() == strat.tp_levels.len() {
                    close_trade(trade, mult, "ALL_TP_HIT");
                }
            }
        }

        // ── TRAILING PROFIT LOCK ──────────────────────────────────────
        // When coin hits a trigger mult, set a floor.
        // If price drops back to the floor, sell sell_pct% to lock profit.
        // Stored as negative values in tps_hit to distinguish from TPs.
        for &(trigger, floor, sell_pct) in &strat.trailing_locks {
            let lock_key = -(trigger as i64) as f64; // negative = trailing lock marker
            let floor_key = -(floor as i64 * 100 + 1) as f64; // unique floor marker

            // Activate lock when trigger is hit
            if mult >= trigger && !trade.tps_hit.contains(&lock_key) {
                trade.tps_hit.push(lock_key);
                changed = true;
            }

            // Fire the floor sell if lock is active and price dropped to floor
            if trade.tps_hit.contains(&lock_key)
                && mult <= floor
                && !trade.tps_hit.contains(&floor_key)
                && sell_pct > 0.0
                && trade.status != TradeStatus::Closed
            {
                trade.tps_hit.push(floor_key);
                let exit_pct = sell_pct / 100.0;
                trade.sol_out += trade.sol_in * exit_pct * mult;
                changed = true;
                println!("  🔒 TRAIL LOCK {} | triggered {:.1}x floor → sold {:.0}% at {:.2}x",
                    &trade.mint[..8], floor, sell_pct, mult);
            }
        }
    }

    if changed {
        save_trades(&trades);
    }
}

fn close_trade(trade: &mut PaperTrade, exit_mult: f64, reason: &str) {
    // Any remaining position exits at current price
    let already_out_pct: f64 = trade.tps_hit.len() as f64 / 10.0; // rough
    let remaining = (trade.sol_in - trade.sol_out / exit_mult.max(1.0)).max(0.0);
    trade.sol_out += remaining * exit_mult;
    trade.exit_mult = exit_mult;
    trade.pnl_sol = trade.sol_out - trade.sol_in;
    trade.status = TradeStatus::Closed;
    trade.exit_reason = reason.to_string();
    let _ = already_out_pct; // suppress warning
}

/// Build dashboard stats for all 5 strategies
pub fn dashboard_stats() -> Vec<StrategyStats> {
    let trades = load_trades();
    let strategies = all_strategies();

    strategies.iter().map(|strat| {
        let strat_trades: Vec<&PaperTrade> = trades.iter()
            .filter(|t| t.strategy == strat.name)
            .collect();

        let closed: Vec<&PaperTrade> = strat_trades.iter()
            .filter(|t| t.status == TradeStatus::Closed)
            .copied()
            .collect();

        let wins = closed.iter().filter(|t| t.pnl_sol > 0.0).count();
        let losses = closed.iter().filter(|t| t.pnl_sol <= 0.0).count();
        let total_pnl_sol: f64 = closed.iter().map(|t| t.pnl_sol).sum();
        let best = closed.iter().map(|t| t.exit_mult).fold(1.0_f64, f64::max);
        let worst = closed.iter().map(|t| t.exit_mult).fold(999.0_f64, f64::min);

        StrategyStats {
            name: strat.name.to_string(),
            total_trades: strat_trades.len(),
            open_trades: strat_trades.iter().filter(|t| t.status == TradeStatus::Open).count(),
            wins,
            losses,
            total_pnl_sol,
            best_trade_mult: if closed.is_empty() { 0.0 } else { best },
            worst_trade_mult: if closed.is_empty() { 0.0 } else { worst },
        }
    }).collect()
}

/// Format dashboard as a compact Telegram message
pub fn dashboard_telegram() -> String {
    let stats = dashboard_stats();
    let mut lines = vec!["📊 <b>Paper Wallet Dashboard</b>\n".to_string()];

    for s in &stats {
        let pnl_sign = if s.total_pnl_sol >= 0.0 { "+" } else { "" };
        lines.push(format!(
            "<b>{}</b>\n  Trades: {} ({} open) | WR: {:.0}%\n  PnL: {}{:.3} SOL | Best: {:.1}x\n",
            s.name,
            s.total_trades,
            s.open_trades,
            s.win_rate(),
            pnl_sign,
            s.total_pnl_sol,
            s.best_trade_mult,
        ));
    }

    lines.join("")
}
