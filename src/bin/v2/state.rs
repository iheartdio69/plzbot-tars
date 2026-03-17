use super::market::DexSnap;
use std::collections::{HashMap, HashSet, VecDeque};

#[derive(Clone, Debug)]
pub struct Cand {
    pub mint: String,
    pub symbol: Option<String>,
    pub name: Option<String>,
    pub fdv: Option<f64>,
    pub liq_usd: Option<f64>,
    pub tx_5m: Option<u64>,
    pub age_secs: Option<u64>,
}

#[derive(Default)]
pub struct State {
    seen: HashSet<String>,
    watch: VecDeque<String>,

    first_seen: HashMap<String, i64>,
    last_call: HashMap<String, i64>,

    band: HashSet<String>,
    snaps: HashMap<String, DexSnap>,

    poll_cursor: usize,
}

impl State {
    pub fn watch_len(&self) -> usize {
        self.watch.len()
    }

    pub fn watch_snapshot(&self, n: usize) -> Vec<String> {
        self.watch.iter().take(n).cloned().collect()
    }

    pub fn new() -> Self {
        Self::default()
    }

    pub fn ingest_mint(&mut self, mint: String, cap: usize) -> bool {
        if mint.trim().is_empty() {
            return false;
        }
        if !self.seen.insert(mint.clone()) {
            return false;
        }
        self.first_seen.entry(mint.clone()).or_insert_with(now_ts);
        self.watch.push_back(mint);

        while self.watch.len() > cap {
            if let Some(old) = self.watch.pop_front() {
                self.seen.remove(&old);
                self.band.remove(&old);
                self.snaps.remove(&old);
                self.first_seen.remove(&old);
                self.last_call.remove(&old);
            }
        }
        true
    }

    pub fn update_snap(&mut self, mint: &str, snap: &DexSnap) {
        self.snaps.insert(mint.to_string(), snap.clone());
    }

    pub fn touch_watch_band(&mut self, mint: &str) {
        self.band.insert(mint.to_string());
    }

    pub fn first_seen(&self, mint: &str) -> i64 {
        *self.first_seen.get(mint).unwrap_or(&now_ts())
    }

    pub fn last_call_ts(&self, mint: &str) -> Option<i64> {
        self.last_call.get(mint).copied()
    }

    pub fn mark_called(&mut self, mint: &str, ts: i64) {
        self.last_call.insert(mint.to_string(), ts);
    }

    pub fn next_poll_batch(&mut self, n: usize) -> Vec<String> {
        if self.watch.is_empty() {
            return vec![];
        }

        let len = self.watch.len();
        let mut out: Vec<String> = Vec::with_capacity(n);

        for _ in 0..n {
            if self.poll_cursor >= len {
                self.poll_cursor = 0;
            }
            if let Some(m) = self.watch.get(self.poll_cursor) {
                out.push(m.clone());
            }
            self.poll_cursor += 1;
            if out.len() >= n {
                break;
            }
        }

        out
    }

    pub fn top_candidates(&self, n: usize) -> Vec<Cand> {
        let mut rows: Vec<Cand> = Vec::new();
        let now = now_ts();

        for mint in self.band.iter() {
            if let Some(s) = self.snaps.get(mint) {
                let age = now.saturating_sub(self.first_seen(mint)) as u64;
                rows.push(Cand {
                    mint: mint.clone(),
                    symbol: s.symbol.clone(),
                    name: s.name.clone(),
                    fdv: s.fdv,
                    liq_usd: s.liq_usd,
                    tx_5m: s.tx_5m,
                    age_secs: Some(age),
                });
            }
        }

        rows.sort_by(|a, b| {
            let ax = a.tx_5m.unwrap_or(0);
            let bx = b.tx_5m.unwrap_or(0);
            bx.cmp(&ax)
                .then_with(|| (b.fdv.unwrap_or(0.0) as i64).cmp(&(a.fdv.unwrap_or(0.0) as i64)))
        });

        rows.truncate(n);
        rows
    }
}

fn now_ts() -> i64 {
    let s = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or(std::time::Duration::from_secs(0))
        .as_secs();
    s as i64
}
