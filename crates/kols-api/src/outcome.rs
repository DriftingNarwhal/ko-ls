//! What a command produced.

use intranet_identity::PerNetworkIdentityId;
use kols_core::{CategoryId, ChannelId, MessageId, Privacy, Rejection, RenderedMessage};

/// The result of running a command.
///
/// Typed rather than printed, which is the whole point of having an executor at
/// all: the thing that performs a command must not also decide how it looks, or
/// the interface cannot reuse it. `kols` renders these to a terminal; a webview
/// would render the same values differently, and neither is the executor's
/// business.
///
/// Every variant is produced by code that exists. There is no `NotImplemented`,
/// because a command the executor cannot run is refused before it reaches one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// A channel was rendered.
    Opened {
        /// Which channel.
        channel: ChannelId,
        /// Its messages, merged and ordered.
        messages: Vec<RenderedMessage>,
        /// Records the reader refused, and why.
        ///
        /// Carried rather than dropped: a record this node refuses is one some
        /// other client may be showing, and silence would make the two look like
        /// they agree.
        rejected: Vec<(MessageId, Rejection)>,
        /// How many distinct authors contributed to what was rendered.
        authors: usize,
    },
    /// A signed record was appended to the actor's own log.
    ///
    /// One variant for messages, edits, withdrawals, reactions and pins alike,
    /// because at this layer they are the same event: a record this member
    /// signed, in the one log they can write.
    Wrote {
        /// The record's id.
        record: MessageId,
        /// Bytes the append actually moved.
        ///
        /// The number `design/01` §3.1 exists to keep small, surfaced because a
        /// client that cannot see it cannot notice it regressing.
        moved: usize,
        /// The size of the object it moved them within.
        total: usize,
    },
    /// A channel was defined.
    ChannelCreated {
        /// Its derived id.
        channel: ChannelId,
        /// Its name.
        name: String,
        /// Whether it is restricted to a roster.
        privacy: Privacy,
    },
    /// A channel's definition changed.
    ChannelUpdated {
        /// Which channel.
        channel: ChannelId,
    },
    /// A category was named and positioned.
    CategoryCreated {
        /// Its derived id.
        category: CategoryId,
        /// What it is called.
        name: String,
    },
    /// A category's definition changed.
    CategoryUpdated {
        /// Which category.
        category: CategoryId,
    },
    /// A display name was claimed.
    NameClaimed {
        /// The name, as it will be displayed.
        name: String,
    },
    /// An invite was minted.
    InviteCreated {
        /// Its canonical bytes, which are the protocol's and are what travels.
        ///
        /// Not a URI: how an invite is *carried* — a link, a QR code, a pasted
        /// string — is presentation, and the shell that renders it decides.
        invite: Vec<u8>,
        /// When it stops being valid, in milliseconds since the epoch.
        expires_at_millis: i64,
        /// How many identities may be admitted with it.
        uses: u32,
    },
    /// The network's designated relays changed.
    BootstrapRelaysSet {
        /// The set now in force.
        relays: Vec<String>,
    },
    /// The network's membership changed.
    MembershipChanged {
        /// Whose.
        identity: PerNetworkIdentityId,
        /// Added if true, removed if false.
        admitted: bool,
        /// Which group — `everyone` for admission to the network, a role otherwise.
        group: intranet_governance::GroupId,
    },
    /// The network's name changed — D32, spec 07 §1.7.
    NetworkNamed {
        /// What it is now. Empty means the network is unnamed, which is a real
        /// state rather than a missing one.
        name: String,
    },
    /// One of the network's chat settings changed.
    ChatSettingSet {
        /// The policy key, as the log carries it.
        key: &'static str,
        /// Its new value.
        value: i64,
    },
    /// The network's admission mode changed.
    AdmissionModeSet {
        /// What it is now.
        mode: intranet_governance::AdmissionMode,
    },
    /// A role was created, holding nothing.
    RoleCreated {
        /// Its name.
        group: intranet_governance::GroupId,
    },
    /// A role gained or lost one capability.
    PermissionSet {
        /// Which role.
        group: intranet_governance::GroupId,
        /// The capability's full name, as the log carries it.
        ///
        /// The resolved name rather than the verb and scope it was built from,
        /// because that is what a reader will be looking for when a grant does
        /// not resolve — and building it a second time to display it is how the
        /// two spellings drift apart.
        capability: String,
        /// Granted if true, withdrawn if false.
        granted: bool,
    },
}
