//! Creating a network, and reading channel structure back out of its log.

use crate::store::{Store, StoreError};
use intranet_crypto::Timestamp;
use intranet_governance::{
    Capability, ContentType, EntryBody, GovernanceState, LogEntry, NetworkPolicy,
};
use intranet_identity::{NetworkId, PerNetworkIdentity};
use kols_core::{
    CHAT_LOG_CONTENT_TYPE, CategoryId, ChannelEntry, ChannelEntryBody, ChannelId, ChannelKind,
    ChatPolicy, Privacy,
};
use std::collections::BTreeMap;

/// The genesis entry for a new `server`-profile network.
///
/// Three things have to be right here or the network is quietly unusable, and
/// each was learned by getting it wrong:
///
/// 1. **`chat-log` must be on the content-type allowlist.** Publishing a pointer
///    is gated on both an allowlisted type and a `publish:<type>` capability
///    (Core §2.8's two gates); missing either one means every post is refused by
///    the author's own node.
/// 2. **The chat capability vocabulary must be registered.** An unregistered
///    extension name is refused outright rather than assumed ordinary, so a
///    network that skips this cannot grant a chat permission at all.
///
/// The profile is deliberately *not* written: absent means `server` (spec 07
/// §1.2), and writing today's default into a network freezes it there. A network
/// name is likewise not a policy value — spec 07 defines no key for one, and
/// inventing vocabulary the normative document does not have is how two clients
/// end up disagreeing about what a network is called. The CLI keeps it locally.
pub fn genesis(
    founder: &PerNetworkIdentity,
    network: NetworkId,
    relays: Vec<String>,
) -> LogEntry {
    let mut policy = NetworkPolicy::conservative_default();
    // Core §5.5: a network with no designated relay is reachable only by members
    // who can already dial each other, which two people behind NAT cannot. The
    // set is chosen at creation and replaced later by `define-policy`.
    policy.bootstrap_relays = relays;
    policy
        .content_type_allowlist
        .insert(ContentType::new(CHAT_LOG_CONTENT_TYPE));
    policy
        .extension_capabilities
        .extend(kols_core::capabilities::namespaces());
    // `design/01` §8's "open archive" preset. A joiner receives the historical
    // epoch keys as well as the current one, so scrollback that predates them is
    // readable — which is what people expect of a chat server and is a genuine
    // choice rather than a default: the conservative reading (Core §3.4) gives a
    // joiner the current epoch forward and nothing before it.
    policy.history_access = intranet_governance::HistoryAccess::FullHistory;
    LogEntry::create(
        founder,
        None,
        Timestamp::from_millis(0),
        EntryBody::Genesis {
            network,
            policy,
            // What every member gets on arrival. Reading and posting are
            // ordinary; creating channels is not granted here, so a founder
            // hands it out deliberately rather than it being ambient.
            everyone_capabilities: [
                Capability::ReadContent,
                Capability::publish(CHAT_LOG_CONTENT_TYPE),
                Capability::extension("chat:post:*"),
                Capability::extension("chat:read:*"),
                // Claiming your own name is ordinary and belongs to everyone:
                // the payload carries no identity, so however broadly this is
                // granted it can only ever bind the claimant (spec 07 §3.9).
                Capability::extension("chat:set-name:*"),
            ]
            .into_iter()
            .collect(),
        },
    )
}

/// A channel as replay currently understands it.
#[derive(Debug, Clone)]
pub struct Channel {
    /// Its derived id.
    pub id: ChannelId,
    /// Its current name.
    pub name: String,
    /// The category it sits in, which is where permissions bind.
    pub category: Option<CategoryId>,
    /// What it is for. Text is the only kind this build drives; voice and
    /// stage channels are P3 work and would be listed but not enterable.
    #[allow(dead_code)]
    pub kind: ChannelKind,
    /// Whether the network at large may read it.
    pub privacy: Privacy,
    /// Its current topic.
    pub topic: String,
    /// Seconds between one author's messages.
    pub slowmode: u32,
    /// Whether it has been archived.
    pub archived: bool,
    /// Its position among its siblings — spec 07 §1.6. `None` means it has never
    /// been given one, which sorts it after every sibling that has rather than
    /// at zero, so a new channel lands at the end instead of the top.
    pub position: Option<u32>,
}

/// Replays the chat namespace into current channel state.
///
/// This is the consuming half of Core §2.7.2: the protocol carries application
/// entries without knowing what they mean, and each consuming spec replays the
/// log filtering for its own namespace. Anything this reader refuses is skipped
/// rather than fatal — one malformed entry from one member must not make the
/// whole network unreadable — but every refusal is returned so a caller can say
/// so rather than silently showing less than exists.
pub fn channels(
    store: &Store,
    state: &GovernanceState,
) -> Result<(BTreeMap<ChannelId, Channel>, Vec<String>), StoreError> {
    let log = store.log()?;
    let profile = ChatPolicy::of(&state.policy).profile();
    let mut channels: BTreeMap<ChannelId, Channel> = BTreeMap::new();
    let mut refused = Vec::new();

    for hash in log.canonical_chain() {
        let Some(entry) = log.get(&hash) else { continue };
        let EntryBody::AppEntry { namespace, .. } = &entry.body else {
            continue;
        };
        if namespace != kols_core::CHAT_NAMESPACE {
            continue;
        }

        // The category a channel is already known to sit in, which is what lets
        // a category-scoped grant authorize a later entry against it.
        let known = channel_of(&entry.body).and_then(|id| channels.get(&id)).and_then(|c| c.category);
        let read = ChannelEntry::read(&entry.body, profile, known.as_ref());
        let channel_entry = match read {
            Ok(entry) => entry,
            Err(refusal) => {
                refused.push(refusal.to_string());
                continue;
            }
        };

        // Every channel kind decodes to a channel subject, because the
        // discriminant chooses both. Guarded rather than unwrapped so that a
        // future kind cannot turn a wrong assumption into a panic.
        let subject = channel_entry.channel();

        match channel_entry.body {
            ChannelEntryBody::Definition {
                name,
                category,
                kind,
                privacy,
                topic,
                slowmode,
            } => {
                let Some(id) = subject else {
                    refused.push("channel definition naming a category".to_owned());
                    continue;
                };
                channels.insert(
                    id,
                    Channel {
                        id,
                        name,
                        category,
                        kind,
                        privacy,
                        topic,
                        slowmode,
                        archived: false,
                        position: None,
                    },
                );
            }
            ChannelEntryBody::Update { change } => {
                let Some(channel) = subject.and_then(|id| channels.get_mut(&id)) else {
                    refused.push("update for a channel never defined".to_owned());
                    continue;
                };
                match change {
                    kols_core::ChannelChange::Rename(name) => channel.name = name,
                    kols_core::ChannelChange::Recategorise(category) => channel.category = category,
                    kols_core::ChannelChange::SetTopic(topic) => channel.topic = topic,
                    kols_core::ChannelChange::SetSlowmode(seconds) => channel.slowmode = seconds,
                    kols_core::ChannelChange::SetPosition(position) => {
                        channel.position = Some(position);
                    }
                    kols_core::ChannelChange::Archive => channel.archived = true,
                    kols_core::ChannelChange::Delete => {
                        if let Some(id) = subject {
                            channels.remove(&id);
                        }
                    }
                }
            }
            // Roster and rotation state belong to the keying layer, which is P2
            // work. Skipped rather than mis-modelled: pretending to track a
            // roster that nothing enforces would be worse than not showing one.
            //
            // Category entries are replayed by `categories` instead. They are a
            // different subject with a different id space, and folding them in
            // here would mean a map keyed by something that is not a channel.
            ChannelEntryBody::Membership { .. }
            | ChannelEntryBody::Rotation { .. }
            | ChannelEntryBody::CategoryDefinition { .. }
            | ChannelEntryBody::CategoryUpdate { .. } => {}
        }
    }

    Ok((channels, refused))
}

/// A category as replay currently understands it — spec 07 §1.8.
///
/// Name and position, because that is all a category definition carries. It is
/// **not** where permissions live: those bind against the id a channel itself
/// names, so a category with no definition still scopes capabilities exactly as
/// one with a definition does.
#[derive(Debug, Clone)]
pub struct Category {
    /// Its id.
    pub id: CategoryId,
    /// What to call it.
    pub name: String,
    /// Where it sits among the other categories.
    pub position: Option<u32>,
}

/// Replays the chat namespace into current category state — spec 07 §1.8.
///
/// A second pass over the same log rather than a second return value from
/// [`channels`], so that nine existing callers keep the shape they have. Both
/// walks are the cost `design/05` §5's projection exists to remove, and neither
/// is worth optimising before it is measured.
pub fn categories(
    store: &Store,
    state: &GovernanceState,
) -> Result<(BTreeMap<CategoryId, Category>, Vec<String>), StoreError> {
    let log = store.log()?;
    let profile = ChatPolicy::of(&state.policy).profile();
    let mut categories: BTreeMap<CategoryId, Category> = BTreeMap::new();
    let mut refused = Vec::new();

    for hash in log.canonical_chain() {
        let Some(entry) = log.get(&hash) else { continue };
        let EntryBody::AppEntry { namespace, .. } = &entry.body else {
            continue;
        };
        if namespace != kols_core::CHAT_NAMESPACE {
            continue;
        }
        // No known category to supply: a category entry is scoped `*` or to its
        // own id, never to an enclosing one, because nothing encloses a category.
        let category_entry = match ChannelEntry::read(&entry.body, profile, None) {
            Ok(entry) => entry,
            Err(refusal) => {
                refused.push(refusal.to_string());
                continue;
            }
        };
        let Some(id) = category_entry.category() else {
            continue;
        };
        match category_entry.body {
            ChannelEntryBody::CategoryDefinition { name, position } => {
                categories.insert(
                    id,
                    Category {
                        id,
                        name,
                        position: Some(position),
                    },
                );
            }
            ChannelEntryBody::CategoryUpdate { change } => {
                let Some(category) = categories.get_mut(&id) else {
                    refused.push("update for a category never defined".to_owned());
                    continue;
                };
                match change {
                    kols_core::CategoryChange::Rename(name) => category.name = name,
                    kols_core::CategoryChange::SetPosition(position) => {
                        category.position = Some(position);
                    }
                    // Removes a name and a sort key, never a scope: channels
                    // naming this category stay in it and resolve exactly what
                    // they did before (spec 07 §1.8).
                    kols_core::CategoryChange::Delete => {
                        categories.remove(&id);
                    }
                }
            }
            _ => {}
        }
    }

    Ok((categories, refused))
}

fn channel_of(body: &EntryBody) -> Option<ChannelId> {
    let EntryBody::AppEntry { payload, .. } = body else {
        return None;
    };
    ChannelEntry::decode_payload(payload)
        .ok()
        .and_then(|e| e.channel())
}

/// Finds a channel by name, or by the leading hex of its id.
pub fn resolve<'a>(
    channels: &'a BTreeMap<ChannelId, Channel>,
    needle: &str,
) -> Option<&'a Channel> {
    let needle = needle.strip_prefix('#').unwrap_or(needle);
    channels
        .values()
        .find(|c| c.name == needle)
        .or_else(|| {
            channels
                .values()
                .find(|c| intranet_crypto::to_hex(c.id.as_bytes()).starts_with(needle))
        })
}
