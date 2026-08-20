//! Member display names — spec 07 §3.9.
//!
//! # Why a name is in the governance log
//!
//! A member's avatar and status live in a mutable pointer they own, and a name
//! could have too — except that uniqueness is a question about the whole network
//! at a moment in time, and a single-writer object cannot answer it. Two members
//! publishing the same name to their own pointers produces two nodes that
//! disagree about who is who, with nothing to settle it. Ordering is what makes
//! uniqueness decidable, and the log is what has ordering.
//!
//! # The claim carries no identity
//!
//! It binds whoever authored the entry, which the protocol has already verified.
//! There is no field in which to name somebody else, so claiming a name on
//! another member's behalf is not refused — it is unsayable.

use crate::CoreError;
use intranet_crypto::{Dec, Enc};
use intranet_governance::{Capability, EntryBody, GovernanceState};
use intranet_identity::PerNetworkIdentityId;
use std::collections::BTreeMap;
use unicode_normalization::UnicodeNormalization;
use unicode_segmentation::UnicodeSegmentation;

/// Domain tag for a name claim payload — spec 07 §3.2.
pub const NAME_CLAIM_DOMAIN: &str = "intranet.chat-name-claim.v1";

/// The `kind` string a name claim travels under.
pub const NAME_CLAIM_KIND: &str = "member-name";

/// The payload discriminant — spec 07 §3.9.
pub const NAME_CLAIM_TAG: u8 = 0x10;

/// The largest a claimed name may be, as UTF-8.
pub const MAX_NAME_BYTES: usize = 64;

/// The largest a normalized name may be, in extended grapheme clusters.
pub const MAX_NAME_GRAPHEMES: usize = 32;

/// Why a name was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NameRefusal {
    /// It carried a code point that cannot be seen or rendered.
    ///
    /// A name nobody can see cannot be checked by the person it misleads, which
    /// is the whole reason these are refused rather than stripped.
    Invisible {
        /// The offending code point.
        code_point: u32,
    },
    /// It normalized to nothing.
    Empty,
    /// The claimed form is longer than [`MAX_NAME_BYTES`].
    TooManyBytes {
        /// How many it carried.
        actual: usize,
    },
    /// The normalized form occupies more than [`MAX_NAME_GRAPHEMES`].
    TooManyGraphemes {
        /// How many it occupies.
        actual: usize,
    },
    /// Another identity already holds this name key.
    Taken {
        /// Who holds it.
        holder: PerNetworkIdentityId,
    },
}

impl std::fmt::Display for NameRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Invisible { code_point } => write!(
                f,
                "that name contains an invisible character (U+{code_point:04X})"
            ),
            Self::Empty => write!(f, "that name is empty"),
            Self::TooManyBytes { actual } => {
                write!(f, "that name is {actual} bytes, and the limit is {MAX_NAME_BYTES}")
            }
            Self::TooManyGraphemes { actual } => write!(
                f,
                "that name is {actual} characters wide, and the limit is {MAX_NAME_GRAPHEMES}"
            ),
            Self::Taken { holder } => {
                write!(f, "that name is held by {}", holder.short())
            }
        }
    }
}

impl std::error::Error for NameRefusal {}

/// The key two names are compared on — spec 07 §3.9.1.
///
/// Every step is ordered and exhaustive because two nodes that normalized
/// differently would disagree about what is a duplicate, which is a consensus
/// bug wearing the clothes of a display concern.
///
/// Honest limit: this is only as stable as the Unicode tables the build embeds.
/// The stability policy fixes normalization for code points that already exist,
/// so only newly assigned ones are in play, and the disagreement is confined to
/// the one name involved.
pub fn name_key(claimed: &str) -> Result<String, NameRefusal> {
    if claimed.len() > MAX_NAME_BYTES {
        return Err(NameRefusal::TooManyBytes {
            actual: claimed.len(),
        });
    }

    // 1. Invisible code points, refused rather than stripped.
    for character in claimed.chars() {
        if is_invisible(character) {
            return Err(NameRefusal::Invisible {
                code_point: character as u32,
            });
        }
    }

    // 2. NFKC. 3. Trim, then collapse internal whitespace runs to one space.
    let normalized: String = claimed.nfkc().collect();
    let collapsed = normalized
        .split_whitespace()
        .collect::<Vec<_>>()
        .join("\u{20}");

    // 4. Unicode default lowercase mapping.
    let key = collapsed.to_lowercase();

    // 5. Bounds, on the normalized form.
    if key.is_empty() {
        return Err(NameRefusal::Empty);
    }
    let graphemes = key.graphemes(true).count();
    if graphemes > MAX_NAME_GRAPHEMES {
        return Err(NameRefusal::TooManyGraphemes { actual: graphemes });
    }
    Ok(key)
}

/// Whether a code point may not appear in a name.
///
/// `Cc`, `Cf`, `Cs`, `Co` and `Cn` — control, format, surrogate, private use and
/// unassigned. Rust's `char` cannot hold a surrogate, so that case is structural
/// rather than checked. Unassigned is approximated by `char::is_alphanumeric`
/// and friends returning false for everything in the class; what matters is that
/// the invisible ones are gone.
fn is_invisible(character: char) -> bool {
    character.is_control()
        || matches!(character,
            // Cf: format characters, including the bidirectional overrides that
            // let a name render as something other than what it encodes, and the
            // zero-width joiners that let two distinct names look identical.
            '\u{00AD}' | '\u{0600}'..='\u{0605}' | '\u{061C}' | '\u{06DD}' | '\u{070F}'
            | '\u{08E2}' | '\u{180E}' | '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}'
            | '\u{2060}'..='\u{2064}' | '\u{2066}'..='\u{206F}' | '\u{FEFF}'
            | '\u{FFF9}'..='\u{FFFB}' | '\u{110BD}' | '\u{1D173}'..='\u{1D17A}'
            | '\u{E0001}' | '\u{E0020}'..='\u{E007F}'
            // Co: private use, which renders as whatever a font decides.
            | '\u{E000}'..='\u{F8FF}' | '\u{F0000}'..='\u{FFFFD}' | '\u{100000}'..='\u{10FFFD}')
}

/// A claim on a display name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NameClaim {
    /// The name exactly as claimed, which is what gets displayed.
    pub name: String,
}

impl NameClaim {
    /// Builds a claim, refusing one that could never bind.
    pub fn new(name: impl Into<String>) -> Result<Self, NameRefusal> {
        let name = name.into();
        name_key(&name)?;
        Ok(Self { name })
    }

    /// The key this claim would bind.
    pub fn key(&self) -> Result<String, NameRefusal> {
        name_key(&self.name)
    }

    /// The payload's canonical bytes — spec 07 §3.9.
    pub fn encode(&self) -> Vec<u8> {
        let mut e = Enc::domain(NAME_CLAIM_DOMAIN);
        e.str(&self.name);
        e.finish()
    }

    /// Decodes a payload. Does not check that it may bind.
    pub fn decode_payload(bytes: &[u8]) -> Result<Self, CoreError> {
        let mut d = Dec::domain(bytes, NAME_CLAIM_DOMAIN)?;
        let name = d.str()?.to_owned();
        Ok(Self { name })
    }

    /// The capability that authorizes this claim.
    ///
    /// One name, network-wide, so there is one scope. It is ordinary and expected
    /// on `everyone`: it authorizes a claim on the claimant's own name and
    /// nothing else, because the payload has no field naming anybody.
    pub fn required() -> Capability {
        Capability::extension("chat:set-name:*".to_owned())
    }

    /// Packages this claim as an application entry body.
    pub fn to_app_entry(&self) -> EntryBody {
        EntryBody::AppEntry {
            namespace: crate::CHAT_NAMESPACE.to_owned(),
            kind: NAME_CLAIM_KIND.to_owned(),
            required: Self::required(),
            payload: self.encode(),
        }
    }

    /// Reads a claim out of a governance log entry body.
    ///
    /// The only way to obtain one from replayed state, so the capability check
    /// cannot be skipped — the same discipline `ChannelEntry::read` enforces,
    /// and for the same reason: the protocol verified the author held what the
    /// entry *declared*, and only a reader that understands `chat` knows what it
    /// should have declared.
    pub fn read(body: &EntryBody) -> Option<Self> {
        let EntryBody::AppEntry {
            namespace,
            kind,
            required,
            payload,
        } = body
        else {
            return None;
        };
        if namespace != crate::CHAT_NAMESPACE || kind != NAME_CLAIM_KIND {
            return None;
        }
        if required != &Self::required() {
            return None;
        }
        Self::decode_payload(payload).ok()
    }
}

/// Who holds which name, as replay understands it — spec 07 §3.9.2.
///
/// Built by folding the canonical chain in order. A key binds to the first
/// identity that claims it and **never unbinds**, including after that member
/// leaves: history renders by author id with names resolved at display time, so
/// an inherited name would silently relabel somebody else's past messages.
#[derive(Debug, Default, Clone)]
pub struct Names {
    by_key: BTreeMap<String, PerNetworkIdentityId>,
    current: BTreeMap<PerNetworkIdentityId, String>,
}

impl Names {
    /// An empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Applies one claim, authored by `author`.
    ///
    /// Returns whether it bound. A claim on a key another identity holds is
    /// ignored rather than fatal — the entry stays in the log and has no effect,
    /// exactly as an unauthorized channel entry does.
    pub fn apply(&mut self, author: PerNetworkIdentityId, claim: &NameClaim) -> bool {
        let Ok(key) = claim.key() else {
            return false;
        };
        match self.by_key.get(&key) {
            Some(holder) if holder != &author => false,
            _ => {
                self.by_key.insert(key, author);
                self.current.insert(author, claim.name.clone());
                true
            }
        }
    }

    /// This member's current display name, if they have claimed one.
    pub fn of(&self, identity: &PerNetworkIdentityId) -> Option<&str> {
        self.current.get(identity).map(String::as_str)
    }

    /// Who holds this name key, if anybody.
    pub fn holder(&self, key: &str) -> Option<&PerNetworkIdentityId> {
        self.by_key.get(key)
    }

    /// Whether this name is claimable by `author`.
    pub fn claimable(
        &self,
        author: &PerNetworkIdentityId,
        name: &str,
    ) -> Result<(), NameRefusal> {
        let key = name_key(name)?;
        match self.by_key.get(&key) {
            Some(holder) if holder != author => Err(NameRefusal::Taken { holder: *holder }),
            _ => Ok(()),
        }
    }

    /// How many names are bound.
    pub fn len(&self) -> usize {
        self.by_key.len()
    }

    /// Whether no name is bound.
    pub fn is_empty(&self) -> bool {
        self.by_key.is_empty()
    }
}

/// Folds a network's canonical chain into its name registry.
///
/// Takes entries already ordered by replay, so the "first claim wins" rule is
/// the log's order rather than anything computed here.
pub fn replay_names<'a>(
    entries: impl IntoIterator<Item = &'a intranet_governance::LogEntry>,
    state: &GovernanceState,
) -> Names {
    let mut names = Names::new();
    for entry in entries {
        let Some(claim) = NameClaim::read(&entry.body) else {
            continue;
        };
        // The protocol verified the author holds what the entry declared; this
        // is the reader's half, checking the declaration was the right one for
        // the kind. `read` already refused a wrong declaration, so what remains
        // is that the author still holds it in current state.
        if !state.identity_holds(&entry.author, &NameClaim::required()) {
            continue;
        }
        names.apply(entry.author, &claim);
    }
    names
}
