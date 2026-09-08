//! This member's own readings in a channel, kept so a send need not replay.
//!
//! # Why this exists
//!
//! An author log is a pure function of its record sequence (`design/01` §3.1),
//! and the executor was re-executing that function on every send: three full
//! scans of a channel's record directory — every record, every author, read from
//! its own file, decoded and sorted — to answer three small questions. That is
//! linear in everything the member has ever written, which is a date rather than
//! a constant (`design/05` §5).
//!
//! The three questions are all this side of the boundary ever asks:
//!
//! 1. the newest reading, for the next one;
//! 2. the newest `Message`-class reading, for slowmode;
//! 3. how many of a class were written in the trailing minute, for the ceiling.
//!
//! # It is a cache, and the records stay the source of truth
//!
//! Absent, truncated or unreadable, this is recomputed from the records exactly
//! as before and written back. Nothing depends on it existing and nothing here
//! is a second answer to a question replay answers differently — which is the
//! condition `design/05` §5 sets for every checkpoint in this client, and the
//! same one Core §2.7 sets for checkpointed replay of the governance log.
//!
//! # What is exact and what is pruned
//!
//! The two "newest" values are exact and kept for the life of the channel: they
//! are single readings, and slowmode may reach back six hours (spec 07 §4.3's
//! `chat:slowmode-max-seconds`), which no window worth keeping would cover.
//!
//! The recent list is pruned to [`WINDOW_MILLIS`], which is comfortably wider
//! than the one minute the rate ceiling asks about, so the answer inside that
//! minute stays exact while the file stays small.

use kols_core::{Hlc, Record, RecordClass};

/// How much of the recent past the pruned list keeps.
///
/// Two minutes against the rate ceiling's one, so that a reading which is inside
/// the minute being asked about is never outside the list holding it. The margin
/// is for clocks rather than for luck: a reading carries its author's wall time,
/// and the window is computed against the reading being written.
pub const WINDOW_MILLIS: i64 = 2 * 60 * 1000;

/// The most readings the list holds, however little time they span.
///
/// Time alone does not bound this. A network may set no ceiling at all — spec 07
/// §4.3's rate values are policy, and zero means *no limit* — and then nothing
/// refuses a member writing as fast as a machine can, so a window measured only
/// in minutes grows with how fast they type.
///
/// **The count it answers stays exact wherever a ceiling exists**, which is the
/// only case it is consulted in: the writer refuses past the ceiling, so the
/// readings inside any one minute never exceed it, and a cap far above any
/// plausible ceiling can never cut into the minute being counted. `01` §10.1
/// calls 30 a minute deliberately generous; this is seventeen a second. A
/// network setting a ceiling beyond it would get a writer-side check that
/// undercounts, and readers would still enforce the real one (`01` §10.2) —
/// worth stating rather than leaving as an assumption about configuration.
///
/// Kept small on purpose as well as bounded: this file is rewritten on every
/// record, so its size is a cost per send rather than a cost at rest.
pub const MAX_RECENT: usize = 512;

/// What this member has written in one channel, as far as a writer needs to know.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct OwnReadings {
    /// The newest reading this member wrote here. Exact.
    pub last: Option<Hlc>,
    /// The newest `Message`-class reading. Exact, and what slowmode asks about.
    pub last_message: Option<Hlc>,
    /// Readings inside [`WINDOW_MILLIS`] of the newest, with their class.
    pub recent: Vec<(Hlc, RecordClass)>,
}

impl OwnReadings {
    /// Folds a member's records into the three answers.
    ///
    /// The rebuild path, run when the cache is missing rather than per send.
    pub fn of(records: &[Record]) -> Self {
        let mut folded = Self::default();
        for record in records {
            folded.note(record.hlc, record.body.class());
        }
        folded
    }

    /// Records one reading, keeping the invariants above.
    pub fn note(&mut self, hlc: Hlc, class: RecordClass) {
        if self.last.is_none_or(|last| hlc > last) {
            self.last = Some(hlc);
        }
        if class == RecordClass::Message && self.last_message.is_none_or(|last| hlc > last) {
            self.last_message = Some(hlc);
        }
        self.recent.push((hlc, class));
        self.prune();
    }

    /// How many readings of `class` fall inside `window` before `hlc`.
    ///
    /// Counts strictly after the boundary, which is what the ceiling has always
    /// asked: a record exactly at the edge of the window is outside it.
    pub fn count_within(&self, class: RecordClass, hlc: Hlc, window: i64) -> usize {
        let since = hlc.wall_millis.saturating_sub(window);
        self.recent
            .iter()
            .filter(|(reading, kind)| *kind == class && reading.wall_millis > since)
            .count()
    }

    /// Drops readings the recent list no longer needs.
    fn prune(&mut self) {
        let Some(newest) = self.recent.iter().map(|(hlc, _)| hlc.wall_millis).max() else {
            return;
        };
        let floor = newest.saturating_sub(WINDOW_MILLIS);
        self.recent.retain(|(hlc, _)| hlc.wall_millis >= floor);
        // Newest kept, since the question is always about the recent past.
        if self.recent.len() > MAX_RECENT {
            self.recent.drain(..self.recent.len() - MAX_RECENT);
        }
    }

    /// The stored form: one reading per line, oldest first.
    ///
    /// Plain text rather than the canonical encoding, deliberately. This is
    /// local state that never reaches a wire, so it owes no determinism to
    /// anybody — and `kols-core`'s encoders are for the bytes that do, where a
    /// second serialization beside the first is the drift this project keeps
    /// warning about.
    pub fn encode(&self) -> String {
        let mut out = String::new();
        if let Some(last) = self.last {
            out.push_str(&format!("last {} {}\n", last.wall_millis, last.counter));
        }
        if let Some(last) = self.last_message {
            out.push_str(&format!("message {} {}\n", last.wall_millis, last.counter));
        }
        for (hlc, class) in &self.recent {
            out.push_str(&format!(
                "recent {} {} {}\n",
                hlc.wall_millis,
                hlc.counter,
                class_name(*class)
            ));
        }
        out
    }

    /// Reads the stored form, or `None` if any line does not parse.
    ///
    /// All-or-nothing on purpose: a half-read index would answer the rate
    /// question with a number that is too small, which permits rather than
    /// refuses. Rebuilding from the records is always available and always
    /// right, so there is nothing to salvage a damaged file for.
    pub fn decode(text: &str) -> Option<Self> {
        let mut folded = Self::default();
        for line in text.lines().filter(|line| !line.trim().is_empty()) {
            let mut parts = line.split_whitespace();
            match parts.next()? {
                "last" => folded.last = Some(hlc(&mut parts)?),
                "message" => folded.last_message = Some(hlc(&mut parts)?),
                "recent" => {
                    let reading = hlc(&mut parts)?;
                    folded.recent.push((reading, class_from(parts.next()?)?));
                }
                _ => return None,
            }
        }
        Some(folded)
    }
}

fn hlc<'a>(parts: &mut impl Iterator<Item = &'a str>) -> Option<Hlc> {
    let wall = parts.next()?.parse().ok()?;
    let counter = parts.next()?.parse().ok()?;
    Some(Hlc::new(wall, counter))
}

const fn class_name(class: RecordClass) -> &'static str {
    match class {
        RecordClass::Message => "message",
        RecordClass::Reaction => "reaction",
        RecordClass::Control => "control",
        RecordClass::Reserved => "reserved",
    }
}

fn class_from(name: &str) -> Option<RecordClass> {
    match name {
        "message" => Some(RecordClass::Message),
        "reaction" => Some(RecordClass::Reaction),
        "control" => Some(RecordClass::Control),
        "reserved" => Some(RecordClass::Reserved),
        _ => None,
    }
}
