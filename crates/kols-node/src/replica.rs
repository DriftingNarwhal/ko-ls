//! Replica duty — Storage §3.3–§3.4, and `design/02` §6.4's contribution.
//!
//! # What this closes
//!
//! Content used to outlive its author because somebody happened to read it.
//! This client fetched what it wanted and served whatever it still had, so a
//! message survived its writer going offline exactly when a third member had
//! opened that channel. That is a happy accident, not durability — and it is
//! also why a storage offer meant nothing here: every byte on disk was this
//! node's own reading, and nothing was ever held *for* the network.
//!
//! Duty is the difference. A node computes the same placement every other node
//! computes, and keeps what it is ranked for whether or not anybody here wants
//! to read it.
//!
//! # Why the ranking is not ours to invent
//!
//! `intranet_ledger::placement` is the protocol's, deterministic given a ledger,
//! and weighted only by gossiped capacity. Reimplementing it — or weighting it
//! by anything local — would break the property the whole scheme rests on, that
//! every node computes the same answer (Storage §3.3, Core §4.6). This module
//! chooses *which keys* to rank over and does no arithmetic of its own.

use crate::store::Store;
use intranet_ledger::{CapabilityLedger, WeightField, placement};
use intranet_identity::PerNetworkIdentityId;
use intranet_storage::Cid;
use std::collections::BTreeMap;

/// How much room this node has for other members' content.
///
/// Two ceilings rather than one, because they answer different questions and
/// merging them would break one of the two rules this rests on (`02` §6.4):
/// `offered` is what this member gives *this network*, and `installation` is
/// what the application may take from their disk **altogether**, cache and duty
/// alike.
///
/// The second is the binding one when it binds. A member reading a great deal in
/// one network can leave no room for duty anywhere, and that is correct: the
/// ceiling is absolute, and what gives way under it is the contribution rather
/// than the member's own use of their own machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Budget {
    /// What this member offers this network.
    pub offered: u64,
    /// What this installation may use across every network.
    pub installation: u64,
    /// What every network here already costs this disk, duty and cache alike.
    pub installation_used: u64,
}

impl Budget {
    /// Bytes of new duty this node may still take on, given what it already holds.
    ///
    /// **Saturating on both sides deliberately.** A store already past either
    /// ceiling returns zero rather than underflowing into an enormous
    /// allowance — which is not a hypothetical, since a ceiling can be *lowered*
    /// below what is already held and that is exactly when the arithmetic must
    /// not hand out room that does not exist.
    pub fn remaining(&self, duty_held: u64) -> u64 {
        let by_offer = self.offered.saturating_sub(duty_held);
        let by_disk = self
            .installation
            .saturating_sub(self.installation_used);
        by_offer.min(by_disk)
    }
}

/// What one pass of duty evaluation found.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Duty {
    /// Objects considered.
    pub considered: usize,
    /// Objects this node is ranked for and now holds on the network's behalf.
    pub mine: usize,
    /// What those objects weigh, in bytes.
    ///
    /// Summed from the marks rather than the disk, so this is what the *duty
    /// tier* costs and never what the store costs — the two differ by everything
    /// this member fetched to read, which is not a contribution and must not be
    /// reported as one.
    pub mine_bytes: u64,
    /// Objects this node is ranked for and **refused**, being out of room.
    ///
    /// Covers both kinds of refusal — duty this node was ranked for, and repair
    /// it was the standby for — because a member out of room is out of room, and
    /// a second counter would imply repair has an allowance of its own.
    ///
    /// Not a failure: declining is how a ceiling works, and the repair loop
    /// places what this node did not take (Storage §3.4). It is counted because
    /// a member who offered less than the network wants to give them should be
    /// able to see that, and because a node silently ranked-for-and-not-holding
    /// is indistinguishable from one nobody ranked at all.
    pub refused: usize,
    /// Objects taken on as **repair**: under-replicated, and this node was the
    /// standby the shortfall reached (Storage §3.4).
    ///
    /// Counted apart from `mine` — which includes them — because they are the
    /// only duty here that placement did not ask for, and a member whose node is
    /// carrying a network's losses should be able to see that it is.
    pub adopted: usize,
    /// What this pass newly adopted, in bytes.
    pub adopted_bytes: u64,
    /// Repairs handed back, the shortfall they were taken for having closed.
    pub returned: usize,
    /// Objects whose replica set came back **smaller than the network asked
    /// for**, so fewer nodes hold them than its own policy requires.
    ///
    /// Surfaced rather than counted quietly, because Storage §3.2 asks for
    /// exactly this: a small or under-contributing network still functions, and
    /// "replication status should be observable so degraded durability is
    /// visible rather than silent". A member cannot volunteer more disk to a
    /// problem nobody told them about.
    pub under_replicated: usize,
}

/// Decides what this node holds for the network, and records it.
///
/// # The small-network case needs no special rule
///
/// `placement::select` returns *fewer* than asked for when too few nodes have
/// volunteered, which is Storage §3.2's degraded operation rather than an error.
/// So in a network with two contributing members and a replication factor of
/// three, both are selected for everything and everybody holds everything —
/// which is the desired behaviour, arrived at by the ordinary path rather than
/// by a branch that would have to be kept in step with it.
///
/// A node offering nothing is excluded from ranking entirely (`placement::rank`
/// refuses to conscript a zero-weight node), so contributing nothing means
/// exactly that, and it never costs the member the ability to read.
///
/// # Sealed segments only
///
/// A head segment is republished on every append, so its object id changes with
/// every message — ranking over it would re-place the whole network each time
/// somebody typed, and leave a trail of duty marks naming objects no longer
/// current. Sealed segments are immutable and are what history is made of, so
/// duty is computed over those and an author keeps its own head until it seals.
/// This also matches how retention is judged (`design/01` §8, per segment rather
/// than per log).
///
/// Sealed is decided by the chain rather than by a flag ([`sealed`]).
///
/// # Repair, and why it is the same function
///
/// Storage §3.4 asks for under-replicated content to be re-placed onto *the next
/// nodes in the same deterministic ranking* — not onto whoever noticed. So
/// repair is not a second policy layered over placement, it is the same ranking
/// read further down: a node ranked at `replication_factor + k` steps in when
/// the census says the object is `k + 1` or more copies short, and otherwise
/// does not. Every node computes the same ranking and reads the same count, so
/// the set that steps in is determined rather than raced.
///
/// This is what makes a generous member a real backstop instead of a statistical
/// one. Without it a node holds only what placement happened to rank it for, so
/// offering a great deal of disk catches content falling out of the network
/// exactly where it was already going to be caught.
///
/// # Ledgers converge; they do not agree
///
/// Placement is deterministic *given a ledger*, and two nodes hold different
/// ledgers for a propagation window (Storage §3.3, Core §4.5). This is written
/// to be correct either side of that: a node that wrongly takes duty holds a
/// harmless extra copy, and one that wrongly declines is caught when the ledger
/// settles and this runs again. Nothing here may require agreement at an
/// instant, and nothing here does.
pub fn evaluate(
    store: &Store,
    ledger: &CapabilityLedger,
    me: &PerNetworkIdentityId,
    replication_factor: usize,
    budget: Budget,
    census: &Census,
    now: i64,
) -> Result<Duty, String> {
    let mut duty = Duty::default();
    let mut room = budget.remaining(store.duty_bytes());
    let candidates: Vec<_> = ledger.entries().cloned().collect();
    let sealed = sealed(store);

    // **Repair is decided after placement, not beside it**, because the two
    // compete for one allowance and the order matters: an object this node is
    // actually ranked for must never lose its room to one this node merely
    // volunteered for. So pass one settles duty and collects the standby
    // positions it found, and pass two spends whatever is left.
    let mut standby: Vec<(Cid, usize)> = Vec::new();

    for cid in sealed.iter().copied() {
        duty.considered += 1;

        // One ranking rather than two calls, because repair needs to know *how
        // far past* the replica set this node sits and `select` throws that
        // away. The first `replication_factor` entries are exactly what `select`
        // would have returned.
        let ranked = placement::rank(
            cid.hash().as_bytes(),
            &candidates,
            WeightField::StorageOffered,
        );
        if ranked.len() < replication_factor {
            duty.under_replicated += 1;
        }
        let position = ranked.iter().position(|scored| scored.node == *me);

        match position {
            Some(index) if index < replication_factor => {
                if store.has_duty(&cid) {
                    // Already held. A ceiling refuses *new* duty; what is already
                    // promised is given up only with evidence somebody else holds
                    // it, which is a separate act and a separate round trip.
                    //
                    // If this was repair, it stops being repair: the ledger now
                    // ranks this node for it, so it is held under placement like
                    // anything else and ends under placement's rule.
                    if store.has_repair(&cid) {
                        store.clear_repair(&cid).map_err(|e| e.to_string())?;
                    }
                    duty.mine += 1;
                } else if let Some(bytes) = store.object_bytes(&cid) {
                    // **Declining is ordinary, and the ceiling is why this
                    // exists.** A node past either ceiling stops taking work
                    // rather than quietly exceeding what it promised, and
                    // Storage §3.4's repair places elsewhere what this node did
                    // not take.
                    if bytes > room {
                        duty.refused += 1;
                    } else {
                        room -= bytes;
                        store.take_duty(&cid, bytes).map_err(|e| e.to_string())?;
                        duty.mine += 1;
                    }
                }
                // An object whose weight cannot be read is counted as neither,
                // and marked as neither. Duty is a promise about disk, and one
                // nobody can size would escape the ceiling built on it — so this
                // waits for the next tick rather than recording a zero that
                // reads as free.
            }
            Some(index) => {
                let over = index - replication_factor;
                if !store.has_duty(&cid) {
                    standby.push((cid, over));
                } else if !store.has_repair(&cid) {
                    // Ranked out — by a member joining, or by somebody raising
                    // their offer. The mark goes and the bytes stay: this node
                    // may still be reading it, and duty is only ever one of the
                    // reasons to hold something.
                    store.release_duty(&cid).map_err(|e| e.to_string())?;
                } else if shortfall(census, &cid, replication_factor, now) == Some(0) {
                    // **The one way a repair ends: evidence the hole closed.** A
                    // standby is by definition outside the replica set, so the
                    // ordinary release rule above would give this back the
                    // instant it was taken. It goes when a *fresh* answer says
                    // the network has its target without this node — and an
                    // absent or stale answer keeps it, because not having been
                    // told is not the same as having been told nobody needs it.
                    store.release_duty(&cid).map_err(|e| e.to_string())?;
                    duty.returned += 1;
                } else {
                    duty.mine += 1;
                }
            }
            None => {
                // Not ranked at all, which for a node that offers storage means
                // it is not in the ledger this pass sees, and for one offering
                // nothing means exactly what it asked for. Either way there is
                // no standing to hold anything for the network.
                if store.has_duty(&cid) {
                    store.release_duty(&cid).map_err(|e| e.to_string())?;
                }
            }
        }
    }

    // Pass two: Storage §3.4's repair, over the room pass one left.
    for (cid, over) in standby {
        let Some(short) = shortfall(census, &cid, replication_factor, now) else {
            continue;
        };
        // **Depth follows the size of the hole**, which is what keeps this from
        // being a stampede. One missing copy wakes one standby; three missing
        // copies wake three. Every node computes the same ranking and the same
        // count, so the nodes that step in are the next ones down the list and
        // no more — the deterministic re-placement §3.4 asks for, rather than
        // whoever noticed first.
        if over >= short {
            continue;
        }
        let Some(bytes) = store.object_bytes(&cid) else {
            continue;
        };
        if bytes > room {
            // Counted with the other refusals: a member out of room is out of
            // room, and splitting the number would suggest repair has an
            // allowance of its own. It does not — that is the point.
            duty.refused += 1;
            continue;
        }
        room -= bytes;
        store.take_duty(&cid, bytes).map_err(|e| e.to_string())?;
        store.take_repair(&cid).map_err(|e| e.to_string())?;
        duty.mine += 1;
        duty.adopted += 1;
        duty.adopted_bytes += bytes;
    }

    duty.mine_bytes = store.duty_bytes();
    Ok(duty)
}

/// How many copies short of the network's target an object is, ignoring this node.
///
/// `None` when nobody has answered, which is the state this must never confuse
/// with zero: never having asked and having asked and learned that nobody holds
/// something are opposite answers, and only one of them is a reason to act.
///
/// This node's own copy is excluded deliberately — [`Census::heard`] counts
/// others — because the question being asked is *would the network still be
/// short if I promised nothing*. Counting a copy this node has not promised, and
/// may shed on the next tick, would answer a more comfortable question.
fn shortfall(census: &Census, cid: &Cid, replication_factor: usize, now: i64) -> Option<usize> {
    census
        .others_holding(cid, now)
        .map(|others| replication_factor.saturating_sub(others))
}

/// The segments this node holds that can never be republished.
///
/// Sealed is decided by the chain rather than by a flag: a head is nobody's
/// predecessor, so the set named as some held segment's `previous` is exactly the
/// set that is finished. A sealed segment whose successor this node does not hold
/// is excluded too, since nothing here names it — conservative in the safe
/// direction, since the cost is not acting on something rather than acting on it
/// wrongly.
pub fn sealed(store: &Store) -> std::collections::BTreeSet<Cid> {
    store
        .segments()
        .iter()
        .filter_map(|cid| store.segment_link(cid)?.1)
        .collect()
}

/// What this node has learned about who else holds its duty objects.
///
/// # Why this cannot be a function
///
/// Everything else in this module is a pure pass: hand it a ledger and a store
/// and it answers. Asking *who actually holds this* is not — `find_providers`
/// returns a query id and the answer arrives later as a `ProvidersFound` event,
/// possibly never. So the question has state: what has been asked, when, and
/// what came back.
///
/// # Why a provider count is not proof, and what it is good for
///
/// Kademlia provider records are a hint rather than a promise: they outlive the
/// node that stopped holding the bytes, and enumeration is best-effort
/// (Storage §2.5). That makes the count safe in exactly one direction. It may
/// **overstate** who holds something, so it must never be the reason to drop a
/// replica on its own — the last copy would go while three nodes each believed
/// another had it. It may also understate, which costs only a replica this node
/// keeps longer than it had to.
///
/// So: adequate for *reporting* under-replication, and adequate as the first
/// half of an eviction decision that a direct ask confirms.
#[derive(Debug, Default)]
pub struct Census {
    /// Asked and not yet answered, with when the question went out.
    pending: BTreeMap<Cid, i64>,
    /// What came back: how many *other* nodes hold it, and when we heard.
    heard: BTreeMap<Cid, (usize, i64)>,
}

/// How long an answer is treated as current, in milliseconds.
///
/// Ten minutes. Long enough that a busy node is not re-asking the network about
/// the same object every tick, short enough that a count is not acted on long
/// after the members behind it left. Provider records carry their own TTL and
/// this is deliberately shorter than any of them.
pub const ANSWER_FRESH_MILLIS: i64 = 10 * 60 * 1000;

/// How long to wait before asking again about something nobody answered.
///
/// A query that resolves to nothing looks identical to one still in flight, so
/// an unanswered ask has to time out rather than be waited on forever.
pub const ASK_TIMEOUT_MILLIS: i64 = 60 * 1000;

impl Census {
    /// Whether this object is worth asking about now.
    ///
    /// False while a question is outstanding and not yet stale, and while a
    /// recent answer is still current — the two states that make re-asking
    /// waste rather than diligence.
    pub fn should_ask(&self, cid: &Cid, now: i64) -> bool {
        if let Some(asked) = self.pending.get(cid)
            && now.saturating_sub(*asked) < ASK_TIMEOUT_MILLIS
        {
            return false;
        }
        match self.heard.get(cid) {
            Some((_, at)) => now.saturating_sub(*at) >= ANSWER_FRESH_MILLIS,
            None => true,
        }
    }

    /// Records that a question went out.
    pub fn asked(&mut self, cid: Cid, now: i64) {
        self.pending.insert(cid, now);
    }

    /// Records an answer, counting holders **other than this node**.
    ///
    /// Excluding ourselves is the whole point of the number: the question being
    /// asked is *would this survive without me*, and a count that included this
    /// node would answer a different one and always be off by exactly the amount
    /// that matters.
    pub fn heard(&mut self, cid: Cid, providers: &[PerNetworkIdentityId], me: &PerNetworkIdentityId, now: i64) {
        self.pending.remove(&cid);
        let others = providers.iter().filter(|node| *node != me).count();
        self.heard.insert(cid, (others, now));
    }

    /// How many other nodes are currently believed to hold this, if it is known.
    ///
    /// `None` means *this node has not been told*, which is not zero and must
    /// never be read as it: never having asked and having asked and learned that
    /// nobody holds something are opposite answers, and only one of them is a
    /// reason to act.
    pub fn others_holding(&self, cid: &Cid, now: i64) -> Option<usize> {
        self.heard
            .get(cid)
            .filter(|(_, at)| now.saturating_sub(*at) < ANSWER_FRESH_MILLIS)
            .map(|(others, _)| *others)
    }

    /// Forgets everything about an object this node no longer holds.
    pub fn forget(&mut self, cid: &Cid) {
        self.pending.remove(cid);
        self.heard.remove(cid);
    }
}

/// How long a segment somebody asked for is safe from being shed.
///
/// An hour. Long enough that reading back through history is not a race against
/// the ceiling, short enough that a page nobody returned to stops being special.
/// A member who pins more than their ceiling holds is told rather than served
/// silently wrong, which is what `Shed::still_over` is for.
pub const PIN_MILLIS: i64 = 60 * 60 * 1000;

/// What one shedding pass gave back.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Shed {
    /// Objects whose servable copy was dropped.
    pub objects: usize,
    /// Bytes reclaimed.
    pub bytes: u64,
    /// The chunk ids that are gone, so the caller can stop announcing them.
    ///
    /// A node that drops bytes and keeps advertising them sends every peer that
    /// believes it on a fetch that fails, and a failed fetch counts against the
    /// serving node with whoever asked. Shedding quietly is worse than not
    /// shedding.
    pub dropped: Vec<Cid>,
    /// Bytes still over the ceiling after shedding everything sheddable.
    ///
    /// Non-zero means the cheap half is exhausted and what is left is either
    /// this member's own records or duty this node promised — neither of which
    /// this pass will touch. It is the signal that the next decision is a real
    /// one rather than more of the same.
    pub still_over: u64,
}

/// Sheds cached copies until the store is back under its ceiling.
///
/// # Why this needs no evidence and duty eviction does
///
/// A cached copy is not a replica. The nodes placement ranked for an object are
/// the ones responsible for it, so dropping this node's copy is giving up a
/// spare rather than a copy anybody was counting on, and none of the census, the
/// two-holders rule or the grace window applies. That machinery exists for
/// **duty**, where dropping could lose the object, and confusing the two would
/// either make this pass impossibly expensive or make that one unsafe.
///
/// # Where that stops being true, and why this still does not change
///
/// The ranked holders can be *gone* — that is the whole reason repair exists —
/// so a cached copy of under-replicated content is not always a spare. This pass
/// still drops it, and the reason is the cap rather than an oversight: adopting
/// it would mean promising bytes this node has no room for, and the ceiling is
/// absolute (`02` §6.4). What resolves the tension is order. Repair runs after
/// shedding and takes on only what fits, so a node with room keeps the copy as
/// duty — where the census, the two-holders rule and the grace window all
/// protect it — and a node at its ceiling gives it up, which is the same answer
/// it gives to every other demand on a disk that is full.
///
/// # What is shed, and what is never
///
/// Only **sealed, non-duty** segments, oldest in their chain first.
///
/// - *Sealed*, because a head is republished on every append and successive
///   versions share every chunk but the tail — deleting an old head's chunks
///   takes the current one's with them.
/// - *Non-duty*, because duty is a promise to the network and is given up under
///   §5's rule rather than under disk pressure.
/// - *Oldest first*, because within a chain that is the history a member is
///   least likely to be reading. Across chains the order is arbitrary, which is
///   stated rather than dressed up: nothing here records when a segment was
///   absorbed, so there is no better signal to sort on yet.
///
/// **Records are never touched.** They are what rendering reads and they are the
/// member's own history; this drops the *servable* copy, so the cost falls on
/// what this node can give the network and not on what its member can see. That
/// asymmetry is why this is the first thing shed and why it can be done without
/// asking anybody.
pub fn shed_cache(store: &Store, over_by: u64, now: i64) -> Result<Shed, String> {
    let mut shed = Shed {
        still_over: over_by,
        ..Shed::default()
    };
    if over_by == 0 {
        return Ok(shed);
    }

    let sealed = sealed(store);

    let mut candidates: Vec<(u64, Cid)> = sealed
        .iter()
        .filter(|cid| !store.has_duty(cid))
        // **Not what somebody just asked for.** Shedding takes the oldest
        // history first, which is exactly what a member scrolling back has gone
        // looking for — so without this a node at its ceiling would fetch a page
        // and throw it away before anybody read it, every time, forever. Age is
        // a decent guess at what is cold and a request is direct evidence that
        // something is not.
        .filter(|cid| {
            store
                .wanted_at(cid)
                .is_none_or(|at| now.saturating_sub(at) >= PIN_MILLIS)
        })
        .filter_map(|cid| Some((store.segment_link(cid)?.0, *cid)))
        .collect();
    // Oldest first, with the id as a tie-break so two nodes in the same state
    // shed the same things — not required for correctness, and it makes a
    // divergence between two members something worth investigating rather than
    // something to shrug at.
    candidates.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));

    for (_, cid) in candidates {
        if shed.still_over == 0 {
            break;
        }
        // Weighed before it goes, since the weight is read from the very bytes
        // being removed.
        let Some(freed) = store.object_bytes(&cid) else {
            continue;
        };
        let gone = store.forget_object(&cid).map_err(|e| e.to_string())?;
        if gone.is_empty() {
            continue;
        }
        shed.objects += 1;
        shed.bytes += freed;
        shed.still_over = shed.still_over.saturating_sub(freed);
        shed.dropped.extend(gone);
    }
    Ok(shed)
}

/// How many *other* nodes must be known to hold something before it is given up.
///
/// Two, clamped to how many other members have volunteered any storage at all —
/// a network of three cannot produce evidence of four, and demanding it would
/// mean nothing is ever evictable and the ceiling stops binding.
///
/// **Why two rather than one.** Dropping at one leaves the object a single disk
/// failure from gone, and the whole reason a replica exists is that one node is
/// not enough. Two leaves it degraded rather than lost, which the repair loop
/// (Storage §3.4) is built to correct and a member is told about.
pub const SAFE_HOLDERS: usize = 2;

/// How long the last known copy of something is held past the ceiling.
///
/// Seven days. **Exceeding a ceiling briefly to avoid destroying data is the
/// right way round**, and the window is bounded in both directions: in time by
/// this, and in size by the fact that it is reached only after cache and every
/// evictable replica have already gone.
///
/// A member who is away for a day does not lose content they would have saved;
/// a member who is away for a month has, and was told.
pub const GRACE_MILLIS: i64 = 7 * 24 * 60 * 60 * 1000;

/// What one duty-eviction pass did, and what it could not do.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Evict {
    /// Replicas given up because others demonstrably hold them.
    pub objects: usize,
    /// Bytes reclaimed.
    pub bytes: u64,
    /// Ids no longer held, so the caller can stop announcing them.
    pub dropped: Vec<Cid>,
    /// Objects this node may be the last holder of, and is holding anyway.
    ///
    /// The number a member has to be told, in the words that say what it means:
    /// this network is about to lose these unless somebody makes room.
    pub at_risk: Vec<Cid>,
    /// Objects given up **despite** being the last known copy, the window having run.
    pub lost: Vec<Cid>,
    /// Candidates whose holders are not yet known, so nothing can be decided.
    ///
    /// Distinct from `at_risk` on purpose: *nobody answered* and *nobody holds
    /// it* are opposite states, and treating the first as the second is how a
    /// last copy gets dropped because the network was slow.
    pub awaiting: Vec<Cid>,
    /// Bytes still over the ceiling once everything permitted has gone.
    pub still_over: u64,
}

/// Gives up replicas that others demonstrably hold, and reports what it cannot.
///
/// # The order this runs in, and why it is last
///
/// Cache goes first (`shed_cache`): a cached copy is refetchable and dropping it
/// cannot lose anything. Only when that is exhausted does a node start giving
/// back what it *promised*, and only with evidence. What is left after this is
/// content that would be lost, which is a different kind of decision and gets a
/// window and a warning rather than a rule.
///
/// # Evidence, and the one direction a provider count is safe in
///
/// A provider count may **overstate** who holds something — records outlive the
/// bytes they name (Storage §2.5) — so it can never be the sole reason to drop a
/// replica. It is used here as a *filter*, not a proof: an object nobody claims
/// to hold is certainly not safe, and one that several claim to hold is a
/// candidate the count alone does not settle. That is why `SAFE_HOLDERS` is two
/// rather than one, and why a stale answer reads as unknown rather than as its
/// last value.
pub fn shed_duty(
    store: &Store,
    census: &Census,
    over_by: u64,
    others_contributing: usize,
    now: i64,
) -> Result<Evict, String> {
    let mut evict = Evict {
        still_over: over_by,
        ..Evict::default()
    };
    if over_by == 0 {
        return Ok(evict);
    }
    // A network cannot be asked for evidence it has no members to produce.
    let enough = SAFE_HOLDERS.min(others_contributing);

    let mut candidates: Vec<(u64, Cid)> = store
        .segments()
        .into_iter()
        .filter(|cid| store.has_duty(cid))
        .filter_map(|cid| Some((store.segment_link(&cid)?.0, cid)))
        .collect();
    candidates.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));

    for (_, cid) in candidates {
        if evict.still_over == 0 {
            break;
        }
        let Some(others) = census.others_holding(&cid, now) else {
            // Nobody has answered. Not a reason to keep it forever and not a
            // reason to drop it — a reason to ask, which the caller does.
            evict.awaiting.push(cid);
            continue;
        };

        if others >= enough && enough > 0 {
            let _ = store.clear_at_risk(&cid);
            let Some(freed) = store.object_bytes(&cid) else {
                continue;
            };
            let gone = store.forget_object(&cid).map_err(|e| e.to_string())?;
            if gone.is_empty() {
                continue;
            }
            evict.objects += 1;
            evict.bytes += freed;
            evict.still_over = evict.still_over.saturating_sub(freed);
            evict.dropped.extend(gone);
            continue;
        }

        // Not enough holders. This node may be the last, so the window starts —
        // and until it runs out, the ceiling gives way rather than the content.
        store.mark_at_risk(&cid, now).map_err(|e| e.to_string())?;
        let since = store.at_risk_since(&cid).unwrap_or(now);
        if now.saturating_sub(since) < GRACE_MILLIS {
            evict.at_risk.push(cid);
            continue;
        }

        // The window has run. This is the one place in the client that gives up
        // content knowing it may be gone from the network, so it is written down
        // rather than merely done.
        let Some(freed) = store.object_bytes(&cid) else {
            continue;
        };
        let gone = store.forget_object(&cid).map_err(|e| e.to_string())?;
        if gone.is_empty() {
            continue;
        }
        store.record_dropped(&cid, now).map_err(|e| e.to_string())?;
        let _ = store.clear_at_risk(&cid);
        evict.objects += 1;
        evict.bytes += freed;
        evict.still_over = evict.still_over.saturating_sub(freed);
        evict.dropped.extend(gone);
        evict.lost.push(cid);
    }
    Ok(evict)
}
