//! Starting a conversation — spec 07 §6.2, `design/03` §4.3, `design/05` §3.1.
//!
//! # Why this is not behind `kols-api`
//!
//! Because it creates a network, and that is a workspace act rather than a
//! per-network one — `design/05` §3.1 has the argument in full, and §5.1 states
//! the principle it rests on: the command boundary is per network, and a fact or
//! an act about the workspace above them sits outside it. Accepting a request is
//! the same act from the other side, since it joins one.
//!
//! What *does* cross the boundary is [`kols_api::Event::DirectMessageRequest`],
//! which arrives on the shared network's node and is that network's business.
//!
//! # The ordering that shapes everything here
//!
//! An invite must carry an address, and only a running node knows one (`02`
//! §6.1) — so a conversation's invite cannot be minted until the conversation's
//! own node has run. Nothing here can wait for that: an act that blocked on a
//! peer would be the first one in this client to do it.
//!
//! So starting a conversation **records a want and returns**, exactly as
//! `FetchHistory` does for a page (`design/05` §5.1), and the daemon mints and
//! sends on a later tick once there is an address to put in the invite. The
//! supervisor is already running a node for the new network, because a joined
//! network is warm (`09` §2) — this needs nothing new from it.

use intranet_identity::{NetworkId, PerNetworkIdentityId};

use crate::store::Store;
use crate::workspace::Workspace;

/// What starting a conversation produced.
#[derive(Debug)]
pub struct Started {
    /// The conversation's network id.
    pub conversation: NetworkId,
    /// Whether this member is already able to reach the other one.
    ///
    /// **Reported rather than waited on.** `false` is the ordinary state
    /// immediately after starting, not a failure: the conversation's node has
    /// not run yet, so there is no address for its invite to carry. It becomes
    /// true on a later tick without anybody asking again.
    pub deliverable: bool,
}

/// Starts a conversation with a member of a network the two already share.
///
/// # What it checks, and what it deliberately does not
///
/// It refuses a stranger and it refuses yourself. Both are refusals about the
/// *shared* network, answered by replaying its log — `00` §2's third principle,
/// and the only place the answer could come from, since a conversation network
/// does not exist yet and knows nothing about anybody.
///
/// It does **not** refuse a second conversation with the same person, and does
/// not treat one as a mistake. `03` §4.4 makes a group conversation a network
/// distinct from every pairwise one among the same people, so two networks
/// between two people is a shape this design already has; inventing a rule
/// against it here would be a product decision nothing has asked for. What
/// re-asking *does* replace is an undelivered **offer** to the same person,
/// because a request is a standing ask rather than a message
/// ([`Store::offer_conversation`]).
///
/// # The origin is recorded, and D39 is why
///
/// The conversation stores which network it was arranged in and who with, so
/// that [`Workspace::borrowable_relay`] can recompute — every time, never once —
/// whether this pair may still borrow that network's rendezvous. Without it a
/// conversation between two NATed people has no way to meet at all, and with it
/// stored as a *designation* instead there would be no way to take it back.
pub fn start(
    workspace: &Workspace,
    shared: &Store,
    with: &PerNetworkIdentityId,
    label: Option<&str>,
) -> Result<Started, String> {
    let state = shared.state().map_err(|err| err.to_string())?;
    let mine = shared.identity().map_err(|err| err.to_string())?.id();

    if *with == mine {
        return Err("a conversation is with somebody else".to_owned());
    }
    // **Membership in the shared network, not merely an identity that parses.**
    // The proof this flow sends binds the sender's identity *here* to the
    // conversation's issuer, and the recipient refuses a sender who is not a
    // current member (spec 07 §6.2). So a request to a non-member is one the far
    // end is obliged to throw away, and minting a network for it would leave a
    // store on this disk that nothing will ever use.
    if !state.is_member(with) {
        return Err(
            "that identity is not a member of this network, so there is no way to reach them \
             and nothing to prove to them"
                .to_owned(),
        );
    }

    let fallback = intranet_crypto::to_hex(with.verifying_key().as_bytes());
    let label = label.unwrap_or(&fallback[..8]);
    let conversation = workspace.create_conversation(label)?;

    // Before the offer, so that a crash between the two leaves a conversation
    // nothing will deliver rather than an offer naming a network with no origin
    // — the first is inert and the second would borrow no relay and never say
    // why.
    conversation
        .set_origin(shared.network(), with)
        .map_err(|err| err.to_string())?;

    shared
        .offer_conversation(with, conversation.network())
        .map_err(|err| err.to_string())?;

    Ok(Started {
        conversation: *conversation.network(),
        deliverable: !conversation.addresses().is_empty(),
    })
}

/// A request this node can hand over now.
pub struct Deliverable {
    /// Who it is for.
    pub to: PerNetworkIdentityId,
    /// The conversation it offers, kept so the offer can be forgotten once the
    /// carrier has taken it.
    pub conversation: NetworkId,
    /// The encoded [`kols_core::DmInvite`], ready for Core §5.1's carrier.
    pub payload: Vec<u8>,
}

/// How long a conversation request's invite is good for.
///
/// The same twenty-four hours this client's ordinary invites default to, rather
/// than a number invented for this path — and it starts at *delivery* rather
/// than at the offer, because the invite is minted when there is finally an
/// address to put in it. So an offer that sat for a week waiting for the
/// recipient to come online still arrives fresh.
///
/// **What it costs is that an unanswered request expires**, and the member has
/// to offer again. That is the honest shape: a credential that never expired
/// would be one a recipient could redeem long after the sender stopped meaning
/// it.
const REQUEST_VALID_MILLIS: i64 = 24 * 3_600_000;

/// Builds every offered request this node can deliver at this moment.
///
/// # Why this is a function of the disk rather than a queue
///
/// The daemon calls it on a tick and sends what comes back. An offer with no
/// address yet simply is not in the result, and appears in a later call without
/// anybody re-asking — which is what makes the ordering in this module's header
/// work rather than needing a state machine to sequence it.
///
/// # What is minted here, and what is only read
///
/// The invite and the proof are made fresh each time, because both are cheap and
/// neither is state: an invite is a signed claim with a lifetime, and a proof is
/// a signed statement about two public keys. Nothing is stored by either — spec
/// 07 §6.2 permits the two parties to keep the *request*, and this is how it is
/// built rather than something else to keep.
///
/// # Two rules this deliberately does not share with `Command::CreateInvite`
///
/// That path refuses to mint unless the network designates a relay and the node
/// holds a circuit, because a server's invite goes to a stranger and a LAN-only
/// one is a trap they cannot diagnose. Neither applies here:
///
/// - **A conversation designates no relay and must not** (D39). It borrows one,
///   which is recomputed at every start rather than stored, so there is nothing
///   for a check on `bootstrap_relays` to find.
/// - **The recipient is not a stranger**, and a pair on one LAN reaching each
///   other directly is the case that needs no relay at all. Refusing here would
///   turn D39's stated limit — a conversation between two people who cannot
///   reach each other stops working — into an earlier and less honest failure,
///   which hides the first behind a refusal to even try.
///
/// What it does require is **at least one address**, since an invite carrying
/// none cannot establish the connection it exists for.
pub fn deliverable(
    workspace: &Workspace,
    shared: &Store,
    now_millis: i64,
) -> Result<Vec<Deliverable>, String> {
    let mine = shared.identity().map_err(|err| err.to_string())?;
    let mut ready = Vec::new();

    for (to, conversation) in shared.offered_conversations() {
        // An offer naming a network this disk no longer holds is stale rather
        // than an error — a member may have forgotten the conversation before it
        // was ever delivered, which is a thing they are allowed to do.
        let Some(store) = workspace.store_for(&conversation) else {
            continue;
        };
        let addresses = store.addresses();
        if addresses.is_empty() {
            continue;
        }
        let issuer = store.identity().map_err(|err| err.to_string())?;

        let invite = intranet_invite::Invite::issue(
            &issuer,
            addresses,
            // Bearer, for the reason `CreateInvite` gives: the recipient's
            // identity in the conversation is derived from a network id they are
            // about to learn, so there is nobody to name yet.
            intranet_invite::InviteSubject::Bearer,
            intranet_crypto::Timestamp::from_millis(now_millis),
            intranet_crypto::Timestamp::from_millis(
                now_millis.saturating_add(REQUEST_VALID_MILLIS),
            ),
            1,
        );

        // The one deliberate escape from unlinkability in this whole design
        // (Core §1.2), made by the person it is about, to the person they want
        // to talk to. It needs both private keys, which is why it is built here
        // rather than in `kols-core`.
        let link = intranet_identity::CommonOwnershipProof::create(&mine, &issuer);

        ready.push(Deliverable {
            to,
            conversation,
            payload: kols_core::DmInvite::new(invite, link).encode(),
        });
    }
    Ok(ready)
}

/// What a request turned out to be, once it was checked.
///
/// Both outcomes are **local**. The carrier has already answered at the delivery
/// level by the time a consumer sees a payload (Core §5.1: a request/response
/// cannot wait on a person), so nothing here is sent back to the sender — which
/// is what makes it safe to record *why* something was refused. spec 07 §6.2's
/// rule that a refusal must not distinguish itself is about an answer to the
/// sender, and there is no answer to the sender here.
#[derive(Debug)]
pub enum Arrived {
    /// Checked, kept, and worth showing somebody.
    Request {
        /// Who asked.
        from: PerNetworkIdentityId,
    },
    /// Not shown to anybody, and not kept.
    Refused {
        /// Who it claimed to be from — the carrier bound this to the connection.
        from: PerNetworkIdentityId,
        /// Why, for this node's own log.
        why: &'static str,
    },
}

/// Checks a conversation request and keeps it if it holds up — spec 07 §6.2.
///
/// # The three checks, and which of them no platform could make
///
/// The carrier has already done its own: the payload is signed over sender,
/// namespace, kind and content together, the claimed sender was checked against
/// the connection, and delivery was metered per identity. What is left needs to
/// know what the payload *is*:
///
/// 1. **It decodes.** A payload that does not is not a request.
/// 2. **The identity link binds the right pair** —
///    [`kols_core::DmInvite::verify_from`], which requires the proof to name
///    this sender in *this* network and the invite's issuer in the invite's
///    network. Verifying the signatures is necessary and insufficient: a proof
///    with two genuine signatures over a true statement about somebody else
///    verifies perfectly, and that is the forgery that matters.
/// 3. **The sender is a current member of this network.** Core §5.1 says
///    plainly that the carrier cannot check this — it holds no governance state
///    for the purpose — and §6.2 is the section that owes it. Answered by
///    replaying this store's own log.
///
/// # Nothing is kept before all three pass
///
/// The request is stored only at the end, because the store is what an interface
/// renders from — so keeping an unchecked payload would make this the place an
/// unverified request gets shown from, which is the thing §6.2 forbids by
/// requiring verification *before* the request is shown to anybody.
pub fn receive(shared: &Store, from: &PerNetworkIdentityId, payload: &[u8]) -> Arrived {
    let refused = |why: &'static str| Arrived::Refused { from: *from, why };

    let Ok(request) = kols_core::DmInvite::decode(payload) else {
        return refused("it does not decode as a conversation request");
    };
    if request.verify_from(from, shared.network()).is_err() {
        return refused(
            "its identity link does not bind this sender to the invite's issuer, so it \
             proves nothing about who is asking",
        );
    }
    // Replayed, never remembered — `00` §2's third principle. A sender revoked
    // since the request was composed is not a member now, and now is when the
    // question is being asked.
    let Ok(state) = shared.state() else {
        return refused("this node cannot replay its own log to check who is a member");
    };
    if !state.is_member(from) {
        return refused("its sender is not a current member of this network");
    }

    match shared.record_request(from, payload) {
        Ok(()) => Arrived::Request { from: *from },
        Err(_) => refused("it could not be written down, so nothing will remember it"),
    }
}

/// Accepts a conversation somebody offered — spec 07 §6.2.
///
/// # It joins a network, which is why it is not a command
///
/// The same reason starting one is not (`design/05` §3.1): the `kols-api`
/// boundary is per network, and this makes a second one exist on this disk.
///
/// # What it supplies that the join path could not work out
///
/// **The profile.** A joiner cannot learn it before it syncs — an invite
/// carries only connection bootstrap (Core §5.7) and the profile lives in a log
/// this store has not got — and an absent profile reads as `server`, which is
/// the safe reading for a server and the wrong one here: a node built with
/// discovery on puts a conversation into a routing table, which is the
/// correlation D29 exists to prevent. This flow is the one party that knows
/// what it accepted, so it says. That is E12's obligation on E10, discharged
/// here.
///
/// # The request is verified again rather than trusted
///
/// It was verified before it reached the disk (`receive`), and it is checked
/// again here because the two happen at different times and the answer can
/// change between them: a sender revoked in the meantime is not a member now,
/// and *now* is when a network is about to be joined on their word. `00` §2's
/// third principle — authorization is a computation, never a cached claim —
/// applies to a request this node wrote down as much as to anything a peer says.
///
/// # Why it takes a network id rather than an open store
///
/// A [`Store`] holds the read-side projection's database handle, which is not
/// shareable across threads — so holding one across the `await` below would
/// make this future non-`Send` and the shell could not run it. The stores are
/// therefore opened in scopes either side of the join, which also happens to be
/// the more honest shape: the checks run against the store *now*, and forgetting
/// the request runs against it *after*, and pretending those are one moment is
/// how the re-verification below would have got skipped.
pub async fn accept(
    workspace: &Workspace,
    shared_network: &NetworkId,
    from: &PerNetworkIdentityId,
) -> Result<NetworkId, String> {
    let invite = {
        let shared = workspace
            .store_for(shared_network)
            .ok_or("that network is not on this disk")?;
        let payload = shared
            .request_from(from)
            .ok_or("there is no conversation request from them")?;
        let request = kols_core::DmInvite::decode(&payload)
            .map_err(|_| "that request no longer decodes".to_owned())?;
        request
            .verify_from(from, shared.network())
            .map_err(|_| "that request's identity link no longer holds".to_owned())?;
        let state = shared.state().map_err(|err| err.to_string())?;
        if !state.is_member(from) {
            return Err("they are no longer a member of this network".to_owned());
        }
        request.invite().clone()
    };

    let conversation = invite.network;
    crate::join::redeem(
        workspace.path_for(&conversation),
        invite,
        30,
        false,
        kols_core::NetworkProfile::Conversation,
    )
    .await?;

    // **The origin, so D39 has something to recompute against.** A conversation
    // borrows its rendezvous from the network it was arranged in and never
    // designates one, and the permission to borrow is recomputed every time
    // from *both* parties' membership — which needs to know which network and
    // which peer. Written on this side as well as the sender's, because each
    // end recomputes for itself.
    if let Some(store) = workspace.store_for(&conversation) {
        let _ = store.set_origin(shared_network, from);
    }

    // Only now. A request forgotten before the join completed would leave
    // somebody with no way to accept and nothing on screen explaining why —
    // and the invite it carries is the only copy.
    if let Some(shared) = workspace.store_for(shared_network) {
        shared.forget_request(from).map_err(|err| err.to_string())?;
    }
    Ok(conversation)
}

/// Declines a conversation, which tells nobody — spec 07 §6.2.
///
/// # Nothing is sent, and both sides are owed a sentence about that
///
/// The carrier acknowledged delivery when the payload arrived (Core §5.1), and
/// there is no application-level answer by design: a refusal that
/// distinguished itself would turn every decline into a disclosure the decliner
/// did not choose to make. So the sender sees *delivered* and never learns
/// this, and cannot tell a decline from somebody who has not looked yet.
///
/// `design/09` §1.8 is where the interface owes that in both directions: the
/// person declining is told the sender will not learn it, and the sender's row
/// never says *waiting for an answer*.
///
/// # Blocking is not this
///
/// A blocked sender's request is refused before display and is a client-side
/// list (spec 07 §6.2). This is the ordinary *no* to one request, and it leaves
/// the person able to ask again — which is the honest default, since a decline
/// that silently became a block would be a standing decision nobody took.
pub fn decline(shared: &Store, from: &PerNetworkIdentityId) -> Result<(), String> {
    if shared.request_from(from).is_none() {
        return Err("there is no conversation request from them".to_owned());
    }
    shared.forget_request(from).map_err(|err| err.to_string())
}
