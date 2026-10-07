//! The deterministic [`vf_core::ports::IdGen`] (architecture §A6.1, `IdGen`
//! row: "real and local: the system CSPRNG; test: deterministic").
//!
//! # What "deterministic" has to mean here
//!
//! §A5 pins `uuid` with the `v7` feature, so every identifier in the system is
//! a UUID v7: a 48-bit millisecond timestamp followed by 74 bits of
//! randomness, which makes ids sort by creation time in a B-tree. A test
//! double that returned `Uuid::nil()` or a counter in the low bytes would lose
//! that property, and the ordering is load-bearing — §A4 pages runs and
//! findings by id, and `ORDER BY id` is the pagination key.
//!
//! So [`SeqIdGen`] emits real v7 identifiers with the version and variant bits
//! set as the RFC requires, at a fixed timestamp, with a counter where the
//! randomness would be. Two consequences a test can rely on:
//!
//! * the `n`-th id from a fresh generator is always the same value, so it can
//!   be written into a golden file; and
//! * ids from one generator are strictly increasing in byte order, so a test
//!   that asserts on pagination order gets the same order as production.
//!
//! # Why the port implementation is empty
//!
//! As for [`crate::clock`]: `vf_core::ports::IdGen` has no methods until task
//! C1 declares them. The empty `impl` records that this is the test adapter
//! for the §A6.1 row; the inherent API below is what a test drives.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use chrono::{DateTime, Utc};
use uuid::Uuid;

/// Generates reproducible UUID v7 identifiers.
///
/// Cloning shares the counter, so the generator injected into a service and
/// the handle the test kept hand out from one sequence. A test that wants an
/// independent sequence constructs a second generator.
#[derive(Debug, Clone)]
pub struct SeqIdGen {
    /// Milliseconds since the Unix epoch, written into the timestamp field of
    /// every id this generator emits.
    millis: u64,
    /// The next sequence number.
    next: Arc<AtomicU64>,
}

impl Default for SeqIdGen {
    /// A generator at [`crate::clock::epoch`], starting from sequence 0.
    fn default() -> Self {
        Self::at(crate::clock::epoch())
    }
}

impl SeqIdGen {
    /// Bits of the 74 random bits of a v7 that [`SeqIdGen`] uses for the
    /// sequence number.
    ///
    /// The 12 `rand_a` bits are left zero and the sequence occupies `rand_b`,
    /// whose top two bits are the RFC variant. 62 bits is more identifiers
    /// than any test will ask for; past that the sequence wraps and ids repeat,
    /// which is why the bound is documented rather than silently relied upon.
    pub const SEQUENCE_BITS: u32 = 62;

    /// A generator at [`crate::clock::epoch`]. Same as
    /// [`SeqIdGen::default`].
    pub fn new() -> Self {
        Self::default()
    }

    /// A generator whose ids carry `timestamp` in their v7 timestamp field.
    ///
    /// Use it when a test asserts on the relative order of ids from two
    /// generators — give the later one a later timestamp — or when a golden
    /// file has to record a specific creation time.
    ///
    /// A `timestamp` before the Unix epoch is clamped to the epoch: the v7
    /// timestamp field is unsigned and has no way to carry a negative value,
    /// and no identifier in this system is dated before 1970.
    pub fn at(timestamp: DateTime<Utc>) -> Self {
        Self {
            millis: u64::try_from(timestamp.timestamp_millis()).unwrap_or(0),
            next: Arc::new(AtomicU64::new(0)),
        }
    }

    /// The next identifier.
    ///
    /// Infallible by construction: every bit pattern it produces is a valid
    /// UUID v7, and the counter wraps rather than overflowing.
    pub fn next_id(&self) -> Uuid {
        self.id_at(self.next.fetch_add(1, Ordering::Relaxed))
    }

    /// The identifier this generator emits for sequence number `sequence`,
    /// without consuming one.
    ///
    /// This is the function a golden file is written from, and the one a test
    /// uses to say "the id the third insert should have got" without depending
    /// on how many ids the code under test happened to take.
    pub fn id_at(&self, sequence: u64) -> Uuid {
        let mut bytes = [0u8; 16];

        // unix_ts_ms: the low 48 bits of the millisecond timestamp, big-endian.
        bytes[0..6].copy_from_slice(&self.millis.to_be_bytes()[2..8]);

        // ver = 7 in the high nibble of byte 6; rand_a (the low nibble of byte
        // 6 plus byte 7) stays zero so the sequence is contiguous in rand_b.
        bytes[6] = 0x70;

        // var = 0b10 in the top two bits of byte 8, then the 62-bit sequence.
        let seq = sequence.to_be_bytes();
        bytes[8] = 0x80 | (seq[0] & 0x3f);
        bytes[9..16].copy_from_slice(&seq[1..8]);

        Uuid::from_bytes(bytes)
    }

    /// How many identifiers this generator has handed out.
    pub fn issued(&self) -> u64 {
        self.next.load(Ordering::Relaxed)
    }

    /// Restarts the sequence at 0, so a test that reuses one generator across
    /// phases can assert on the same golden ids in each.
    pub fn reset(&self) {
        self.next.store(0, Ordering::Relaxed);
    }
}

/// Test adapter for the §A6.1 `IdGen` row. Empty until task C1 declares the
/// trait's methods; see the module header.
impl vf_core::ports::IdGen for SeqIdGen {}
