//! The chat capability vocabulary — `design/02` §2.2.
//!
//! # Registration is required, and exact
//!
//! `Capability::Extension` carries no tier. A network's policy registry maps a
//! capability *name* to its tier, and an unregistered name is refused outright
//! rather than assumed ordinary — deliberately, so nobody can declare a
//! governance-tier capability as ordinary and slip it onto `everyone`.
//!
//! The registry matches names exactly, which is the constraint this module has
//! to work within: a parametrized capability like `chat:post:<channel>` needs an
//! entry per channel. [`network_scoped`] registers the wildcard verbs every
//! network needs; [`for_category`] and [`for_channel`] register a scope's names
//! at the point that scope is created. See `design/06` §11 for why this is
//! worth changing in the protocol rather than living with.

use crate::{CategoryId, ChannelId};
use intranet_crypto::to_hex;
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

/// Registry entries for the network-wide form of every verb (`chat:<verb>:*`).
///
/// These belong in a network's genesis policy. Without them a network cannot
/// grant chat permissions at all, since the grant itself is refused.
pub fn network_scoped() -> BTreeMap<String, Tier> {
    VERBS
        .iter()
        .map(|(verb, tier)| (format!("chat:{verb}:*"), *tier))
        .collect()
}

/// Registry entries for one category's names.
pub fn for_category(category: &CategoryId) -> BTreeMap<String, Tier> {
    let scope = to_hex(category.as_bytes());
    VERBS
        .iter()
        .map(|(verb, tier)| (format!("chat:{verb}:cat:{scope}"), *tier))
        .collect()
}

/// Registry entries for one channel's names, needed only where a channel
/// overrides its category.
pub fn for_channel(channel: &ChannelId) -> BTreeMap<String, Tier> {
    let scope = to_hex(channel.as_bytes());
    VERBS
        .iter()
        .map(|(verb, tier)| (format!("chat:{verb}:{scope}"), *tier))
        .collect()
}
