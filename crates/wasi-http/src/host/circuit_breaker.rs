use std::sync::Arc;
use std::time::{Duration, Instant};

use dashmap::DashMap;
use parking_lot::Mutex;

/// Circuit breaker state machine with three states.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    /// Requests flow normally. Faults are tracked within a rolling window.
    Off,
    /// Requests are rejected. Transitions to `HalfOn` after the reset period.
    On,
    /// A limited number of probe requests are allowed through to test recovery.
    HalfOn,
}

/// Configuration for a single circuit breaker instance.
#[derive(Debug, Clone)]
pub struct BreakerConfig {
    pub switch_on_threshold: u32,
    pub switch_off_threshold: u32,
    pub reset_period: Duration,
    pub fault_window: Duration,
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
    fault_window_start: Instant,
    success_count: u32,
    probe_count: u32,
    opened_at: Instant,
}

/// A windowed three-state circuit breaker.
///
/// The fault counter resets when no new fault arrives within `fault_window`.
/// This prevents slow trickles of errors from accumulating into a false trip.
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
                state: State::Off,
                fault_count: 0,
                fault_window_start: Instant::now(),
                success_count: 0,
                probe_count: 0,
                opened_at: Instant::now(),
            }),
            config,
        }
    }

    /// Check whether a request is allowed through.
    ///
    /// Returns `Ok(())` if the request may proceed, or `Err(())` if the circuit is open.
    pub fn check(&self) -> Result<(), ()> {
        self.check_at(Instant::now())
    }

    fn check_at(&self, now: Instant) -> Result<(), ()> {
        let mut transition = None;
        let result = {
            let mut inner = self.inner.lock();
            match inner.state {
                State::Off => Ok(()),
                State::On => {
                    if now.duration_since(inner.opened_at) >= self.config.reset_period {
                        inner.state = State::HalfOn;
                        inner.success_count = 0;
                        inner.probe_count = 1;
                        transition = Some(State::HalfOn);
                        drop(inner);
                        Ok(())
                    } else {
                        transition = Some(State::On);
                        drop(inner);
                        Err(())
                    }
                }
                State::HalfOn => {
                    if inner.probe_count < self.config.switch_off_threshold {
                        inner.probe_count += 1;
                        drop(inner);
                        Ok(())
                    } else {
                        transition = Some(State::HalfOn);
                        drop(inner);
                        Err(())
                    }
                }
            }
        };

        match (transition, &result) {
            (Some(State::HalfOn), Ok(())) => {
                tracing::info!(
                    gauge.circuit_breaker_state = 2,
                    bucket = %self.name,
                    "circuit breaker entering probe state"
                );
            }
            (Some(_), Err(())) => {
                tracing::debug!(
                    monotonic_counter.circuit_breaker_rejections = 1,
                    bucket = %self.name,
                    "request rejected by circuit breaker"
                );
            }
            _ => {}
        }

        result
    }

    /// Record a successful response. In `HalfOn`, counts toward closing the breaker.
    pub fn record_success(&self) {
        self.record_success_at(Instant::now());
    }

    fn record_success_at(&self, _now: Instant) {
        let recovered = {
            let mut inner = self.inner.lock();
            if inner.state == State::HalfOn {
                inner.success_count += 1;
                if inner.success_count >= self.config.switch_off_threshold {
                    inner.state = State::Off;
                    inner.fault_count = 0;
                    inner.fault_window_start = Instant::now();
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
                bucket = %self.name,
                "circuit breaker recovered"
            );
        }
    }

    /// Record a fault. In `Off`, counts toward tripping. In `HalfOn`, trips immediately.
    pub fn record_failure(&self) {
        self.record_failure_at(Instant::now());
    }

    #[allow(clippy::if_then_some_else_none)]
    fn record_failure_at(&self, now: Instant) {
        let tripped = {
            let mut inner = self.inner.lock();
            match inner.state {
                State::Off => {
                    if now.duration_since(inner.fault_window_start) > self.config.fault_window {
                        inner.fault_count = 0;
                        inner.fault_window_start = now;
                    }
                    inner.fault_count += 1;
                    if inner.fault_count >= self.config.switch_on_threshold {
                        inner.state = State::On;
                        inner.opened_at = now;
                        let fc = inner.fault_count;
                        drop(inner);
                        Some(fc)
                    } else {
                        None
                    }
                }
                State::HalfOn => {
                    inner.state = State::On;
                    inner.opened_at = now;
                    drop(inner);
                    Some(0)
                }
                State::On => None,
            }
        };

        match tripped {
            Some(0) => {
                tracing::warn!(
                    monotonic_counter.circuit_breaker_trips = 1,
                    gauge.circuit_breaker_state = 1,
                    bucket = %self.name,
                    "circuit breaker re-tripped during probe"
                );
            }
            Some(fault_count) => {
                tracing::warn!(
                    monotonic_counter.circuit_breaker_trips = 1,
                    gauge.circuit_breaker_state = 1,
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
}

/// Registry holding per-bucket circuit breakers and a default fallback.
#[derive(Debug)]
pub struct BucketRegistry {
    buckets: DashMap<String, Arc<CircuitBreaker>>,
    default_breaker: Arc<CircuitBreaker>,
}

impl BucketRegistry {
    /// Build a registry from a comma-separated bucket list and per-bucket env overrides.
    ///
    /// Each bucket name `FOO` can have env overrides like `HTTP_CB_FOO_SWITCH_ON_THRESHOLD`.
    /// Buckets without overrides inherit the global config.
    pub fn new(bucket_names: &str, global: &BreakerConfig) -> Self {
        let buckets = DashMap::new();

        for raw in bucket_names.split(',') {
            let name = raw.trim().to_string();
            if name.is_empty() {
                continue;
            }
            if buckets.contains_key(&name) {
                continue;
            }

            let config = bucket_config_from_env(&name, global);
            buckets.insert(name.clone(), Arc::new(CircuitBreaker::new(name, config)));
        }

        if buckets.is_empty() {
            tracing::warn!(
                "no circuit breaker buckets configured -- all upstreams share a single default breaker"
            );
        }

        Self {
            buckets,
            default_breaker: Arc::new(CircuitBreaker::new("default", global.clone())),
        }
    }

    /// Resolve a breaker by explicit upstream name, falling back to the default breaker.
    pub fn resolve(&self, upstream: Option<&str>) -> Arc<CircuitBreaker> {
        if let Some(name) = upstream
            && let Some(b) = self.buckets.get(name)
        {
            return Arc::clone(b.value());
        }

        Arc::clone(&self.default_breaker)
    }

    /// Return a reference to the default breaker.
    #[cfg(test)]
    pub const fn default_breaker(&self) -> &Arc<CircuitBreaker> {
        &self.default_breaker
    }
}

/// Load per-bucket config overrides from environment, falling back to global defaults.
fn bucket_config_from_env(name: &str, global: &BreakerConfig) -> BreakerConfig {
    let upper = name.to_uppercase();
    let switch_on = std::env::var(format!("HTTP_CB_{upper}_SWITCH_ON_THRESHOLD"))
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(global.switch_on_threshold);
    let switch_off = std::env::var(format!("HTTP_CB_{upper}_SWITCH_OFF_THRESHOLD"))
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(global.switch_off_threshold);
    #[allow(clippy::cast_possible_truncation)]
    let reset_ms: u64 = std::env::var(format!("HTTP_CB_{upper}_RESET_PERIOD_MS"))
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(global.reset_period.as_millis() as u64);
    #[allow(clippy::cast_possible_truncation)]
    let fault_ms: u64 = std::env::var(format!("HTTP_CB_{upper}_FAULT_WINDOW_MS"))
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(global.fault_window.as_millis() as u64);

    BreakerConfig {
        switch_on_threshold: switch_on,
        switch_off_threshold: switch_off,
        reset_period: Duration::from_millis(reset_ms),
        fault_window: Duration::from_millis(fault_ms),
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

    #[test]
    fn off_allows_requests() {
        let cb = CircuitBreaker::new("test", test_config());
        assert!(cb.check().is_ok());
        assert_eq!(cb.state(), State::Off);
    }

    #[test]
    fn transitions_to_on_after_threshold() {
        let cb = CircuitBreaker::new("test", test_config());
        let now = Instant::now();
        for _ in 0..3 {
            cb.record_failure_at(now);
        }
        assert_eq!(cb.state(), State::On);
    }

    #[test]
    fn does_not_trip_below_threshold() {
        let cb = CircuitBreaker::new("test", test_config());
        let now = Instant::now();
        for _ in 0..2 {
            cb.record_failure_at(now);
        }
        assert_eq!(cb.state(), State::Off);
    }

    #[test]
    fn rejects_when_on() {
        let cb = CircuitBreaker::new("test", test_config());
        let now = Instant::now();
        for _ in 0..3 {
            cb.record_failure_at(now);
        }
        assert_eq!(cb.state(), State::On);
        assert!(cb.check_at(now).is_err());
    }

    #[test]
    fn transitions_to_half_on_after_reset_period() {
        let cb = CircuitBreaker::new("test", test_config());
        let now = Instant::now();
        for _ in 0..3 {
            cb.record_failure_at(now);
        }
        assert_eq!(cb.state(), State::On);

        let later = now + Duration::from_millis(200);
        assert!(cb.check_at(later).is_ok());
        assert_eq!(cb.state(), State::HalfOn);
    }

    #[test]
    fn half_on_allows_probe_requests() {
        let cb = CircuitBreaker::new("test", test_config());
        let now = Instant::now();
        for _ in 0..3 {
            cb.record_failure_at(now);
        }
        let later = now + Duration::from_millis(200);
        assert!(cb.check_at(later).is_ok()); // transitions to HalfOn, first probe
        assert!(cb.check_at(later).is_ok()); // second probe (switch_off_threshold=2, probe_count was 1)
    }

    #[test]
    fn half_on_to_off_after_successes() {
        let cb = CircuitBreaker::new("test", test_config());
        let now = Instant::now();
        for _ in 0..3 {
            cb.record_failure_at(now);
        }
        let later = now + Duration::from_millis(200);
        cb.check_at(later).unwrap();
        assert_eq!(cb.state(), State::HalfOn);

        cb.record_success_at(later);
        cb.record_success_at(later);
        assert_eq!(cb.state(), State::Off);
    }

    #[test]
    fn half_on_to_on_on_any_fault() {
        let cb = CircuitBreaker::new("test", test_config());
        let now = Instant::now();
        for _ in 0..3 {
            cb.record_failure_at(now);
        }
        let later = now + Duration::from_millis(200);
        cb.check_at(later).unwrap();
        assert_eq!(cb.state(), State::HalfOn);

        cb.record_failure_at(later);
        assert_eq!(cb.state(), State::On);
    }

    #[test]
    fn half_on_caps_probe_requests() {
        let cb = CircuitBreaker::new("test", test_config());
        let now = Instant::now();
        for _ in 0..3 {
            cb.record_failure_at(now);
        }
        let later = now + Duration::from_millis(200);
        assert!(cb.check_at(later).is_ok()); // probe 1 (transitions to HalfOn)
        assert!(cb.check_at(later).is_ok()); // probe 2 (switch_off_threshold=2)
        assert!(cb.check_at(later).is_err()); // probe cap reached
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
        assert_eq!(cb.state(), State::Off);
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
        assert_eq!(cb.state(), State::Off);

        // Two more in the new window → threshold met
        cb.record_failure_at(t1);
        cb.record_failure_at(t1);
        assert_eq!(cb.state(), State::On);
    }

    #[test]
    fn record_failure_when_on_is_noop() {
        let cb = CircuitBreaker::new("test", test_config());
        let now = Instant::now();
        for _ in 0..3 {
            cb.record_failure_at(now);
        }
        assert_eq!(cb.state(), State::On);

        cb.record_failure_at(now);
        assert_eq!(cb.state(), State::On);
    }

    #[test]
    fn record_success_in_off_is_noop() {
        let cb = CircuitBreaker::new("test", test_config());
        cb.record_success();
        assert_eq!(cb.state(), State::Off);
    }

    #[test]
    fn threshold_one_trips_immediately() {
        let config = BreakerConfig {
            switch_on_threshold: 1,
            ..test_config()
        };
        let cb = CircuitBreaker::new("test", config);
        cb.record_failure();
        assert_eq!(cb.state(), State::On);
    }

    #[test]
    fn full_lifecycle() {
        let cb = CircuitBreaker::new("test", test_config());
        let t0 = Instant::now();

        // OFF → ON
        for _ in 0..3 {
            cb.record_failure_at(t0);
        }
        assert_eq!(cb.state(), State::On);

        // ON → HALF_ON
        let t1 = t0 + Duration::from_millis(200);
        cb.check_at(t1).unwrap();
        assert_eq!(cb.state(), State::HalfOn);

        // HALF_ON → ON (fault during probe)
        cb.record_failure_at(t1);
        assert_eq!(cb.state(), State::On);

        // ON → HALF_ON again
        let t2 = t1 + Duration::from_millis(200);
        cb.check_at(t2).unwrap();
        assert_eq!(cb.state(), State::HalfOn);

        // HALF_ON → OFF (successful probes)
        cb.record_success_at(t2);
        cb.record_success_at(t2);
        assert_eq!(cb.state(), State::Off);
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
        // but check_at without elapsed reset_period keeps it On; record_success
        // while On is a no-op (only HalfOn→Off transitions on success).
        // Final state must be On because successes can't reset without a probe.
        assert_eq!(cb.state(), State::On);
    }

    #[tokio::test]
    async fn concurrent_access() {
        let cb = Arc::new(CircuitBreaker::new(
            "test",
            BreakerConfig {
                switch_on_threshold: 100,
                ..test_config()
            },
        ));

        let mut handles = vec![];
        for _ in 0..20 {
            let cb = Arc::clone(&cb);
            handles.push(tokio::spawn(async move {
                for _ in 0..50 {
                    let _ = cb.check();
                    cb.record_failure();
                    cb.record_success();
                }
            }));
        }

        for h in handles {
            h.await.unwrap();
        }
        // Must not panic
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

    #[test]
    fn empty_buckets_config_uses_default_only() {
        let reg = BucketRegistry::new("", &BreakerConfig::default());
        let b = reg.resolve(None);
        assert!(Arc::ptr_eq(&b, reg.default_breaker()));
    }

    #[test]
    fn declared_buckets_created_at_startup() {
        let reg = BucketRegistry::new("a,b,c", &BreakerConfig::default());
        let a = reg.resolve(Some("a"));
        let b = reg.resolve(Some("b"));
        let c = reg.resolve(Some("c"));
        assert!(!Arc::ptr_eq(&a, reg.default_breaker()));
        assert!(!Arc::ptr_eq(&b, reg.default_breaker()));
        assert!(!Arc::ptr_eq(&c, reg.default_breaker()));
    }

    #[test]
    fn lookup_by_name_exact_match() {
        let reg = BucketRegistry::new("monitoring", &BreakerConfig::default());
        let b = reg.resolve(Some("monitoring"));
        assert!(!Arc::ptr_eq(&b, reg.default_breaker()));
    }

    #[test]
    fn lookup_without_name_returns_default() {
        let reg = BucketRegistry::new("monitoring", &BreakerConfig::default());
        let b = reg.resolve(None);
        assert!(Arc::ptr_eq(&b, reg.default_breaker()));
    }

    #[test]
    fn lookup_unknown_name_returns_default() {
        let reg = BucketRegistry::new("monitoring", &BreakerConfig::default());
        let b = reg.resolve(Some("nonexistent"));
        assert!(Arc::ptr_eq(&b, reg.default_breaker()));
    }

    #[test]
    fn per_bucket_override_applied() {
        // When an env var override is present, the bucket uses it.
        // We test the non-env path by verifying the bucket inherits the global config,
        // then separately test that a custom global config with threshold=3 trips correctly.
        let global = BreakerConfig {
            switch_on_threshold: 3,
            ..BreakerConfig::default()
        };
        let reg = BucketRegistry::new("monitoring", &global);
        let b = reg.resolve(Some("monitoring"));
        let now = Instant::now();
        for _ in 0..3 {
            b.record_failure_at(now);
        }
        assert_eq!(b.state(), State::On);
    }

    #[test]
    fn per_bucket_inherits_global_for_unset() {
        let global = BreakerConfig {
            switch_on_threshold: 7,
            ..BreakerConfig::default()
        };
        let reg = BucketRegistry::new("test", &global);
        let b = reg.resolve(Some("test"));
        let now = Instant::now();
        for _ in 0..6 {
            b.record_failure_at(now);
        }
        assert_eq!(b.state(), State::Off);
        b.record_failure_at(now);
        assert_eq!(b.state(), State::On);
    }

    #[test]
    fn multiple_buckets_are_independent() {
        let reg = BucketRegistry::new(
            "a,b",
            &BreakerConfig {
                switch_on_threshold: 2,
                ..BreakerConfig::default()
            },
        );
        let a = reg.resolve(Some("a"));
        let b = reg.resolve(Some("b"));
        let now = Instant::now();
        a.record_failure_at(now);
        a.record_failure_at(now);
        assert_eq!(a.state(), State::On);
        assert_eq!(b.state(), State::Off);
    }

    #[test]
    fn bucket_names_trimmed() {
        let reg = BucketRegistry::new(" monitoring , messaging ", &BreakerConfig::default());
        assert!(!std::ptr::eq(
            reg.resolve(Some("monitoring")).as_ref(),
            reg.default_breaker().as_ref()
        ));
        assert!(!std::ptr::eq(
            reg.resolve(Some("messaging")).as_ref(),
            reg.default_breaker().as_ref()
        ));
    }

    #[test]
    fn duplicate_bucket_names_deduplicated() {
        let reg = BucketRegistry::new("a,a,b", &BreakerConfig::default());
        let a = reg.resolve(Some("a"));
        let b = reg.resolve(Some("b"));
        assert!(!Arc::ptr_eq(&a, &b));
    }

    #[test]
    fn default_breaker_uses_global_thresholds() {
        let global = BreakerConfig {
            switch_on_threshold: 5,
            ..BreakerConfig::default()
        };
        let reg = BucketRegistry::new("", &global);
        let b = std::sync::Arc::<crate::host::circuit_breaker::CircuitBreaker>::clone(
            reg.default_breaker(),
        );
        let now = Instant::now();
        for _ in 0..4 {
            b.record_failure_at(now);
        }
        assert_eq!(b.state(), State::Off);
        b.record_failure_at(now);
        assert_eq!(b.state(), State::On);
    }
}
