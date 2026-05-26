//! Three-state circuit breaker with a fixed fault window.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Result, bail};
use parking_lot::Mutex;

/// Circuit breaker state machine with three states.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    /// Requests flow normally. Faults are tracked within a fixed fault window.
    Closed,
    /// Requests are rejected. Transitions to `HalfOpen` after the reset period.
    Open,
    /// A limited number of probe requests are allowed through to test recovery.
    HalfOpen,
}

/// Result of a circuit breaker check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BreakerDecision {
    /// The request may proceed.
    Allow,
    /// The circuit is open; the request is rejected.
    Reject,
}

/// Internal outcome of `check_at`, encoding the decision and any state transition.
#[derive(Debug)]
enum CheckOutcome {
    /// Request allowed; breaker is `Closed` (no transition).
    Allowed,
    /// Request allowed; breaker just entered `HalfOpen` (first probe).
    EnteredProbe,
    /// Request rejected; breaker is `Open` or `HalfOpen` at probe capacity.
    Rejected,
}

/// How the breaker was tripped to `Open`.
#[derive(Debug)]
enum TripCause {
    /// Fault threshold reached while `Closed`.
    FaultsExceeded(u32),
    /// A fault during `HalfOpen` immediately re-opened the breaker.
    ProbeFailed,
}

/// Configuration for a single circuit breaker instance.
#[derive(Debug, Clone)]
pub struct BreakerConfig {
    /// Number of faults within `fault_window` required to trip `Closed` → `Open`.
    pub switch_on_threshold: u32,
    /// Number of successful probe requests required to transition `HalfOpen` → `Closed`.
    /// Also caps the total probe requests admitted per `HalfOpen` period.
    pub switch_off_threshold: u32,
    /// How long the breaker stays `Open` before transitioning to `HalfOpen`.
    pub reset_period: Duration,
    /// Rolling window in which faults are counted while `Closed`.
    pub fault_window: Duration,
}

impl BreakerConfig {
    /// Validate that all thresholds and durations are sensible.
    ///
    /// Called at startup via `BucketRegistry::new`; invalid configs surface as
    /// an error through `Backend::connect_with` rather than silently misbehaving.
    pub fn validate(&self) -> Result<()> {
        if self.switch_on_threshold < 1 {
            bail!("switch_on_threshold must be >= 1");
        }
        if self.switch_off_threshold < 1 {
            bail!("switch_off_threshold must be >= 1");
        }
        if self.reset_period.is_zero() {
            bail!("reset_period must be > 0");
        }
        if self.fault_window.is_zero() {
            bail!("fault_window must be > 0");
        }
        Ok(())
    }
}

impl Default for BreakerConfig {
    fn default() -> Self {
        Self {
            switch_on_threshold: 10,
            switch_off_threshold: 5,
            reset_period: Duration::from_secs(10),
            fault_window: Duration::from_secs(30),
        }
    }
}

#[derive(Debug)]
struct Inner {
    state: State,
    fault_count: u32,
    fault_window_start: Option<Instant>,
    success_count: u32,
    probe_count: u32,
    opened_at: Option<Instant>,
}

/// A three-state circuit breaker with a fixed fault window.
///
/// Faults are counted within a fixed window anchored at the first fault
/// after construction (or after each window reset). Both `fault_window_start`
/// and `opened_at` are lazily initialized — they remain `None` until the
/// first fault or trip, so the breaker carries no synthetic timestamps.
///
/// When the window expires (elapsed > `fault_window`), the counter resets
/// and the window re-anchors at the next fault. This prevents slow trickles
/// of errors from accumulating into a false trip.
///
/// Note: the boundary comparison uses strict `>`, so a fault arriving exactly
/// at the window edge is still counted in the current window.
#[derive(Debug)]
pub struct CircuitBreaker {
    name: String,
    inner: Mutex<Inner>,
    config: BreakerConfig,
}

impl CircuitBreaker {
    pub fn new(name: impl Into<String>, config: BreakerConfig) -> Self {
        Self {
            name: name.into(),
            inner: Mutex::new(Inner {
                state: State::Closed,
                fault_count: 0,
                fault_window_start: None,
                success_count: 0,
                probe_count: 0,
                opened_at: None,
            }),
            config,
        }
    }

    /// Check whether a request is allowed through.
    pub fn check(&self) -> BreakerDecision {
        self.check_at(Instant::now())
    }

    fn check_at(&self, now: Instant) -> BreakerDecision {
        let outcome = {
            let mut inner = self.inner.lock();
            match inner.state {
                State::Closed => CheckOutcome::Allowed,
                State::Open => {
                    // Invariant: opened_at is always Some when state == Open (set by record_failure_at).
                    // Fallback to `now` keeps the breaker Open (elapsed 0 < reset_period) rather than panicking.
                    let opened = inner.opened_at.unwrap_or(now);
                    if now.duration_since(opened) >= self.config.reset_period {
                        inner.state = State::HalfOpen;
                        inner.success_count = 0;
                        inner.probe_count = 1;
                        CheckOutcome::EnteredProbe
                    } else {
                        CheckOutcome::Rejected
                    }
                }
                State::HalfOpen => {
                    if inner.probe_count < self.config.switch_off_threshold {
                        inner.probe_count += 1;
                        // Release lock before exiting the outer block (clippy::significant_drop_tightening).
                        drop(inner);
                        CheckOutcome::Allowed
                    } else {
                        CheckOutcome::Rejected
                    }
                }
            }
        };

        match outcome {
            CheckOutcome::EnteredProbe => {
                tracing::info!(
                    gauge.circuit_breaker_state = 2,
                    state = "half_open",
                    bucket = %self.name,
                    "circuit breaker entering probe state"
                );
                BreakerDecision::Allow
            }
            CheckOutcome::Rejected => {
                tracing::debug!(
                    monotonic_counter.circuit_breaker_rejections = 1,
                    bucket = %self.name,
                    "request rejected by circuit breaker"
                );
                BreakerDecision::Reject
            }
            CheckOutcome::Allowed => BreakerDecision::Allow,
        }
    }

    /// Record a successful response. In `HalfOpen`, counts toward closing the breaker.
    pub fn record_success(&self) {
        self.record_success_at(Instant::now());
    }

    // Kept for parametric symmetry with check_at / record_failure_at; success doesn't anchor a window.
    fn record_success_at(&self, _now: Instant) {
        let recovered = {
            let mut inner = self.inner.lock();
            if inner.state == State::HalfOpen {
                inner.success_count += 1;
                if inner.success_count >= self.config.switch_off_threshold {
                    inner.state = State::Closed;
                    inner.fault_count = 0;
                    inner.fault_window_start = None;
                    true
                } else {
                    false
                }
            } else {
                false
            }
        };

        if recovered {
            tracing::info!(
                monotonic_counter.circuit_breaker_recoveries = 1,
                gauge.circuit_breaker_state = 0,
                state = "closed",
                bucket = %self.name,
                "circuit breaker recovered"
            );
        }
    }

    /// Record a fault. In `Closed`, counts toward tripping. In `HalfOpen`, trips immediately.
    pub fn record_failure(&self) {
        self.record_failure_at(Instant::now());
    }

    fn record_failure_at(&self, now: Instant) {
        let tripped = {
            let mut inner = self.inner.lock();
            match inner.state {
                State::Closed => {
                    let window_expired = inner
                        .fault_window_start
                        .is_some_and(|start| now.duration_since(start) > self.config.fault_window);
                    if inner.fault_window_start.is_none() || window_expired {
                        inner.fault_count = 0;
                        inner.fault_window_start = Some(now);
                    }
                    inner.fault_count += 1;
                    (inner.fault_count >= self.config.switch_on_threshold).then(|| {
                        inner.state = State::Open;
                        inner.opened_at = Some(now);
                        TripCause::FaultsExceeded(inner.fault_count)
                    })
                }
                State::HalfOpen => {
                    inner.state = State::Open;
                    inner.opened_at = Some(now);
                    // Release lock before exiting the outer block (clippy::significant_drop_tightening).
                    drop(inner);
                    Some(TripCause::ProbeFailed)
                }
                State::Open => None,
            }
        };

        match tripped {
            Some(TripCause::ProbeFailed) => {
                tracing::warn!(
                    monotonic_counter.circuit_breaker_trips = 1,
                    gauge.circuit_breaker_state = 1,
                    state = "open",
                    bucket = %self.name,
                    "circuit breaker re-tripped during probe"
                );
            }
            Some(TripCause::FaultsExceeded(fault_count)) => {
                tracing::warn!(
                    monotonic_counter.circuit_breaker_trips = 1,
                    gauge.circuit_breaker_state = 1,
                    state = "open",
                    bucket = %self.name,
                    fault_count,
                    "circuit breaker tripped"
                );
            }
            None => {}
        }
    }

    /// Return the current state (for diagnostics).
    #[cfg(test)]
    pub fn state(&self) -> State {
        self.inner.lock().state
    }

    #[cfg(test)]
    fn fault_count(&self) -> u32 {
        self.inner.lock().fault_count
    }
}

/// The result of resolving a circuit breaker from the registry.
#[derive(Debug)]
pub struct ResolvedBreaker {
    /// The circuit breaker instance.
    pub breaker: Arc<CircuitBreaker>,
    /// The bucket name used.
    pub bucket: String,
}

/// Registry holding per-bucket circuit breakers.
///
/// The breaker only activates when the guest explicitly names a bucket via
/// `OutboundPolicy.upstream`. Requests without an upstream skip the breaker
/// entirely (retry + timeout still apply). Requesting an unknown bucket is
/// a misconfiguration error.
#[derive(Debug)]
pub struct BucketRegistry {
    buckets: HashMap<String, Arc<CircuitBreaker>>,
}

impl BucketRegistry {
    /// Build a registry from parsed bucket names.
    ///
    /// Every bucket shares the same `config`. Bucket names create isolated
    /// fault-tracking boundaries — requests to different upstreams get independent
    /// breaker instances with independent fault counts.
    ///
    /// Duplicate names are silently deduplicated.
    pub fn new(names: impl IntoIterator<Item = String>, config: &BreakerConfig) -> Result<Self> {
        config.validate()?;

        let mut buckets = HashMap::new();
        for name in names {
            if !buckets.contains_key(&name) {
                buckets.insert(name.clone(), Arc::new(CircuitBreaker::new(name, config.clone())));
            }
        }

        if buckets.is_empty() {
            tracing::debug!("no circuit breaker buckets configured");
        }

        Ok(Self { buckets })
    }

    /// Resolve a breaker by explicit upstream name.
    ///
    /// Returns `Ok(None)` when no upstream is specified (breaker is skipped).
    /// Returns `Ok(Some(_))` for a known bucket, `Err(_)` for an unknown bucket.
    pub fn resolve(
        &self, upstream: Option<&str>,
    ) -> std::result::Result<Option<ResolvedBreaker>, String> {
        let Some(name) = upstream else {
            return Ok(None);
        };

        self.buckets.get(name).map_or_else(
            || Err(format!("unknown circuit breaker bucket: {name}")),
            |b| {
                Ok(Some(ResolvedBreaker {
                    breaker: Arc::clone(b),
                    bucket: name.to_owned(),
                }))
            },
        )
    }

    /// Returns `true` if the registry has no configured buckets.
    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.buckets.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> BreakerConfig {
        BreakerConfig {
            switch_on_threshold: 3,
            switch_off_threshold: 2,
            reset_period: Duration::from_millis(100),
            fault_window: Duration::from_millis(500),
        }
    }

    /// Record enough faults at `now` to trip the breaker from `Closed` → `Open`.
    fn trip_breaker(cb: &CircuitBreaker, now: Instant) {
        for _ in 0..cb.config.switch_on_threshold {
            cb.record_failure_at(now);
        }
        assert_eq!(cb.state(), State::Open);
    }

    #[test]
    fn closed_allows_requests() {
        let cb = CircuitBreaker::new("test", test_config());
        assert_eq!(cb.check(), BreakerDecision::Allow);
        assert_eq!(cb.state(), State::Closed);
    }

    #[test]
    fn transitions_to_open_after_threshold() {
        let cb = CircuitBreaker::new("test", test_config());
        trip_breaker(&cb, Instant::now());
    }

    #[test]
    fn does_not_trip_below_threshold() {
        let cb = CircuitBreaker::new("test", test_config());
        let now = Instant::now();
        for _ in 0..2 {
            cb.record_failure_at(now);
        }
        assert_eq!(cb.state(), State::Closed);
    }

    #[test]
    fn rejects_when_open() {
        let cb = CircuitBreaker::new("test", test_config());
        let now = Instant::now();
        trip_breaker(&cb, now);
        assert_eq!(cb.check_at(now), BreakerDecision::Reject);
    }

    #[test]
    fn transitions_to_half_open_after_reset_period() {
        let cb = CircuitBreaker::new("test", test_config());
        let now = Instant::now();
        trip_breaker(&cb, now);

        let later = now + Duration::from_millis(200);
        assert_eq!(cb.check_at(later), BreakerDecision::Allow);
        assert_eq!(cb.state(), State::HalfOpen);
    }

    #[test]
    fn half_open_allows_probe_requests() {
        let cb = CircuitBreaker::new("test", test_config());
        let now = Instant::now();
        trip_breaker(&cb, now);
        let later = now + Duration::from_millis(200);
        assert_eq!(cb.check_at(later), BreakerDecision::Allow); // transitions to HalfOpen, first probe
        assert_eq!(cb.check_at(later), BreakerDecision::Allow); // second probe (switch_off_threshold=2, probe_count was 1)
    }

    #[test]
    fn half_open_to_closed_after_successes() {
        let cb = CircuitBreaker::new("test", test_config());
        let now = Instant::now();
        trip_breaker(&cb, now);
        let later = now + Duration::from_millis(200);
        assert_eq!(cb.check_at(later), BreakerDecision::Allow);
        assert_eq!(cb.state(), State::HalfOpen);

        cb.record_success_at(later);
        cb.record_success_at(later);
        assert_eq!(cb.state(), State::Closed);
    }

    #[test]
    fn half_open_to_open_on_any_fault() {
        let cb = CircuitBreaker::new("test", test_config());
        let now = Instant::now();
        trip_breaker(&cb, now);
        let later = now + Duration::from_millis(200);
        assert_eq!(cb.check_at(later), BreakerDecision::Allow);
        assert_eq!(cb.state(), State::HalfOpen);

        cb.record_failure_at(later);
        assert_eq!(cb.state(), State::Open);
    }

    #[test]
    fn half_open_caps_probe_requests() {
        let cb = CircuitBreaker::new("test", test_config());
        let now = Instant::now();
        trip_breaker(&cb, now);
        let later = now + Duration::from_millis(200);
        assert_eq!(cb.check_at(later), BreakerDecision::Allow); // probe 1 (transitions to HalfOpen)
        assert_eq!(cb.check_at(later), BreakerDecision::Allow); // probe 2 (switch_off_threshold=2)
        assert_eq!(cb.check_at(later), BreakerDecision::Reject); // probe cap reached
    }

    #[test]
    fn fault_window_resets_count() {
        let cb = CircuitBreaker::new("test", test_config());
        let now = Instant::now();
        cb.record_failure_at(now);
        cb.record_failure_at(now);

        // Advance past fault window
        let later = now + Duration::from_millis(600);
        cb.record_failure_at(later);
        // Only 1 fault in new window, below threshold
        assert_eq!(cb.state(), State::Closed);
    }

    #[test]
    fn fault_window_boundary() {
        let cb = CircuitBreaker::new("test", test_config());
        let t0 = Instant::now();
        cb.record_failure_at(t0);
        cb.record_failure_at(t0);

        // Just past the window — old faults expire, counter resets
        let t1 = t0 + Duration::from_millis(501);
        cb.record_failure_at(t1);
        assert_eq!(cb.state(), State::Closed);

        // Two more in the new window → threshold met
        cb.record_failure_at(t1);
        cb.record_failure_at(t1);
        assert_eq!(cb.state(), State::Open);
    }

    #[test]
    fn record_failure_when_open_is_noop() {
        let cb = CircuitBreaker::new("test", test_config());
        let now = Instant::now();
        trip_breaker(&cb, now);

        cb.record_failure_at(now);
        assert_eq!(cb.state(), State::Open);
    }

    #[test]
    fn record_success_in_closed_is_noop() {
        let cb = CircuitBreaker::new("test", test_config());
        cb.record_success();
        assert_eq!(cb.state(), State::Closed);
    }

    #[test]
    fn threshold_one_trips_immediately() {
        let config = BreakerConfig {
            switch_on_threshold: 1,
            ..test_config()
        };
        let cb = CircuitBreaker::new("test", config);
        cb.record_failure();
        assert_eq!(cb.state(), State::Open);
    }

    #[test]
    fn full_lifecycle() {
        let cb = CircuitBreaker::new("test", test_config());
        let t0 = Instant::now();

        // Closed → Open
        trip_breaker(&cb, t0);

        // Open → HalfOpen
        let t1 = t0 + Duration::from_millis(200);
        assert_eq!(cb.check_at(t1), BreakerDecision::Allow);
        assert_eq!(cb.state(), State::HalfOpen);

        // HalfOpen → Open (fault during probe)
        cb.record_failure_at(t1);
        assert_eq!(cb.state(), State::Open);

        // Open → HalfOpen again
        let t2 = t1 + Duration::from_millis(200);
        assert_eq!(cb.check_at(t2), BreakerDecision::Allow);
        assert_eq!(cb.state(), State::HalfOpen);

        // HalfOpen → Closed (successful probes)
        cb.record_success_at(t2);
        cb.record_success_at(t2);
        assert_eq!(cb.state(), State::Closed);
    }

    #[test]
    fn rapid_transitions_do_not_corrupt_state() {
        let cb = CircuitBreaker::new("test", test_config());
        let t0 = Instant::now();

        for i in 0..10 {
            let t = t0 + Duration::from_millis(i);
            cb.record_failure_at(t);
            let _ = cb.check_at(t);
            cb.record_success_at(t);
        }

        // After 10 rounds with threshold=3: early failures trip the breaker,
        // but check_at without elapsed reset_period keeps it Open; record_success
        // while Open is a no-op (only HalfOpen→Closed transitions on success).
        // Final state must be Open because successes can't reset without a probe.
        assert_eq!(cb.state(), State::Open);
    }

    #[tokio::test]
    async fn concurrent_access() {
        const N_THREADS: u32 = 20;
        const OPS_PER_THREAD: u32 = 50;
        let threshold = 100;

        let cb = Arc::new(CircuitBreaker::new(
            "test",
            BreakerConfig {
                switch_on_threshold: threshold,
                ..test_config()
            },
        ));

        let mut handles = vec![];
        for _ in 0..N_THREADS {
            let cb = Arc::clone(&cb);
            handles.push(tokio::spawn(async move {
                for _ in 0..OPS_PER_THREAD {
                    let _ = cb.check();
                    cb.record_failure();
                    cb.record_success();
                }
            }));
        }

        for h in handles {
            h.await.unwrap();
        }

        let state = cb.state();
        assert!(
            matches!(state, State::Closed | State::Open | State::HalfOpen),
            "invalid state after concurrent access: {state:?}"
        );
        assert!(
            cb.fault_count() <= threshold + N_THREADS,
            "fault_count {} exceeds theoretical max {}",
            cb.fault_count(),
            threshold + N_THREADS
        );
    }

    #[tokio::test]
    async fn concurrent_mixed_outcomes() {
        let cb = Arc::new(CircuitBreaker::new(
            "test",
            BreakerConfig {
                switch_on_threshold: 50,
                switch_off_threshold: 10,
                ..test_config()
            },
        ));

        let mut handles = vec![];
        for i in 0..20 {
            let cb = Arc::clone(&cb);
            handles.push(tokio::spawn(async move {
                for _ in 0..25 {
                    let _ = cb.check();
                    if i % 2 == 0 {
                        cb.record_failure();
                    } else {
                        cb.record_success();
                    }
                }
            }));
        }

        for h in handles {
            h.await.unwrap();
        }
    }

    // --- BucketRegistry tests ---

    fn buckets(names: &[&str]) -> Vec<String> {
        names.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn empty_buckets_config() {
        let reg = BucketRegistry::new(Vec::<String>::new(), &BreakerConfig::default()).unwrap();
        assert!(reg.is_empty());
    }

    #[test]
    fn declared_buckets_created_at_startup() {
        let reg =
            BucketRegistry::new(buckets(&["a", "b", "c"]), &BreakerConfig::default()).unwrap();
        reg.resolve(Some("a")).unwrap().unwrap();
        reg.resolve(Some("b")).unwrap().unwrap();
        reg.resolve(Some("c")).unwrap().unwrap();
    }

    #[test]
    fn lookup_by_name_exact_match() {
        let reg = BucketRegistry::new(buckets(&["monitoring"]), &BreakerConfig::default()).unwrap();
        let resolved = reg.resolve(Some("monitoring")).unwrap().unwrap();
        assert_eq!(resolved.bucket, "monitoring");
    }

    #[test]
    fn lookup_without_name_returns_none() {
        let reg = BucketRegistry::new(buckets(&["monitoring"]), &BreakerConfig::default()).unwrap();
        assert!(reg.resolve(None).unwrap().is_none());
    }

    #[test]
    fn lookup_unknown_name_returns_error() {
        let reg = BucketRegistry::new(buckets(&["monitoring"]), &BreakerConfig::default()).unwrap();
        let result = reg.resolve(Some("nonexistent"));
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("unknown circuit breaker bucket"));
    }

    #[test]
    fn bucket_uses_global_config() {
        let global = BreakerConfig {
            switch_on_threshold: 3,
            ..BreakerConfig::default()
        };
        let reg = BucketRegistry::new(buckets(&["monitoring"]), &global).unwrap();
        let resolved = reg.resolve(Some("monitoring")).unwrap().unwrap();
        trip_breaker(&resolved.breaker, Instant::now());
    }

    #[test]
    fn bucket_inherits_global_thresholds() {
        let global = BreakerConfig {
            switch_on_threshold: 7,
            ..BreakerConfig::default()
        };
        let reg = BucketRegistry::new(buckets(&["test"]), &global).unwrap();
        let resolved = reg.resolve(Some("test")).unwrap().unwrap();
        let now = Instant::now();
        for _ in 0..6 {
            resolved.breaker.record_failure_at(now);
        }
        assert_eq!(resolved.breaker.state(), State::Closed);
        resolved.breaker.record_failure_at(now);
        assert_eq!(resolved.breaker.state(), State::Open);
    }

    #[test]
    fn multiple_buckets_are_independent() {
        let reg = BucketRegistry::new(
            buckets(&["a", "b"]),
            &BreakerConfig {
                switch_on_threshold: 2,
                ..BreakerConfig::default()
            },
        )
        .unwrap();
        let a = reg.resolve(Some("a")).unwrap().unwrap();
        let b = reg.resolve(Some("b")).unwrap().unwrap();
        let now = Instant::now();
        a.breaker.record_failure_at(now);
        a.breaker.record_failure_at(now);
        assert_eq!(a.breaker.state(), State::Open);
        assert_eq!(b.breaker.state(), State::Closed);
    }

    #[test]
    fn duplicate_bucket_names_deduplicated() {
        let reg =
            BucketRegistry::new(buckets(&["a", "a", "b"]), &BreakerConfig::default()).unwrap();
        let a = reg.resolve(Some("a")).unwrap().unwrap();
        let b = reg.resolve(Some("b")).unwrap().unwrap();
        assert!(!Arc::ptr_eq(&a.breaker, &b.breaker));
    }

    // --- Config validation tests ---

    #[test]
    fn valid_config_passes_validation() {
        BreakerConfig::default().validate().unwrap();
        test_config().validate().unwrap();
    }

    #[test]
    fn zero_switch_on_threshold_rejected() {
        let config = BreakerConfig {
            switch_on_threshold: 0,
            ..BreakerConfig::default()
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn zero_switch_off_threshold_rejected() {
        let config = BreakerConfig {
            switch_off_threshold: 0,
            ..BreakerConfig::default()
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn zero_reset_period_rejected() {
        let config = BreakerConfig {
            reset_period: Duration::ZERO,
            ..BreakerConfig::default()
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn zero_fault_window_rejected() {
        let config = BreakerConfig {
            fault_window: Duration::ZERO,
            ..BreakerConfig::default()
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn registry_rejects_invalid_config() {
        let bad = BreakerConfig {
            switch_on_threshold: 0,
            ..BreakerConfig::default()
        };
        BucketRegistry::new(Vec::<String>::new(), &bad).unwrap_err();
    }
}
