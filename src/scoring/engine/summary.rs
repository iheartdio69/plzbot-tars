use crate::scoring::engine::Counters;

pub fn print_summary(c: &Counters, active_len: usize, queue_len: usize) {
    // headline color based on whether we're actually calling or just scanning
    let head = if c.called > 0 {
        crate::fmt::NEON_PINK
    } else if c.considered > 0 {
        crate::fmt::YELLOW
    } else {
        crate::fmt::DIM
    };

    // highlight big skip numbers (tweak thresholds whenever)
    let fdv_c = if c.skip_fdv > 0 {
        crate::fmt::ORANGE
    } else {
        crate::fmt::DIM
    };
    let conc_c = if c.skip_conc > 0 {
        crate::fmt::PURPLE
    } else {
        crate::fmt::DIM
    };
    let wallet_c = if c.skip_wallet > 0 {
        crate::fmt::LIGHT_BLUE
    } else {
        crate::fmt::DIM
    };
    let cooldown_c = if c.skip_cooldown > 0 {
        crate::fmt::YELLOW
    } else {
        crate::fmt::DIM
    };
    let signer_c = if c.skip_signer > 0 {
        crate::fmt::CYAN
    } else {
        crate::fmt::DIM
    };
    let other_c = if c.skip_other > 0 {
        crate::fmt::DIM
    } else {
        crate::fmt::DIM
    };
    let dropped_c = if c.queue_dropped_ttl > 0 {
        crate::fmt::RED
    } else {
        crate::fmt::DIM
    };

    // activity colors
    let considered_c = if c.considered > 0 {
        crate::fmt::LIGHT_BLUE
    } else {
        crate::fmt::DIM
    };
    let called_c = if c.called > 0 {
        crate::fmt::NEON_GREEN
    } else {
        crate::fmt::DIM
    };

    let active_c = if active_len >= 10 {
        crate::fmt::NEON_GREEN
    } else {
        crate::fmt::LIGHT_BLUE
    };
    let queue_c = if queue_len >= 50 {
        crate::fmt::RED
    } else if queue_len >= 20 {
        crate::fmt::ORANGE
    } else if queue_len > 0 {
        crate::fmt::YELLOW
    } else {
        crate::fmt::DIM
    };

    println!(
        "{head}🧮 scoring{reset} {considered_c}considered={}{reset} {called_c}called={}{reset} skips: \
{fdv_c}fdv={}{reset} {conc_c}conc={}{reset} {wallet_c}wallet={}{reset} {cooldown_c}cooldown={}{reset} {signer_c}signer={}{reset} {other_c}other={}{reset} \
{dropped_c}queue_dropped={}{reset} \
{active_c}active={}{reset} {queue_c}queue={}{reset}",
        c.considered,
        c.called,
        c.skip_fdv,
        c.skip_conc,
        c.skip_wallet,
        c.skip_cooldown,
        c.skip_signer,
        c.skip_other,
        c.queue_dropped_ttl,
        active_len,
        queue_len,
        head = head,
        reset = crate::fmt::RESET,
        considered_c = considered_c,
        called_c = called_c,
        fdv_c = fdv_c,
        conc_c = conc_c,
        wallet_c = wallet_c,
        cooldown_c = cooldown_c,
        signer_c = signer_c,
        other_c = other_c,
        dropped_c = dropped_c,
        active_c = active_c,
        queue_c = queue_c
    );
}
