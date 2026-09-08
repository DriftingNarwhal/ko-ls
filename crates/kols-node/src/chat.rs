//! The next reading an author may write, and the one clock read in the program.
//!
//! **This used to build author logs**, and `rebuild_log` was what made a send
//! linear in everything the member had ever written — it replayed every record
//! into a segment it never sealed, so the executor encoded the whole history as
//! one object to answer a question about the next reading. The executor stops
//! there now (`design/05` §5) and the daemon does its own replay, so the
//! rebuilding this module was named for has no callers and is gone rather than
//! kept for a future one.

use kols_core::Hlc;

/// Wall-clock now, in milliseconds.
///
/// The one place this program reads a clock. Everything downstream takes a
/// timestamp as an argument, deliberately, so ordering stays a function of
/// explicit inputs rather than of when code happened to run.
pub fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// The next reading for this author, strictly greater than their last.
///
/// Per (author, device) rather than per author — spec 07 §2.6, learned in P0
/// when a merged segment interleaving two devices declared every concurrent
/// recovery invalid.
///
/// **Takes the last reading rather than a log**, which is a fix as well as a
/// saving. Read off the open segment, this returned `None`'s branch on a segment
/// that had just been sealed — so the reading before the seal was invisible, and
/// nothing would have caught the result going backwards across the boundary:
/// `AuthorLog::push` checks monotonicity only within the segment it is pushing
/// to. The index that supplies this keeps the newest reading for the life of the
/// channel, so a seal is not a place where history begins again.
pub const fn next_hlc(last: Option<Hlc>, wall: i64) -> Hlc {
    match last {
        Some(last) if wall <= last.wall_millis => Hlc::new(last.wall_millis, last.counter + 1),
        _ => Hlc::new(wall, 0),
    }
}
