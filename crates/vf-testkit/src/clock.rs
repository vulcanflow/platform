//! The deterministic [`vf_core::ports::Clock`] (architecture §A6.1, `Clock`
//! row: "real and local: system; test: deterministic").
//!
//! # Why a controllable clock rather than a frozen one
//!
//! Most of the time-dependent behaviour in this system is about an interval
//! elapsing, not about an instant: a challenge token expiring (§A7 invariant
//! 2), an outbox lease being reclaimed, a usage period rolling over, a
//! reservation being held across a retry (§A7 invariant 4). A frozen clock
//! cannot express any of those, and `tokio::time::sleep` in a test trades one
//! flake for another. So the clock moves, but only when a test moves it.
//!
//! # Why the port implementation is empty
//!
//! `vf_core::ports::Clock` has no methods yet — §A6.1 fixes the trait names at
//! task F1 and leaves each port's signatures to the task that owns it, which
//! for `Clock` is C1. The `impl` below is therefore empty on purpose: it
//! records that this type is the test adapter for that row, and gains the
//! method bodies when C1 declares them. The inherent API on this page is the
//! part a test drives and is not C1's to change.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use chrono::{DateTime, TimeDelta, Utc};

use crate::{Error, Result};

/// The instant every [`DeterministicClock`] and [`crate::SeqIdGen`] starts at,
/// as whole seconds since the Unix epoch: `2026-01-01T00:00:00Z`.
///
/// A round, recent, mid-week-free instant, chosen so a failure message is
/// readable and so a snapshot of generated timestamps or UUID v7 prefixes is
/// stable across machines and across the year.
pub const EPOCH_SECS: i64 = 1_767_225_600;

/// A clock that only moves when a test moves it.
///
/// Cloning shares the instant: the clone handed to a service as
/// `Arc<dyn Clock>` and the handle the test kept are the same clock, which is
/// the whole point — a test advances its own handle and the code under test
/// sees it.
#[derive(Debug, Clone)]
pub struct DeterministicClock {
    now: Arc<Mutex<DateTime<Utc>>>,
}

impl Default for DeterministicClock {
    /// A clock at [`epoch`].
    fn default() -> Self {
        Self::at(epoch())
    }
}

/// [`EPOCH_SECS`] as a `DateTime`.
///
/// A function rather than a `const`, because building a `DateTime` is not a
/// const operation in `chrono`.
pub fn epoch() -> DateTime<Utc> {
    // Infallible: EPOCH_SECS * 1e9 is well inside i64, and
    // `from_timestamp_nanos` is total over i64.
    DateTime::from_timestamp_nanos(EPOCH_SECS * 1_000_000_000)
}

impl DeterministicClock {
    /// A clock at [`epoch`]. Same as [`DeterministicClock::default`].
    pub fn new() -> Self {
        Self::default()
    }

    /// A clock at `start`.
    pub fn at(start: DateTime<Utc>) -> Self {
        Self {
            now: Arc::new(Mutex::new(start)),
        }
    }

    /// The current instant.
    pub fn now(&self) -> DateTime<Utc> {
        *guard(&self.now)
    }

    /// Moves the clock by `by` and returns the new instant.
    ///
    /// `by` may be negative, for the tests that have to prove a timestamp from
    /// the future is refused rather than trusted.
    ///
    /// Returns [`Error::Clock`] rather than saturating or panicking when the
    /// result is not representable.
    pub fn advance(&self, by: TimeDelta) -> Result<DateTime<Utc>> {
        let mut now = guard(&self.now);
        let next = now
            .checked_add_signed(by)
            .ok_or_else(|| Error::Clock(format!("{} + {by} is out of range", *now)))?;
        *now = next;
        Ok(next)
    }

    /// Puts the clock at `to` and returns the previous instant.
    ///
    /// For the test that needs a specific wall-clock date — a period boundary,
    /// a daylight-saving transition in a tenant time zone — rather than an
    /// offset from where the clock happens to be.
    pub fn set(&self, to: DateTime<Utc>) -> DateTime<Utc> {
        let mut now = guard(&self.now);
        std::mem::replace(&mut *now, to)
    }
}

/// Test adapter for the §A6.1 `Clock` row. Empty until task C1 declares the
/// trait's methods; see the module header.
impl vf_core::ports::Clock for DeterministicClock {}

/// Locks `m`, recovering from poisoning instead of panicking.
///
/// A poisoned mutex here means some other test thread panicked while holding
/// the instant. The instant itself cannot be left inconsistent — it is one
/// `Copy` value — so the useful thing to do is carry on and let the panicking
/// test be the failure the developer reads, rather than turning it into a
/// second, less informative panic in every thread that touches the clock.
fn guard<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}
