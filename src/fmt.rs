// src/fmt.rs

pub const RESET: &str = "\x1b[0m";
pub const BOLD: &str = "\x1b[1m";

// 256-color ANSI palette (works in most terminals)
pub const NEON_PINK: &str = "\x1b[38;5;205m";
pub const MINT_GREEN: &str = "\x1b[38;5;121m";
pub const NEON_GREEN: &str = "\x1b[38;5;82m";
pub const ORANGE: &str = "\x1b[38;5;208m";
pub const LIGHT_BLUE: &str = "\x1b[38;5;117m";
pub const PURPLE: &str = "\x1b[38;5;141m";
pub const YELLOW: &str = "\x1b[38;5;226m";
pub const CYAN: &str = "\x1b[38;5;51m";
pub const RED: &str = "\x1b[38;5;196m";
pub const DIM: &str = "\x1b[2m";

// Extra accents
pub const BLUE: &str = "\x1b[38;5;27m";
pub const GRAY: &str = "\x1b[38;5;245m";
pub const MAGENTA: &str = "\x1b[38;5;201m";

pub fn wrap(color: &str, s: &str) -> String {
    format!("{color}{s}{RESET}")
}

pub fn bold(s: &str) -> String {
    format!("{BOLD}{s}{RESET}")
}

// Backwards-compatible helpers you already use elsewhere
pub fn pink(s: &str) -> String {
    wrap(NEON_PINK, s)
}
pub fn green(s: &str) -> String {
    wrap(NEON_GREEN, s)
}
pub fn yellow(s: &str) -> String {
    wrap(YELLOW, s)
}
pub fn cyan(s: &str) -> String {
    wrap(CYAN, s)
}
pub fn red(s: &str) -> String {
    wrap(RED, s)
}
pub fn dim(s: &str) -> String {
    wrap(DIM, s)
}
pub fn gray(s: &str) -> String {
    wrap(GRAY, s)
}
pub fn blue(s: &str) -> String {
    wrap(BLUE, s)
}
pub fn purple(s: &str) -> String {
    wrap(PURPLE, s)
}
pub fn orange(s: &str) -> String {
    wrap(ORANGE, s)
}
pub fn magenta(s: &str) -> String {
    wrap(MAGENTA, s)
}

// Mint formatting (always mint green, bold)
pub fn mint(s: &str) -> String {
    format!("{BOLD}{MINT_GREEN}{s}{RESET}")
}

// Score formatting (tweak bands however you want)
pub fn score_fmt(score: i32) -> String {
    let c = if score >= 75 {
        NEON_GREEN
    } else if score >= 55 {
        LIGHT_BLUE
    } else if score >= 35 {
        PURPLE
    } else if score >= 20 {
        YELLOW
    } else {
        DIM
    };
    format!("{c}{score}{RESET}")
}

// FDV band colors:
// <20k orange, 20-50k light blue, 50-100k purple, 100k+ neon green
pub fn fdv_band(fdv: f64) -> &'static str {
    if fdv >= 100_000.0 {
        NEON_GREEN
    } else if fdv >= 50_000.0 {
        PURPLE
    } else if fdv >= 20_000.0 {
        LIGHT_BLUE
    } else {
        ORANGE
    }
}

pub fn fdv_fmt(fdv: f64) -> String {
    let c = fdv_band(fdv);
    format!("{c}${:.0}{RESET}", fdv)
}

// Percent formatter (0.0..1.0 -> "12.3%")
pub fn pct(x: f64) -> String {
    format!("{:.1}%", x * 100.0)
}

// Win-rate band colors
pub fn wr_color(wr: f64) -> &'static str {
    if wr >= 0.60 {
        NEON_GREEN
    } else if wr >= 0.45 {
        YELLOW
    } else {
        RED
    }
}

// Simple bold cyan headline label
pub fn headline(s: &str) -> String {
    format!("{BOLD}{CYAN}{s}{RESET}")
}

// Gray bullet prefix
pub fn bullet() -> String {
    format!("{GRAY}•{RESET}")
}

// -------------------------
// Tags (NEW)
// -------------------------

/// Map a single tag to a stable terminal color.
/// Keep this opinionated: one glance => you know what lane it is.
pub fn tag_color(tag: &str) -> &'static str {
    match tag {
        "GAMBOL" => RED,
        "REVIVE" => YELLOW,
        "RUNNER" => NEON_GREEN,
        "MEGA" => PURPLE,
        "NEWBORN" => CYAN,
        "MID" => LIGHT_BLUE,
        "SMALL" => ORANGE,
        "WPLUS" => NEON_GREEN,
        "WNEG" => RED,
        _ => GRAY,
    }
}

/// Colorize "TAG|TAG2|TAG3" where the FIRST tag decides the primary color.
/// (So calls get ONE consistent lane color.)
pub fn tag_fmt(tags: &str) -> String {
    let primary = tags.split('|').next().unwrap_or("");
    let c = tag_color(primary);
    format!("{BOLD}{c}[{tags}]{RESET}")
}

// Used by scoring/engine.rs
pub fn active_line(mint_addr: &str, score: i32) -> String {
    format!(
        "✅ ACTIVE: {} (score={})",
        mint(mint_addr),
        score_fmt(score)
    )
}

pub fn gambol_call_line(inner: &str) -> String {
    format!("{}🎲 GAMBOL {}{}", RED, inner, RESET)
}

// Used by scoring/engine.rs (kept for compatibility)
pub fn call_line(
    mint_addr: &str,
    fdv: f64,
    score: i32,
    tx_5m: u64,
    signers: usize,
    events: usize,
) -> String {
    // CALL label in neon pink, mint in mint green, fdv band-colored, score band-colored
    format!(
        "{NEON_PINK}📣 CALL:{RESET} {} fdv={} score={} tx_5m={} signers={} events={}",
        mint(mint_addr),
        fdv_fmt(fdv),
        score_fmt(score),
        tx_5m,
        signers,
        events
    )
}

/// New helper: CALL line that includes colored tags at the end.
/// You pass tags like "RUNNER|WPLUS" or "GAMBOL|WNEG".
pub fn call_line_tagged(
    mint_addr: &str,
    fdv: f64,
    score: i32,
    tx_5m: u64,
    signers: usize,
    events: usize,
    tags: &str,
) -> String {
    format!(
        "{NEON_PINK}📣 CALL:{RESET} {} fdv={} score={} tx_5m={} signers={} events={}  {}",
        mint(mint_addr),
        fdv_fmt(fdv),
        score_fmt(score),
        tx_5m,
        signers,
        events,
        tag_fmt(tags),
    )
}

pub fn pct_fmt(p: f64) -> String {
    let pct = p * 100.0;
    let c = if pct >= 55.0 {
        NEON_GREEN
    } else if pct >= 45.0 {
        LIGHT_BLUE
    } else if pct >= 35.0 {
        YELLOW
    } else {
        RED
    };
    format!("{c}{:.1}%{RESET}", pct)
}

pub fn mult_fmt(x: f64) -> String {
    let c = if x >= 1.8 {
        NEON_GREEN
    } else if x >= 1.4 {
        LIGHT_BLUE
    } else if x >= 1.1 {
        YELLOW
    } else {
        RED
    };
    format!("{c}{:.2}x{RESET}", x)
}

pub fn perf_line(
    label: &str,
    total: i64,
    wins: i64,
    losses: i64,
    win_rate: f64,
    avg_mult: f64,
) -> String {
    format!(
        "{BOLD}{CYAN}📈 PERF{RESET} {DIM}({label}){RESET} total={}  W={}  L={}  hit={}  avg_peak/call={}",
        total,
        green(&wins.to_string()),
        red(&losses.to_string()),
        pct_fmt(win_rate),
        mult_fmt(avg_mult),
    )
}

/// One-shot “bot accuracy” block (print this after you write new outcomes)
/// inserted = number of outcomes graded in that pass
/// tuples are (total, wins, losses)
pub fn accuracy_block(
    inserted: i64,
    all: (i64, i64, i64),
    last50: (i64, i64, i64),
    last20: (i64, i64, i64),
) -> String {
    let (t_all, w_all, l_all) = all;
    let (t_50, w_50, l_50) = last50;
    let (t_20, w_20, l_20) = last20;

    let wr_all = if t_all > 0 {
        w_all as f64 / t_all as f64
    } else {
        0.0
    };
    let wr_50 = if t_50 > 0 {
        w_50 as f64 / t_50 as f64
    } else {
        0.0
    };
    let wr_20 = if t_20 > 0 {
        w_20 as f64 / t_20 as f64
    } else {
        0.0
    };

    format!(
        "{}  {}{}(updated: +{} graded){}\n\
         {} all-time: {}{}{}  {}W/L={}/{} n={}{}\n\
         {} last 50:  {}{}{}  {}W/L={}/{} n={}{}\n\
         {} last 20:  {}{}{}  {}W/L={}/{} n={}{}",
        headline("📊 BOT ACCURACY"),
        DIM,
        GRAY,
        inserted,
        RESET,
        bullet(),
        wr_color(wr_all),
        pct(wr_all),
        RESET,
        DIM,
        w_all,
        l_all,
        t_all,
        RESET,
        bullet(),
        wr_color(wr_50),
        pct(wr_50),
        RESET,
        DIM,
        w_50,
        l_50,
        t_50,
        RESET,
        bullet(),
        wr_color(wr_20),
        pct(wr_20),
        RESET,
        DIM,
        w_20,
        l_20,
        t_20,
        RESET,
    )
}
