//! The read-side projection — `design/05` §5.
//!
//! # Why this exists
//!
//! Rendering a channel read **every record in it**, decoded each one, merged the
//! whole set and ran the reader-side rate pass over all of it — for a page of
//! fifty. Measured at 222 ms with six thousand records and roughly four seconds
//! at a hundred thousand, growing without limit, because a conversation only
//! gets longer.
//!
//! A page is a query and the record files cannot answer it: they are named by
//! content id, and a content id says nothing about order or about what a record
//! acts on. This is the index that can.
//!
//! # What it is not
//!
//! **Never a source of truth.** The records are the truth and this is derived
//! from them; it is deletable, rebuildable, and consulted for nothing that
//! replay answers. Where it and the files disagree the files win and this is
//! rebuilt — the standing rule for every cache in this client.
//!
//! It also holds **no record bytes**. A page reads fifty record files rather
//! than a hundred thousand, which is the whole saving; copying the bytes in here
//! as well would put a third copy of every message on every member's disk to
//! avoid a decode that costs nothing at fifty.

#![deny(missing_docs)]

mod schema;
mod verdict;

pub use schema::{FoldedUnder, Projection, ProjectionError};
pub use verdict::{Verdict, decide};
