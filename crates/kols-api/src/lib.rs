//! The command/event boundary between ko-ls's interface and its core.
//!
//! # Why there is a boundary at all
//!
//! The interface is HTML/CSS/JS in a webview and holds no keys, no sockets and
//! no files (`design/05` §1). Everything it can do is one of a fixed set of
//! commands, and everything it learns arrives as an event. That is not a style
//! preference: the eventual goal is running the same interface inside the
//! app-bundle sandbox against a *narrower* API (`design/05` §7), and a narrow
//! API is only narrow if it was narrow from the start.
//!
//! # The three properties this crate exists to hold
//!
//! `design/05` §3 names them, and says retrofitting any of the three is
//! expensive. Two are here now and the third has nothing to hold yet:
//!
//! 1. **No ambient authority.** Every command names its target, and the core
//!    re-checks permission on receipt rather than trusting that the interface
//!    only offered buttons the user was allowed to press. [`authorize`] is that
//!    check, and [`Authorized`] is what makes skipping it inexpressible rather
//!    than merely discouraged — there is no other way to construct one.
//! 2. **Consent is a decorator, not a redesign.** Every command carries a
//!    [`Sensitivity`], so the sandboxed build can wrap the ones that sign
//!    something in a platform prompt (App Hosting §3.3) while the native build
//!    prompts for nothing. Nothing else differs between the two builds.
//! 3. **Events are idempotent and re-deliverable.** Not yet expressed here, and
//!    deliberately: the sync engine that would emit them (`design/05` §4) is
//!    `kols serve`, whose records do not yet cross this boundary. An `Event`
//!    enum written before anything emits one would be a contract with no
//!    implementation to keep it honest, which is how the two drift apart. It
//!    lands when the engine does.
//!
//! # What this crate refuses to become
//!
//! It holds no state, performs no I/O and reaches no store. It answers whether a
//! command may proceed, from replayed governance state and network policy, and
//! hands back a value an executor can act on. Anything that needs the store to
//! answer is named in [`authorize`]'s own documentation, along with where it is
//! actually enforced — because a check that looks complete and is not is worse
//! than one that says what it does not cover.

#![deny(missing_docs)]

mod authorize;
mod command;
mod outcome;

pub use authorize::{
    Actor, Authorized, Channels, PlacementMap, Refusal, authorize, placement,
};
pub use command::{Command, Sensitivity};
pub use outcome::Outcome;
