//! The chat capability vocabulary — `design/02` §2.2.
//!
//! # Registration is required
//!
//! `Capability::Extension` carries no tier. A network's policy registry maps a
//! capability *name* to its tier, and an unregistered name is refused outright
//! rather than assumed ordinary — deliberately, so nobody can declare a
//! governance-tier capability as ordinary and slip it onto `everyone`.
//!
//! # One entry per verb, covering every scope of it
//!
//! Chat capabilities are parametrized by scope — `chat:post:<channel>` — and the
//! registry used to match names *exactly*, so each scope needed an entry of its
//! own, added by a `PolicyChange`. Creating a channel with a permission override
//! meant amending network policy, which is a heavyweight action for a routine
//! one, and the registry grew with the channel count forever.
//!
//! Core §2.2.1 now lets a registration ending in `:` cover the namespace beneath
//! it, with the longest match winning. [`namespaces`] is the whole registration a
//! network needs, at genesis, once — every present and future scope of every verb
//! resolves through it, and a network that wants one scope treated differently
//! registers that one name and it overrides the namespace.

use intranet_governance::Tier;
use std::collections::BTreeMap;

/// The verbs, with the tier each carries.
///
/// `manage-channel` and `moderate` are governance-tier because both can widen
/// access to content — `manage-channel` by adding somebody to a private
/// channel's roster. Anything that can do that is governance power however
/// routine it feels, and tiering it correctly is what keeps `everyone` from
/// ever holding it.
pub const VERBS: [(&str, Tier); 7] = [
    ("create-channel", Tier::Ordinary),
    ("post", Tier::Ordinary),
    ("read", Tier::Ordinary),
    ("connect-voice", Tier::Ordinary),
    ("speak-voice", Tier::Ordinary),
    ("manage-channel", Tier::Governance),
    ("moderate", Tier::Governance),
];

/// One registry entry per verb, each covering every scope of that verb.
///
/// These belong in a network's genesis policy. Without them a network cannot
/// grant chat permissions at all, since the grant itself is refused.
///
/// The trailing separator is what makes each entry a namespace rather than an
/// exact name (Core §2.2.1), so `chat:post:` covers the network-wide
/// `chat:post:*`, a category's `chat:post:cat:<id>` and a channel's
/// `chat:post:<id>` alike — including channels that do not exist yet, which is
/// the point. A network wanting one scope on a different tier registers that one
/// name, and being longer it overrides the namespace.
pub fn namespaces() -> BTreeMap<String, Tier> {
    VERBS
        .iter()
        .map(|(verb, tier)| (format!("chat:{verb}:"), *tier))
        .collect()
}
