#[derive(Debug, Default, Clone)]
pub struct Counters {
    pub considered: usize,
    pub called: usize,

    pub skipped_cooldown: usize,
    pub skipped_threshold: usize,

    pub skip_fdv: usize,
    pub skip_conc: usize,
    pub skip_wallet: usize,
    pub skip_signer: usize,
    pub skip_cooldown: usize,
    pub skip_events: usize, // ✅ change this
    pub skip_other: usize,

    pub snapshots_wrote: u64,
    pub queue_rotated: u64,
    pub queue_dropped_ttl: usize,
}
