//! Live compute-unit usage per account.
//!
//! Colab only reports account totals (balance, burn rate, number of
//! assignments). This module turns successive readings into what the UI shows:
//!
//! * [`UsageSnapshot`] — the last reading, marked `stale` when a later fetch
//!   failed. Unknown is never treated as zero.
//! * [`poll_due`] — the cadence from [`Defaults`] (30 s while the account has
//!   sessions, 5 min otherwise by default; manual refresh is always allowed).
//! * [`RateAttribution`] — an *estimated* per-session burn rate, taken from the
//!   change of the account rate observed when sessions start or stop, split
//!   evenly when ambiguous. All per-session figures are estimates (`≈`).

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::time::{Duration, SystemTime};

use serde::{Deserialize, Serialize};

use crate::clock::serde_secs;
use crate::defaults::Defaults;
use crate::error::Error;
use crate::sessions::Accelerator;
/// Default capacity of [`BalanceHistory`] (one hour at 30 s).
pub const HISTORY_CAPACITY: usize = 120;

/// A rate (CU/h) at or below this is rounding error, not a runtime burning.
const NO_RATE: f64 = 1e-9;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Usage {
    pub balance_cu: f64,
    /// Current burn rate of the whole account in CU per hour.
    pub rate_cu_per_hour: f64,
    /// Active assignments (runtimes) on the account, managed or not.
    pub assignments: u32,
}

impl Usage {
    /// The session this reading measures: the only managed session of an
    /// account with a single runtime burns the whole account rate. `None`
    /// while nothing burns (the account rate can lag behind a new runtime).
    pub fn sole_session<'a>(&self, live: &'a BTreeSet<String>) -> Option<&'a str> {
        let burning = self.rate_cu_per_hour.is_finite() && self.rate_cu_per_hour > NO_RATE;
        if self.assignments != 1 || live.len() != 1 || !burning {
            return None;
        }
        live.first().map(String::as_str)
    }
}

/// What users who measured this hardware published as its burn rate (CU/h),
/// for hardware the app has not measured itself. Colab publishes no rates and
/// changes them, so this is only where the figure starts: a reading of the
/// app's own replaces it ([`Usage::sole_session`]).
///
/// From the table measured at
/// <https://mcp-tool-shop-org.github.io/rnd/handbook/colab-golden-path/>; the
/// TPUs from <https://note.com/okakutech/n/nfb368939d8d4>, the high-RAM T4 and
/// the H100 from <https://www.gkbrk.com/google-colab>. The G4 and the H100
/// were published for one machine shape, which stands for both.
pub fn published_rate(accelerator: Accelerator, high_mem: bool) -> f64 {
    let high_mem = high_mem && accelerator.supports_high_mem();
    match (accelerator, high_mem) {
        (Accelerator::None, false) => 0.08,
        (Accelerator::None, true) => 0.26,
        (Accelerator::T4, false) => 1.07,
        (Accelerator::T4, true) => 1.41,
        (Accelerator::L4, _) => 1.54,
        (Accelerator::G4, _) => 8.90,
        (Accelerator::A100, false) => 5.30,
        (Accelerator::A100, true) => 6.77,
        (Accelerator::H100, _) => 18.05,
        (Accelerator::TpuV5e1, _) => 2.92,
        (Accelerator::TpuV6e1, _) => 4.08,
    }
}

/// A burn rate Colab reported, and when.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct RateReading {
    pub cu_per_hour: f64,
    #[serde(with = "serde_secs")]
    pub at: SystemTime,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct UsageSnapshot {
    pub balance_cu: f64,
    pub rate_cu_per_hour: f64,
    pub assignments: u32,
    #[serde(with = "serde_secs")]
    pub fetched_at: SystemTime,
    /// A later fetch failed: the values are the last known ones.
    pub stale: bool,
}

impl UsageSnapshot {
    pub fn new(usage: Usage, at: SystemTime) -> Self {
        UsageSnapshot {
            balance_cu: usage.balance_cu,
            rate_cu_per_hour: usage.rate_cu_per_hour,
            assignments: usage.assignments,
            fetched_at: at,
            stale: false,
        }
    }

    pub fn usage(&self) -> Usage {
        Usage {
            balance_cu: self.balance_cu,
            rate_cu_per_hour: self.rate_cu_per_hour,
            assignments: self.assignments,
        }
    }

    /// Time until the balance reaches zero at the current rate: `balance / rate`.
    /// `None` when nothing is burning; zero when the balance is exhausted.
    pub fn eta(&self) -> Option<Duration> {
        if self.balance_cu <= 0.0 {
            return Some(Duration::ZERO);
        }
        if self.rate_cu_per_hour <= 0.0 || !self.rate_cu_per_hour.is_finite() {
            return None;
        }
        Some(Duration::from_secs_f64(
            (self.balance_cu / self.rate_cu_per_hour * 3600.0).min(1e12),
        ))
    }

    /// Balance extrapolated to `now` with the sampled rate.
    pub fn projected_balance(&self, now: SystemTime) -> f64 {
        let age_h = now
            .duration_since(self.fetched_at)
            .unwrap_or_default()
            .as_secs_f64()
            / 3600.0;
        self.balance_cu - self.rate_cu_per_hour.max(0.0) * age_h
    }

    /// Not stale and at most `max_age` old.
    pub fn is_fresh(&self, now: SystemTime, max_age: Duration) -> bool {
        !self.stale && now.duration_since(self.fetched_at).unwrap_or_default() <= max_age
    }
}

/// True when a poll is due given the last attempt (successful or not) and
/// the cadence in `defaults`.
pub fn poll_due(
    last_attempt: Option<SystemTime>,
    now: SystemTime,
    has_sessions: bool,
    defaults: &Defaults,
) -> bool {
    match last_attempt {
        None => true,
        Some(t) => {
            now.duration_since(t).unwrap_or_default() >= defaults.usage_poll_interval(has_sessions)
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct BalancePoint {
    #[serde(with = "serde_secs")]
    pub at: SystemTime,
    pub balance_cu: f64,
}

/// A bounded history of balances (oldest dropped first).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BalanceHistory {
    capacity: usize,
    points: VecDeque<BalancePoint>,
}

impl Default for BalanceHistory {
    fn default() -> Self {
        BalanceHistory::with_capacity(HISTORY_CAPACITY)
    }
}

impl BalanceHistory {
    /// An empty history keeping at most `capacity` points (minimum 1).
    pub fn with_capacity(capacity: usize) -> Self {
        BalanceHistory {
            capacity: capacity.max(1),
            points: VecDeque::new(),
        }
    }

    /// Appends a point; a point at the same instant replaces the last one.
    pub fn push(&mut self, at: SystemTime, balance_cu: f64) {
        if let Some(last) = self.points.back_mut()
            && last.at == at
        {
            last.balance_cu = balance_cu;
            return;
        }
        self.points.push_back(BalancePoint { at, balance_cu });
        while self.points.len() > self.capacity {
            self.points.pop_front();
        }
    }

    /// Points, oldest first.
    pub fn points(&self) -> Vec<BalancePoint> {
        self.points.iter().copied().collect()
    }

    pub fn len(&self) -> usize {
        self.points.len()
    }

    pub fn is_empty(&self) -> bool {
        self.points.is_empty()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RateAttribution {
    last_rate: Option<f64>,
    rates: BTreeMap<String, Option<f64>>,
}

impl RateAttribution {
    /// `live` are the app-managed sessions running now. Sessions that appeared
    /// since the previous reading share the rate increase (plus whatever the
    /// sessions that disappeared were estimated at) evenly. Sessions without
    /// an estimate (first reading, no positive increase) share whatever rate
    /// is not yet explained, evenly with the unmanaged runtimes
    /// (`assignments` beyond `live`). A zero share leaves a session unknown,
    /// to be worked out again at the next reading: the account rate can lag
    /// behind a new runtime, and a zero kept for good would hide its spend
    /// from the cost guard.
    pub fn observe(&mut self, account_rate: f64, assignments: u32, live: &BTreeSet<String>) {
        let rate = account_rate.max(0.0);
        let mut freed = 0.0;
        self.rates.retain(|name, r| {
            if live.contains(name) {
                true
            } else {
                freed += r.unwrap_or(0.0);
                false
            }
        });
        let started: Vec<String> = live
            .iter()
            .filter(|n| !self.rates.contains_key(*n))
            .cloned()
            .collect();
        let delta_share = match self.last_rate {
            Some(prev) if !started.is_empty() => {
                let delta = rate - prev + freed;
                (delta > NO_RATE).then(|| delta / started.len() as f64)
            }
            _ => None,
        };
        for name in started {
            self.rates.insert(name, delta_share);
        }
        let unknown: Vec<String> = self
            .rates
            .iter()
            // A cache written before zero shares were left unknown can hold one.
            .filter(|(_, r)| r.is_none_or(|r| r <= NO_RATE))
            .map(|(n, _)| n.clone())
            .collect();
        if !unknown.is_empty() {
            let known: f64 = self.rates.values().flatten().sum();
            let unmanaged = (assignments as usize).saturating_sub(live.len());
            let share = (rate - known).max(0.0) / (unknown.len() + unmanaged) as f64;
            if share > NO_RATE {
                for name in unknown {
                    self.rates.insert(name, Some(share));
                }
            }
        }
        self.last_rate = Some(rate);
    }

    /// Estimated rate of a session (CU/h).
    pub fn rate_of(&self, name: &str) -> Option<f64> {
        self.rates.get(name).copied().flatten()
    }

    pub fn rates(&self) -> BTreeMap<String, f64> {
        self.rates
            .iter()
            .filter_map(|(n, r)| r.map(|r| (n.clone(), r)))
            .collect()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AccountUsage {
    pub snapshot: Option<UsageSnapshot>,
    /// Managed runtimes when the snapshot was recorded; absent in older caches.
    #[serde(default)]
    pub managed_at_reading: Option<usize>,
    #[serde(default)]
    pub ownership_verified: bool,
    pub history: BalanceHistory,
    pub attribution: RateAttribution,
    /// Last fetch attempt (successful or not), for [`poll_due`].
    #[serde(default, with = "serde_secs::option")]
    pub last_attempt: Option<SystemTime>,
    pub last_error: Option<Error>,
}

impl AccountUsage {
    pub fn record_success(&mut self, usage: Usage, now: SystemTime, live: &BTreeSet<String>) {
        self.snapshot = Some(UsageSnapshot::new(usage, now));
        self.managed_at_reading = Some(live.len());
        self.ownership_verified = true;
        self.history.push(now, usage.balance_cu);
        self.attribution
            .observe(usage.rate_cu_per_hour, usage.assignments, live);
        self.last_attempt = Some(now);
        self.last_error = None;
    }

    pub fn record_failure(&mut self, error: Error, now: SystemTime) {
        if let Some(s) = self.snapshot.as_mut() {
            s.stale = true;
        }
        self.last_attempt = Some(now);
        self.last_error = Some(error);
    }

    /// Runtimes beyond those managed at the reading (`known` is a legacy-cache fallback).
    pub fn unmanaged_runtimes(&self, known: usize) -> u32 {
        self.snapshot
            .map(|s| {
                s.assignments
                    .saturating_sub(self.managed_at_reading.unwrap_or(known) as u32)
            })
            .unwrap_or(0)
    }
}

/// Sums balances and rates of the accounts whose usage is known. Unknown
/// accounts are skipped, never counted as zero; `complete` says whether all were known.
pub fn totals<'a>(snapshots: impl IntoIterator<Item = Option<&'a UsageSnapshot>>) -> UsageTotals {
    let mut t = UsageTotals {
        balance_cu: 0.0,
        rate_cu_per_hour: 0.0,
        accounts: 0,
        complete: true,
        any_stale: false,
    };
    for s in snapshots {
        match s {
            Some(s) => {
                t.balance_cu += s.balance_cu;
                t.rate_cu_per_hour += s.rate_cu_per_hour;
                t.accounts += 1;
                t.any_stale |= s.stale;
            }
            None => t.complete = false,
        }
    }
    t
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct UsageTotals {
    pub balance_cu: f64,
    pub rate_cu_per_hour: f64,
    pub accounts: u32,
    /// False if some account's usage is unknown.
    pub complete: bool,
    /// True if some included snapshot is stale.
    pub any_stale: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(s: u64) -> SystemTime {
        SystemTime::UNIX_EPOCH + Duration::from_secs(1_800_000_000 + s)
    }
    fn set(names: &[&str]) -> BTreeSet<String> {
        names.iter().map(|s| s.to_string()).collect()
    }
    fn u(b: f64, r: f64, n: u32) -> Usage {
        Usage {
            balance_cu: b,
            rate_cu_per_hour: r,
            assignments: n,
        }
    }

    #[test]
    fn cadence() {
        let d = Defaults::default();
        assert!(poll_due(None, t(0), false, &d));
        assert!(!poll_due(Some(t(0)), t(29), true, &d));
        assert!(poll_due(Some(t(0)), t(30), true, &d));
        assert!(!poll_due(Some(t(0)), t(299), false, &d));
        assert!(poll_due(Some(t(0)), t(300), false, &d));
        let fast = Defaults {
            usage_poll_active: Duration::from_secs(5),
            ..Defaults::default()
        };
        assert!(poll_due(Some(t(0)), t(5), true, &fast));
    }

    #[test]
    fn a_reading_measures_the_only_runtime_of_an_account() {
        let one = set(&["s1"]);
        assert_eq!(u(50.0, 4.82, 1).sole_session(&one), Some("s1"));
        // A runtime the app does not manage, a second session, or no rate yet.
        assert_eq!(u(50.0, 6.58, 2).sole_session(&one), None);
        assert_eq!(u(50.0, 6.58, 2).sole_session(&set(&["s1", "s2"])), None);
        assert_eq!(u(50.0, 0.0, 1).sole_session(&one), None);
        assert_eq!(u(50.0, 4.82, 1).sole_session(&set(&[])), None);
    }

    #[test]
    fn published_rates_follow_the_machine_shape() {
        assert_eq!(published_rate(Accelerator::A100, false), 5.30);
        assert_eq!(published_rate(Accelerator::A100, true), 6.77);
        // An L4 has one machine shape, so the high-RAM option changes nothing.
        assert_eq!(
            published_rate(Accelerator::L4, true),
            published_rate(Accelerator::L4, false)
        );
    }

    #[test]
    fn eta_and_projection() {
        let s = UsageSnapshot::new(u(10.0, 4.0, 1), t(0));
        assert_eq!(s.eta(), Some(Duration::from_secs(9000)));
        assert_eq!(UsageSnapshot::new(u(10.0, 0.0, 0), t(0)).eta(), None);
        assert_eq!(
            UsageSnapshot::new(u(0.0, 4.0, 1), t(0)).eta(),
            Some(Duration::ZERO)
        );
        assert!((s.projected_balance(t(1800)) - 8.0).abs() < 1e-9);
        assert!(s.is_fresh(t(60), Duration::from_secs(300)));
        assert!(!s.is_fresh(t(301), Duration::from_secs(300)));
    }

    #[test]
    fn failure_marks_stale_and_keeps_values() {
        let mut a = AccountUsage::default();
        a.record_failure(Error::new(crate::ErrorKind::Http), t(0));
        assert!(a.snapshot.is_none(), "unknown stays unknown, not zero");
        a.record_success(u(50.0, 5.0, 1), t(30), &set(&["s1"]));
        a.record_failure(Error::new(crate::ErrorKind::Http), t(60));
        let s = a.snapshot.unwrap();
        assert!(s.stale);
        assert_eq!(s.balance_cu, 50.0);
        assert_eq!(a.last_attempt, Some(t(60)));
        assert!(!s.is_fresh(t(61), Duration::from_secs(300)));
    }

    #[test]
    fn history_is_a_ring_buffer() {
        let mut h = BalanceHistory::with_capacity(3);
        for i in 0..5 {
            h.push(t(i), i as f64);
        }
        h.push(t(4), 40.0);
        let p = h.points();
        assert_eq!(p.len(), 3);
        assert_eq!(p[0].balance_cu, 2.0);
        assert_eq!(p[2].balance_cu, 40.0);
    }

    #[test]
    fn attribution_from_rate_deltas() {
        let mut a = RateAttribution::default();
        a.observe(0.0, 0, &set(&[]));
        // s1 starts: rate goes 0 -> 11.8, all of it is s1.
        a.observe(11.8, 1, &set(&["s1"]));
        assert_eq!(a.rate_of("s1"), Some(11.8));
        // s2 (a T4) starts: +1.8.
        a.observe(13.6, 2, &set(&["s1", "s2"]));
        assert!((a.rate_of("s2").unwrap() - 1.8).abs() < 1e-9);
        assert_eq!(a.rate_of("s1"), Some(11.8));
        // s1 stops and s3 starts in the same interval: s3 gets the change plus s1's freed rate.
        a.observe(7.0, 2, &set(&["s2", "s3"]));
        assert!((a.rate_of("s3").unwrap() - 5.2).abs() < 1e-9);
        assert_eq!(a.rate_of("s1"), None);
    }

    #[test]
    fn ambiguous_starts_split_evenly_with_unmanaged() {
        let mut a = RateAttribution::default();
        // First reading ever: two sessions plus one unmanaged runtime share 9 CU/h.
        a.observe(9.0, 3, &set(&["a", "b"]));
        assert_eq!(a.rate_of("a"), Some(3.0));
        assert_eq!(a.rate_of("b"), Some(3.0));
        // Two start together: the increase is split evenly.
        a.observe(13.0, 5, &set(&["a", "b", "c", "d"]));
        assert_eq!(a.rate_of("c"), Some(2.0));
        assert_eq!(a.rate_of("d"), Some(2.0));
        // A start without a rate increase (already billed at the previous
        // reading) falls back to the unexplained remainder.
        let mut b = RateAttribution::default();
        b.observe(5.0, 1, &set(&[]));
        b.observe(5.0, 1, &set(&["late"]));
        assert_eq!(b.rate_of("late"), Some(5.0));
    }

    #[test]
    fn a_start_the_rate_lags_behind_stays_unknown_until_its_rate_shows() {
        let mut a = RateAttribution::default();
        a.observe(0.0, 0, &set(&[]));
        a.observe(0.0, 1, &set(&["h"]));
        assert_eq!(a.rate_of("h"), None);
        a.observe(0.0, 1, &set(&["h"]));
        assert_eq!(a.rate_of("h"), None);
        a.observe(18.3, 1, &set(&["h"]));
        assert_eq!(a.rate_of("h"), Some(18.3));
        // Beside a known session it gets the remainder once that shows.
        a.observe(18.3, 2, &set(&["h", "t"]));
        assert_eq!(a.rate_of("t"), None);
        a.observe(20.06, 2, &set(&["h", "t"]));
        assert!((a.rate_of("t").unwrap() - 1.76).abs() < 1e-9);
        assert_eq!(a.rate_of("h"), Some(18.3));
        // On the first reading too.
        let mut first = RateAttribution::default();
        first.observe(0.0, 1, &set(&["x"]));
        assert_eq!(first.rate_of("x"), None);
        assert!(first.rates().is_empty());
    }

    #[test]
    fn a_lagging_start_beside_an_unmanaged_runtime_takes_its_share_at_once() {
        // An estimate the spend cap can stop on, rather than unknown for as long as it runs.
        let mut a = RateAttribution::default();
        a.observe(5.0, 1, &set(&[]));
        a.observe(5.0, 2, &set(&["m"]));
        assert_eq!(a.rate_of("m"), Some(2.5));
    }

    #[test]
    fn spend_of_a_start_the_rate_lags_behind_counts_from_its_start() {
        use crate::guard::{Concern, Decision, GuardPolicy, Limit, SpendMeter, evaluate};
        let policy = GuardPolicy {
            max_spend_cu: Some(9.0),
            ..GuardPolicy::unlimited()
        };
        let mut a = RateAttribution::default();
        let mut meter = SpendMeter::new(t(0));
        a.observe(0.0, 0, &set(&[]));
        a.observe(0.0, 1, &set(&["h"]));
        meter.record(t(30), a.rate_of("h"));
        assert_eq!(
            evaluate(&policy, t(30), None, &meter),
            Decision::Warn(vec![Concern::UsageUnknown {
                limit: Limit::Spend
            }])
        );
        a.observe(18.3, 1, &set(&["h"]));
        meter.record(t(1800), a.rate_of("h"));
        assert!((meter.spend().unwrap() - 9.15).abs() < 1e-9);
        assert!(matches!(
            evaluate(&policy, t(1800), None, &meter),
            Decision::Stop(_)
        ));
    }

    #[test]
    fn a_zero_share_from_an_older_cache_is_worked_out_again() {
        let mut a: RateAttribution =
            serde_json::from_value(serde_json::json!({"last_rate": 0.0, "rates": {"h": 0.0}}))
                .unwrap();
        a.observe(18.3, 1, &set(&["h"]));
        assert_eq!(a.rate_of("h"), Some(18.3));
    }

    #[test]
    fn unmanaged_and_totals() {
        let mut a = AccountUsage::default();
        a.record_success(u(80.0, 13.6, 3), t(0), &set(&["s1", "s2"]));
        assert_eq!(a.unmanaged_runtimes(2), 1);
        let s1 = UsageSnapshot::new(u(10.0, 1.0, 1), t(0));
        let mut s2 = UsageSnapshot::new(u(5.0, 2.0, 1), t(0));
        s2.stale = true;
        let tot = totals([Some(&s1), Some(&s2), None]);
        assert_eq!(tot.balance_cu, 15.0);
        assert_eq!(tot.accounts, 2);
        assert!(!tot.complete);
        assert!(tot.any_stale);
    }

    #[test]
    fn unmanaged_count_stays_with_the_reading() {
        let mut a = AccountUsage::default();
        assert_eq!(a.unmanaged_runtimes(0), 0);
        a.record_success(u(80.0, 13.6, 3), t(0), &set(&["s1", "s2"]));
        assert_eq!(
            a.unmanaged_runtimes(1),
            1,
            "a stop does not change the reading"
        );
        assert_eq!(
            a.unmanaged_runtimes(3),
            1,
            "a start does not hide a runtime"
        );
        a.record_failure(Error::new(crate::ErrorKind::Http), t(30));
        assert_eq!(a.unmanaged_runtimes(3), 1);
        a.record_success(u(79.0, 13.6, 5), t(60), &set(&["s1", "s2", "s3"]));
        assert_eq!(
            a.unmanaged_runtimes(0),
            2,
            "a successful reading updates the count"
        );
        a.record_success(u(78.0, 13.6, 1), t(90), &set(&["s1", "s2"]));
        assert_eq!(a.unmanaged_runtimes(0), 0, "subtraction saturates");
    }

    #[test]
    fn older_usage_caches_fall_back_to_known_sessions() {
        let mut a = AccountUsage::default();
        a.record_success(u(80.0, 13.6, 3), t(0), &set(&["s1", "s2"]));
        let mut value = serde_json::to_value(&a).unwrap();
        value.as_object_mut().unwrap().remove("managed_at_reading");
        value.as_object_mut().unwrap().remove("ownership_verified");
        let legacy: AccountUsage = serde_json::from_value(value).unwrap();
        assert_eq!(legacy.managed_at_reading, None);
        assert!(!legacy.ownership_verified);
        assert_eq!(legacy.unmanaged_runtimes(2), 1);
        let restored: AccountUsage =
            serde_json::from_value(serde_json::to_value(&a).unwrap()).unwrap();
        assert_eq!(restored.unmanaged_runtimes(0), 1);
    }
}
