//! The direct-message invitation payload — spec 07 §6.2 (E10).
//!
//! A direct message conversation is its own network (§1.5), which leaves exactly
//! one thing to solve: getting its invite to somebody reachable only inside a
//! network the two already share. This is the payload that does it.
//!
//! # What travels, and why each half is needed
//!
//! An **invite** to the conversation network (Core §5.6), and a
//! **common-ownership proof** (Core §1.2) binding the sender's identity *in the
//! shared network* to the identity that issued that invite.
//!
//! Without the proof the invite is unattributable. Identities are per-network and
//! deliberately unlinkable, so an invite issued by some identity in some new
//! network says nothing about who is asking — the recipient would be deciding
//! whether to talk to a stranger who claims to be Alice. The proof is the
//! mechanism Core §1.2 provides for precisely this, and this is its first real
//! use: the one deliberate escape from unlinkability, made by the person it is
//! about, to the person they want to talk to.
//!
//! # Where this rides
//!
//! Core §5.1's generic carrier, `/intranet/direct/1.0.0`, as namespace `chat` and
//! kind `dm-invite`. It was specified as a protocol of its own and landed
//! generically for the reason E2 and E9 did — an application's name in the
//! platform's transport stack is what Core §0 rules out. Nothing about this
//! payload changed with that.
//!
//! # What the carrier already did, so this does not
//!
//! The carrier signed the payload over sender, namespace, kind and content
//! together; checked the claimed sender against the connection it arrived on,
//! which a signature cannot do for itself because a signature travels; and
//! metered delivery per sending identity. So nothing here re-checks authorship of
//! the *envelope*.
//!
//! What it could not do is anything requiring knowledge of what this is. Two
//! checks are therefore this module's, and [`DmInvite::verify_from`] is where
//! they live — see its documentation for why the second one is the one that
//! matters.

use intranet_identity::{CommonOwnershipProof, NetworkId, PerNetworkIdentityId};
use intranet_invite::Invite;

use crate::CoreError;

/// The namespace this payload travels under on Core §5.1's carrier.
pub const DM_NAMESPACE: &str = "chat";

/// The kind this payload travels under on Core §5.1's carrier.
pub const DM_KIND: &str = "dm-invite";

/// Domain tag for the payload — spec 07 §3.2.
const DM_INVITE_DOMAIN: &str = "intranet.wire.chat-dm-invite.v1";

/// A request to start a conversation — spec 07 §6.2.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DmInvite {
    /// The conversation network's invite.
    invite: Invite,
    /// Proof that the sender here is the identity that issued that invite.
    link: CommonOwnershipProof,
}

impl DmInvite {
    /// Builds a request.
    ///
    /// The caller supplies a proof rather than the two identities, because
    /// creating one needs both private keys and this type has no business
    /// holding either — the same reason nothing in `kols-core` touches key
    /// material.
    pub fn new(invite: Invite, link: CommonOwnershipProof) -> Self {
        Self { invite, link }
    }

    /// The conversation network's invite.
    ///
    /// Only reachable through [`verify_from`](Self::verify_from) in practice: a
    /// caller that acted on this without checking the link would be joining a
    /// network on the word of an unauthenticated stranger.
    pub fn invite(&self) -> &Invite {
        &self.invite
    }

    /// The proof, for a caller that wants to render or re-check it.
    pub fn link(&self) -> &CommonOwnershipProof {
        &self.link
    }

    /// Checks that this request really comes from `sender`, in `shared`.
    ///
    /// # The check that matters is the second one
    ///
    /// Verifying the proof's signatures is necessary and is not sufficient, and
    /// the gap between those is the whole of this function. A proof carrying two
    /// genuine signatures over a true statement about **somebody else** verifies
    /// perfectly — so a hostile sender can take any proof they have ever been
    /// shown, or make one about two identities they own, and present it. Core
    /// §1.2 says this in as many words: a valid proof says *some* pair is
    /// commonly owned, and a recipient has to compare that pair against the pair
    /// it expected.
    ///
    /// So this requires the proof to link exactly two things:
    ///
    /// 1. `sender` in `shared` — the identity the carrier already bound to the
    ///    connection, so the person asking is the person the recipient can see in
    ///    the network they share.
    /// 2. The invite's issuer in the invite's network — so the conversation being
    ///    offered is one that same person actually minted.
    ///
    /// Either half alone is useless. Without the first, anybody can forward
    /// Alice's request as their own; without the second, Alice can prove who she
    /// is and hand over an invite to a network she does not control.
    pub fn verify_from(
        &self,
        sender: &PerNetworkIdentityId,
        shared: &NetworkId,
    ) -> Result<(), CoreError> {
        self.link.verify().map_err(|_| CoreError::BadSignature)?;

        let issuer = self.invite.issuer;
        let conversation = self.invite.network;

        let (first, second) = self.link.linked();
        let here = (*shared, *sender);
        let there = (conversation, issuer);

        // Order-independent, because a proof stores its sides canonically (Core
        // §1.2) rather than in the order they were passed — so a check that
        // assumed an order would refuse half of all honest proofs, and which
        // half would depend on the keys.
        let matched = (first == here && second == there) || (first == there && second == here);
        if !matched {
            return Err(CoreError::UnlinkedInvite);
        }

        // A proof linking a network to *itself* proves nothing about a
        // conversation being separate from the network it was arranged in, and
        // §1.5's whole claim is that the two are different networks. Refused
        // rather than tolerated so the degenerate case cannot be used to make a
        // request look verified.
        if conversation == *shared {
            return Err(CoreError::UnlinkedInvite);
        }

        Ok(())
    }

    /// Encodes the payload.
    pub fn encode(&self) -> Vec<u8> {
        let mut e = intranet_crypto::Enc::domain(DM_INVITE_DOMAIN);
        e.bytes(&intranet_invite::encode_invite(&self.invite));
        e.bytes(&self.link.encode());
        e.finish()
    }

    /// Decodes a payload.
    ///
    /// Framing and both inner values only. **This does not check who it is
    /// from** — that needs the sender the carrier bound to the connection, which
    /// a decoder does not have, so it is [`verify_from`](Self::verify_from)'s and
    /// is separate deliberately: a decode that appeared to authenticate would be
    /// read as having done so.
    pub fn decode(bytes: &[u8]) -> Result<Self, CoreError> {
        let mut d = intranet_crypto::Dec::domain(bytes, DM_INVITE_DOMAIN)?;
        let invite_bytes = d.bytes()?;
        let link_bytes = d.bytes()?;
        d.finish()?;

        let invite =
            intranet_invite::decode_invite(invite_bytes).map_err(|_| CoreError::BadSignature)?;
        let link =
            CommonOwnershipProof::decode(link_bytes).map_err(|_| CoreError::BadSignature)?;
        Ok(Self { invite, link })
    }
}
