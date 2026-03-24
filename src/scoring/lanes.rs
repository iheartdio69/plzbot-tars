/// Lane classification — determines the "type" of a coin call
///
/// Ported from psychic-spoon's proven classification system.
/// Used by: scoring engine, resolver, Telegram alerts, trade analyst.
///
/// Win rates (from old bot data):
/// - SNIPE:      ~80% WR, avg 2.13x
/// - CONVICTION: ~70% WR (strong SOL inflow)
/// - SPIKE:      ~60% WR (sudden volume burst)
/// - NEWBORN:    ~25% WR (high risk, high reward)
/// - Others:     track as we build data

use crate::market::cache::MarketTrend;

#[derive(Debug, Clone, PartialEq)]
pub enum Lane {
    /// FDV <$50k + 15%+ growth in 5 min — best signal, 80% WR
    Snipe,
    /// Mid cap ($30k–$500k) + $15k+ absolute FDV gain in 5 min
    Conviction,
    /// Sudden burst: 30+ buys in 5m AND 10%+ velocity
    Spike,
    /// Already running at $700k+ FDV — late but real momentum
    Runner,
    /// Brand new coin, <3 minutes old
    Newborn,
    /// Mid cap $120k+ with traction
    Mid,
    /// Small cap developing
    Small,
}

impl Lane {
    pub fn as_str(&self) -> &'static str {
        match self {
            Lane::Snipe      => "SNIPE",
            Lane::Conviction => "CONVICTION",
            Lane::Spike      => "SPIKE",
            Lane::Runner     => "RUNNER",
            Lane::Newborn    => "NEWBORN",
            Lane::Mid        => "MID",
            Lane::Small      => "SMALL",
        }
    }

    /// Is this a high-confidence lane worth a full position?
    pub fn is_high_confidence(&self) -> bool {
        matches!(self, Lane::Snipe | Lane::Conviction)
    }

    /// Should we use a reduced position size for this lane?
    pub fn is_risky(&self) -> bool {
        matches!(self, Lane::Newborn | Lane::Small)
    }
}

impl std::fmt::Display for Lane {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// Classify a coin into a lane based on market trend data.
///
/// # Arguments
/// * `trend`    — market snapshot from DexScreener
/// * `age_secs` — how long we've been tracking this coin (in seconds)
/// * `fdv`      — current FDV in USD
pub fn classify_lane(trend: &MarketTrend, age_secs: u64, fdv: f64) -> Lane {
    // SNIPE: small cap pumping fast — best signal
    // FDV < $50k AND 15%+ growth in 5 min
    if trend.early_snipe {
        return Lane::Snipe;
    }

    // CONVICTION: mid cap with real SOL inflow
    // $30k–$500k AND $15k+ absolute FDV gain in 5 min
    if trend.conviction_momentum {
        return Lane::Conviction;
    }

    // SPIKE: sudden volume burst regardless of FDV
    if trend.buys_5m >= 30 && trend.fdv_velocity_pct >= 10.0 {
        return Lane::Spike;
    }

    // RUNNER: already at large cap, real momentum
    if fdv >= 700_000.0 {
        return Lane::Runner;
    }

    // NEWBORN: brand new coin, <3 minutes old
    if age_secs <= 180 {
        return Lane::Newborn;
    }

    // MID: established mid cap with traction
    if fdv >= 120_000.0 {
        return Lane::Mid;
    }

    // SMALL: everything else
    Lane::Small
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::market::cache::MarketTrend;

    #[test]
    fn test_snipe_classification() {
        let mut t = MarketTrend::default();
        t.early_snipe = true;
        assert_eq!(classify_lane(&t, 60, 25_000.0), Lane::Snipe);
    }

    #[test]
    fn test_newborn_classification() {
        let t = MarketTrend::default();
        assert_eq!(classify_lane(&t, 120, 10_000.0), Lane::Newborn);
    }

    #[test]
    fn test_runner_classification() {
        let t = MarketTrend::default();
        assert_eq!(classify_lane(&t, 600, 800_000.0), Lane::Runner);
    }
}
