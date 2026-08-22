//! The bounds a *reader* enforces — spec 07 §2.6 and §4.3, `design/01` §10.
//!
//! # Why a reader enforces anything at all
//!
//! The author's own client checks the rate first, and that is the good part of
//! the experience: somebody typing too fast is told so, rather than watching
//! their messages be silently discarded by everybody else. But §4.3 is explicit
//! that these are **validity rules**, not advice:
//!
//! > a record past the ceiling is refused by readers, so a local limit would
//! > mean two members rendering different histories from the same records
//!
//! A writer-side-only limit *is* a local limit — it is whatever the author's
//! own client chose to enforce, which a modified client chooses differently.
//! This module is the other half.
//!
//! # Two rules, and neither works without the other
//!
//! The rate window is computed over the author's **own claimed readings**, never
//! over arrival time, which is what makes every node reach the same verdict
//! (§10.2). That invites an obvious evasion — claim timestamps spaced a minute
//! apart while actually sending them all at once — and `design/01` §10.2 answers
//! it by pointing at the *other* rule rather than by adding a mechanism:
//!
//! > records dated ahead of the receiver's clock are **held until local time
//! > reaches them**. An author who lies about pacing to escape the ceiling gets
//! > exactly the pacing they claimed. No extra mechanism needed.
//!
//! So the skew hold is not a curiosity about clocks; it is the reason the rate
//! ceiling cannot be gamed. Implementing either alone leaves the pair no
//! stronger than the weaker one, which is why they land together.
//!
//! # Held is not refused
//!
//! §2.6 says a future-dated record is **held, not dropped**, and the distinction
//! is load-bearing rather than pedantic. Dropping would make two nodes converge
//! differently depending on when each one happened to look — the exact property
//! the whole merge design exists to protect. A held record stays in the record
//! set, is served like any other, and renders the moment local time reaches its
//! claim. Only *display* waits.
//!
//! That is why [`Withheld`] separates the two: a client can honestly say "this
//! is dated ahead and will appear shortly", and must not say "refused".

use crate::{Hlc, MessageId, Record, RecordClass};
use intranet_identity::PerNetworkIdentityId;
use std::collections::{BTreeMap, BTreeSet};

/// The window every rate ceiling is expressed against — `design/01` §10.1.
///
/// Not a policy value. The ceilings are per *minute* by name
/// (`chat:message-rate-per-minute`), so the window is part of what the key
/// means rather than something a network chooses.
pub const RATE_WINDOW_MILLIS: i64 = 60_000;

/// What a reader needs in order to apply §4.3 and §2.6.
///
/// Gathered into one value rather than passed as five arguments so that adding
/// a bound is a change to one type, and so a caller cannot supply four of the
/// five and believe it is enforcing the set.
///
/// Everything here is **network policy** (§4.3) except `slowmode_seconds`,
/// which is per-channel and delegable (§10.3) — the two arrive from different
/// places and are applied together.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReaderLimits {
    /// `chat:message-rate-per-minute`, bounding the message class.
    pub message_rate_per_minute: i64,
    /// `chat:reaction-rate-per-minute`, bounding the reaction class.
    pub reaction_rate_per_minute: i64,
    /// `chat:message-max-bytes`, bounding one body.
    pub message_max_bytes: usize,
    /// `chat:max-future-skew-millis`, past which a record is held.
    pub max_future_skew_millis: i64,
    /// This channel's slowmode in seconds, from its definition. Zero is off.
    pub slowmode_seconds: u32,
}

impl ReaderLimits {
    /// Reads the network's policy, for a channel with the given slowmode.
    pub fn of(policy: &crate::ChatPolicy<'_>, slowmode_seconds: u32) -> Self {
        Self {
            message_rate_per_minute: policy.message_rate_per_minute(),
            reaction_rate_per_minute: policy.reaction_rate_per_minute(),
            message_max_bytes: policy.message_max_bytes(),
            max_future_skew_millis: policy.max_future_skew_millis(),
            slowmode_seconds,
        }
    }

    /// Limits that refuse nothing.
    ///
    /// For tests about something else, and for reading a record set whose
    /// network policy is genuinely unknown — a backfill walked past the point
    /// where this node can replay governance, say. **Not a default**, and
    /// deliberately not spelled `Default`: a reader that silently enforced
    /// nothing would be the writer-side-only state this module exists to end.
    pub const fn unbounded() -> Self {
        Self {
            message_rate_per_minute: 0,
            reaction_rate_per_minute: 0,
            message_max_bytes: usize::MAX,
            max_future_skew_millis: i64::MAX,
            slowmode_seconds: 0,
        }
    }

    /// The ceiling for a class, or `None` where the class has none.
    ///
    /// Control records are governed by capability instead (§3.3), and a
    /// reserved discriminant never reaches a reader — it is refused at decode.
    /// A ceiling of zero or less is off, matching the writer's reading of it.
    const fn ceiling_for(&self, class: RecordClass) -> Option<i64> {
        let ceiling = match class {
            RecordClass::Message => self.message_rate_per_minute,
            RecordClass::Reaction => self.reaction_rate_per_minute,
            RecordClass::Control | RecordClass::Reserved => return None,
        };
        if ceiling > 0 { Some(ceiling) } else { None }
    }
}

/// Records a reader holds but must not apply, and why.
///
/// Two categories, kept apart because one is permanent and one is not. See the
/// module documentation: telling somebody a message was *refused* when it will
/// render in four minutes is a client asserting something untrue.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Withheld {
    /// Refused, permanently, by a rule about the whole record set.
    pub refused: BTreeMap<MessageId, crate::Rejection>,
    /// Dated further ahead than this node's clock allows, and held until it
    /// catches up (§2.6). These will render on their own.
    pub held: BTreeSet<MessageId>,
}

impl Withheld {
    /// Whether this record must not be applied right now, for any reason.
    pub fn excludes(&self, id: &MessageId) -> bool {
        self.refused.contains_key(id) || self.held.contains(id)
    }

    /// Whether nothing is withheld.
    pub fn is_empty(&self) -> bool {
        self.refused.is_empty() && self.held.is_empty()
    }
}

/// Evaluates §4.3's ceilings and §2.6's hold over a whole record set.
///
/// # Why this is a pass over the set rather than a check in `admit`
///
/// "How many records has this author written in the last minute" has no
/// order-independent answer if it is asked as records land: the same set
/// delivered in two orders would refuse two different records, and two members
/// would render different histories from identical records — which is the one
/// outcome §4.3 exists to prevent. So the rate rule is evaluated the way every
/// other effect in this crate is evaluated, as a function of the sorted set,
/// **after** it is assembled rather than during.
///
/// `records` must therefore be in canonical `(hlc, id)` order, which is what
/// `ChannelView` holds them in.
///
/// # Which records are refused when an author goes over
///
/// The author's records are walked in that same canonical order and admitted
/// greedily: a record is refused when the ceiling is already met by the records
/// **already admitted** in its trailing window. A refused record does not itself
/// occupy a slot, so one burst does not poison everything behind it forever —
/// it costs the author exactly the records that exceeded the rate, and no more.
///
/// Note what this deliberately does not try to be: it is not an attempt to
/// reconstruct what the author's own client decided. That client saw a different
/// set at a different moment. Determinism across *readers* is the property that
/// matters, and every reader computes this identically.
pub fn withheld<'a>(
    records: impl IntoIterator<Item = &'a Record>,
    limits: &ReaderLimits,
    now_millis: i64,
) -> Withheld {
    let mut out = Withheld::default();

    // Admitted readings so far, per (author, class). A `Vec` rather than a
    // counter because the window slides: what matters is how many fall inside
    // it, and the walk is in ascending order so the front expires first.
    let mut seen: BTreeMap<(PerNetworkIdentityId, RecordClass), Vec<Hlc>> = BTreeMap::new();
    // The last message-class reading admitted per author, for slowmode.
    let mut last_message: BTreeMap<PerNetworkIdentityId, Hlc> = BTreeMap::new();

    for record in records {
        let id = record.id();
        let class = record.body.class();
        let at = record.hlc.wall_millis;

        // §2.6. Checked first and independently of everything below: a held
        // record is still a record, still counts toward its author's rate, and
        // is not refused. Only rendering waits.
        //
        // `checked_add` because a record may claim `i64::MAX` and an overflow
        // here would wrap into the past, admitting exactly the record the rule
        // is aimed at.
        if let Some(horizon) = now_millis.checked_add(limits.max_future_skew_millis)
            && at > horizon
        {
            out.held.insert(id);
        }

        let Some(ceiling) = limits.ceiling_for(class) else {
            continue;
        };

        // §10.3. The stricter of the two bounds applies, and slowmode is the
        // stricter one whenever it is set at all — so it is checked first and
        // costs the record its slot either way.
        //
        // *Flagged: applied to the message **class**, so an edit or a withdrawal
        // is paced along with a post.* The specs fix the class of a
        // discriminant (§3.3) and say slowmode is "as deterministic as the
        // network ceiling" (§10.3), but neither says whether it reaches beyond
        // a `Message`. Class is the answer that keeps two client versions
        // agreeing — the whole argument for encoding class in the discriminant
        // range is that an old node counts a new kind correctly without
        // understanding it, and a rule keyed on a variant's *meaning* loses
        // that. The cost is that a long slowmode also delays fixing a typo.
        if class == RecordClass::Message && limits.slowmode_seconds > 0 {
            let interval = i64::from(limits.slowmode_seconds).saturating_mul(1_000);
            if let Some(previous) = last_message.get(&record.author)
                && at.saturating_sub(previous.wall_millis) < interval
            {
                out.refused.insert(id, crate::Rejection::Slowmode);
                continue;
            }
        }

        let window = seen.entry((record.author, class)).or_default();
        let floor = at.saturating_sub(RATE_WINDOW_MILLIS);
        // Ascending order, so everything still inside the window is a suffix.
        let inside = window.iter().filter(|hlc| hlc.wall_millis > floor).count();
        if inside as i64 >= ceiling {
            out.refused.insert(id, crate::Rejection::TooFast);
            continue;
        }

        window.push(record.hlc);
        if class == RecordClass::Message {
            last_message.insert(record.author, record.hlc);
        }
    }

    out
}
