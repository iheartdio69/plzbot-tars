// src/governor.rs
//
// Fix for your compile error:
// - `acquire_owned()` exists on Arc<Semaphore>, not on &Semaphore
// - so we store `Arc<Semaphore>` in Governor.
//
// This file is drop-in.

use std::sync::Arc;
use std::time::{Duration, Instant};

use rand::{thread_rng, Rng};
use tokio::sync::{Mutex, OwnedSemaphorePermit, Semaphore};
use tokio::time::sleep;

#[derive(Debug)]
struct Bucket {
    rate_per_sec: f64,
    burst: f64,
    tokens: f64,
    last: Instant,
}

impl Bucket {
    fn new(rate_per_sec: f64, burst: f64) -> Self {
        Self {
            rate_per_sec,
            burst,
            tokens: 0.0, // start EMPTY so we don't startup-slam the API
            last: Instant::now(),
        }
    }

    fn refill(&mut self) {
        let now = Instant::now();
        let dt = now.duration_since(self.last).as_secs_f64();
        self.last = now;
        self.tokens = (self.tokens + dt * self.rate_per_sec).min(self.burst);
    }

    fn wait_time_for(&mut self, n: f64) -> Duration {
        self.refill();

        if self.tokens >= n {
            self.tokens -= n;
            return Duration::from_millis(0);
        }

        let deficit = n - self.tokens;
        let secs = deficit / self.rate_per_sec.max(0.000001);
        Duration::from_secs_f64(secs)
    }

    fn penalize(&mut self, n: f64) {
        self.tokens = (self.tokens - n).max(0.0);
    }

    fn set_rate_burst(&mut self, rate_per_sec: f64, burst: f64) {
        self.rate_per_sec = rate_per_sec;
        self.burst = burst.max(0.0);
        self.tokens = self.tokens.min(self.burst);
    }
}

#[derive(Clone, Copy, Debug)]
pub enum Lane {
    Rpc,
    Das,
    Enhanced,
}

pub struct Governor {
    rpc: Mutex<Bucket>,
    das: Mutex<Bucket>,
    enhanced: Mutex<Bucket>,

    // Arc so we can use acquire_owned()
    inflight: Option<Arc<Semaphore>>,

    backoff_rpc_ms: Mutex<u64>,
    backoff_das_ms: Mutex<u64>,
    backoff_enh_ms: Mutex<u64>,
}

impl Governor {
    /// inflight_limit:
    ///   - 0 => no inflight semaphore
    ///   - 2..6 => recommended
    pub fn new(
        rpc_rps: f64,
        rpc_burst: f64,
        das_rps: f64,
        das_burst: f64,
        enh_rps: f64,
        enh_burst: f64,
        inflight_limit: usize,
    ) -> Self {
        Self {
            rpc: Mutex::new(Bucket::new(rpc_rps, rpc_burst)),
            das: Mutex::new(Bucket::new(das_rps, das_burst)),
            enhanced: Mutex::new(Bucket::new(enh_rps, enh_burst)),
            inflight: if inflight_limit > 0 {
                Some(Arc::new(Semaphore::new(inflight_limit)))
            } else {
                None
            },
            backoff_rpc_ms: Mutex::new(0),
            backoff_das_ms: Mutex::new(0),
            backoff_enh_ms: Mutex::new(0),
        }
    }

    // -----------------------------
    // Acquire APIs (hold permit across the HTTP request)
    // -----------------------------

    pub async fn acquire_rpc(&self) -> GovPermit {
        self.acquire(Lane::Rpc).await
    }

    pub async fn acquire_das(&self) -> GovPermit {
        self.acquire(Lane::Das).await
    }

    pub async fn acquire_enhanced(&self) -> GovPermit {
        self.acquire(Lane::Enhanced).await
    }

    async fn acquire(&self, lane: Lane) -> GovPermit {
        // 1) inflight cap (prevents request storms)
        let inflight_permit: Option<OwnedSemaphorePermit> = match &self.inflight {
            Some(sem) => Some(sem.clone().acquire_owned().await.expect("semaphore closed")),
            None => None,
        };

        // 2) token bucket pacing
        let bucket_mutex = match lane {
            Lane::Rpc => &self.rpc,
            Lane::Das => &self.das,
            Lane::Enhanced => &self.enhanced,
        };

        loop {
            let wait = {
                let mut b = bucket_mutex.lock().await;
                b.wait_time_for(1.0)
            };
            if wait.is_zero() {
                break;
            }
            sleep(wait).await;
        }

        // 3) lane backoff (after 429s)
        self.apply_backoff_sleep(lane).await;

        GovPermit {
            lane,
            _inflight: inflight_permit,
        }
    }

    // -----------------------------
    // 429 handling (call when response.status == 429)
    // -----------------------------

    pub async fn on_429_rpc(&self) {
        self.on_429(Lane::Rpc).await
    }

    pub async fn on_429_das(&self) {
        self.on_429(Lane::Das).await
    }

    pub async fn on_429_enhanced(&self) {
        self.on_429(Lane::Enhanced).await
    }

    async fn on_429(&self, lane: Lane) {
        // exponential backoff w/ cap
        let backoff = match lane {
            Lane::Rpc => &self.backoff_rpc_ms,
            Lane::Das => &self.backoff_das_ms,
            Lane::Enhanced => &self.backoff_enh_ms,
        };

        let mut ms = backoff.lock().await;
        *ms = if *ms == 0 { 250 } else { (*ms * 2).min(8_000) };

        // penalize bucket so global pace slows immediately
        let bucket_mutex = match lane {
            Lane::Rpc => &self.rpc,
            Lane::Das => &self.das,
            Lane::Enhanced => &self.enhanced,
        };

        let mut b = bucket_mutex.lock().await;
        b.penalize(2.0);
    }

    /// Call on non-429 success so backoff decays quickly.
    pub async fn on_success(&self, lane: Lane) {
        let backoff = match lane {
            Lane::Rpc => &self.backoff_rpc_ms,
            Lane::Das => &self.backoff_das_ms,
            Lane::Enhanced => &self.backoff_enh_ms,
        };

        let mut ms = backoff.lock().await;
        *ms /= 2;
        if *ms < 80 {
            *ms = 0;
        }
    }

    async fn apply_backoff_sleep(&self, lane: Lane) {
        let backoff = match lane {
            Lane::Rpc => &self.backoff_rpc_ms,
            Lane::Das => &self.backoff_das_ms,
            Lane::Enhanced => &self.backoff_enh_ms,
        };

        let ms = *backoff.lock().await;
        if ms == 0 {
            return;
        }

        let jitter: u64 = thread_rng().gen_range(0..=150);
        sleep(Duration::from_millis(ms + jitter)).await;
    }

    // -----------------------------
    // Optional: live tuning
    // -----------------------------

    pub async fn set_rpc_limits(&self, rps: f64, burst: f64) {
        let mut b = self.rpc.lock().await;
        b.set_rate_burst(rps, burst);
    }

    pub async fn set_das_limits(&self, rps: f64, burst: f64) {
        let mut b = self.das.lock().await;
        b.set_rate_burst(rps, burst);
    }

    pub async fn set_enhanced_limits(&self, rps: f64, burst: f64) {
        let mut b = self.enhanced.lock().await;
        b.set_rate_burst(rps, burst);
    }
}

/// Keep this alive until the HTTP request finishes.
pub struct GovPermit {
    lane: Lane,
    _inflight: Option<OwnedSemaphorePermit>,
}

impl GovPermit {
    pub fn lane(&self) -> Lane {
        self.lane
    }
}
