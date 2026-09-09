//! The export that makes an identity survive its machine — `design/02` §6.3.
//!
//! # This is portability, not recovery
//!
//! Calling it recovery is wrong and the distinction decides what it is for.
//! There is no organisation, no reset and nobody responsible but the person at
//! the keyboard; an account that could be *recovered* would contradict the
//! architecture.
//!
//! What it is for is that **the seed is the member, and it lives on exactly one
//! disk.** The account password protects it from somebody who takes the laptop.
//! Nothing protects it from the laptop dying. On a new machine a member without
//! their seed is not locked out — they are a different person: their
//! capabilities do not follow, their display name stays bound to the old
//! identity, their own messages remain authored by somebody they can no longer
//! act as, and getting back in needs a holder of `approve-node` to admit them
//! afresh. A sole Founder may find there is nobody who can.
//!
//! # Why it is a bundle and not a phrase
//!
//! A phrase alone restores nothing. Coming back needs the seed, the network's
//! id, and an address to reach the network at — and a network id cannot be
//! derived from a seed. The list of networks a member belongs to lives in the
//! workspace and in no seed at all, so what has to survive is a **set**.
//!
//! # Why its own passphrase
//!
//! It leaves the machine, so the machine's protection does not travel with it: a
//! plaintext copy on a USB stick or in a cloud drive is every identity in the
//! clear, which is the problem the keyring exists to fix, relocated. And
//! wrapping it under the *account* password would mean one forgotten secret
//! loses both the keyring and the thing kept in case the keyring is lost.
//!
//! A passphrase chosen at export is used once and can be written on paper.

use crate::account::AccountError;
use intranet_crypto::{Dec, DecodeError, Enc};
use intranet_identity::NetworkId;
use intranet_storage::Dek;

/// The domain the payload is encoded under.
const DOMAIN: &str = "kols.bundle.v1";

/// The domain the passphrase key is separated under.
const KEY_DOMAIN: &str = "kols.bundle.key.v1";

/// What the file starts with, so a wrong file is refused rather than decrypted.
const MAGIC: &[u8; 8] = b"KOLSBNDL";

/// How the file is laid out, so a later change can tell.
const VERSION: u8 = 1;

/// Argon2id parameters for the bundle.
///
/// Heavier than the account's, deliberately: this file is the one that leaves
/// the machine and may sit in a cloud drive for years, so the guessing budget an
/// attacker can bring to it is unbounded in a way it is not for a local file.
/// The cost is paid twice in a lifetime — once at export, once at import.
const MEMORY_KIB: u32 = 256 * 1024;
const PASSES: u32 = 4;
const LANES: u32 = 1;

/// One network, as a bundle carries it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// Which network. Cannot be derived from the seed, which is why it is here.
    pub network: NetworkId,
    /// The entropy this member's identity in that network derives from.
    pub seed: [u8; 32],
    /// The local label, so a restored workspace is not a list of hex.
    pub label: String,
    /// Somewhere to reach the network. Without one there is nobody to sync from.
    pub relays: Vec<String>,
}

/// What can go wrong reading or writing a bundle.
#[derive(Debug)]
pub enum BundleError {
    /// The file is not one of these.
    NotABundle,
    /// It was written by a version this build does not read.
    Version(u8),
    /// The passphrase did not open it.
    WrongPassphrase,
    /// It opened and its contents were not readable.
    Malformed(String),
    /// A key could not be derived from the passphrase.
    Kdf(String),
}

impl std::fmt::Display for BundleError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotABundle => write!(f, "that file is not a ko-ls backup"),
            Self::Version(v) => write!(
                f,
                "that backup was written by a newer version of ko-ls (format {v})"
            ),
            Self::WrongPassphrase => write!(f, "that passphrase does not open this backup"),
            Self::Malformed(what) => write!(f, "the backup opened but is unreadable: {what}"),
            Self::Kdf(what) => write!(f, "could not derive a key from that passphrase: {what}"),
        }
    }
}

impl std::error::Error for BundleError {}

impl From<DecodeError> for BundleError {
    fn from(err: DecodeError) -> Self {
        Self::Malformed(err.to_string())
    }
}

/// Seals a set of networks under a passphrase.
pub fn seal(entries: &[Entry], passphrase: &str) -> Result<Vec<u8>, BundleError> {
    let mut salt = [0u8; 16];
    intranet_crypto::random_bytes(&mut salt).map_err(|err| BundleError::Kdf(format!("{err:?}")))?;
    let key = derive(passphrase, &salt, MEMORY_KIB, PASSES, LANES)?;

    let mut enc = Enc::domain(DOMAIN);
    enc.seq(entries.iter(), |e, entry| {
        e.fixed(entry.network.as_bytes())
            .fixed(&entry.seed)
            .str(&entry.label)
            .seq(entry.relays.iter(), |e, relay| {
                e.str(relay);
            });
    });

    let mut out = Vec::new();
    out.extend_from_slice(MAGIC);
    out.push(VERSION);
    out.extend_from_slice(&MEMORY_KIB.to_be_bytes());
    out.extend_from_slice(&PASSES.to_be_bytes());
    out.extend_from_slice(&LANES.to_be_bytes());
    out.extend_from_slice(&salt);
    out.extend_from_slice(&Dek::from_bytes(key).seal_chunk(&enc.finish()));
    Ok(out)
}

/// Opens one.
pub fn open(raw: &[u8], passphrase: &str) -> Result<Vec<Entry>, BundleError> {
    // **The magic is checked before the passphrase is even derived.** Somebody
    // who picked the wrong file should be told that, rather than being told
    // their passphrase is wrong and going looking for it.
    if raw.len() < 8 + 1 + 12 + 16 || &raw[..8] != MAGIC {
        return Err(BundleError::NotABundle);
    }
    if raw[8] != VERSION {
        return Err(BundleError::Version(raw[8]));
    }
    let u32_at = |at: usize| u32::from_be_bytes([raw[at], raw[at + 1], raw[at + 2], raw[at + 3]]);
    let (memory, passes, lanes) = (u32_at(9), u32_at(13), u32_at(17));
    let salt: [u8; 16] = raw[21..37]
        .try_into()
        .map_err(|_| BundleError::Malformed("salt".to_owned()))?;

    let key = derive(passphrase, &salt, memory, passes, lanes)?;
    let plain = Dek::from_bytes(key)
        .open_chunk(&raw[37..])
        .map_err(|_| BundleError::WrongPassphrase)?;

    let mut dec = Dec::domain(&plain, DOMAIN)?;
    let count = dec.u64()?;
    let mut entries = Vec::new();
    for _ in 0..count {
        let network = NetworkId::from_bytes(dec.fixed::<32>()?);
        let seed = dec.fixed::<32>()?;
        let label = dec.str()?.to_owned();
        let relay_count = dec.u64()?;
        let mut relays = Vec::new();
        for _ in 0..relay_count {
            relays.push(dec.str()?.to_owned());
        }
        entries.push(Entry {
            network,
            seed,
            label,
            relays,
        });
    }
    Ok(entries)
}

/// Turns a passphrase into a key.
fn derive(
    passphrase: &str,
    salt: &[u8; 16],
    memory: u32,
    passes: u32,
    lanes: u32,
) -> Result<[u8; 32], BundleError> {
    let params = argon2::Params::new(memory, passes, lanes, Some(32))
        .map_err(|err| BundleError::Kdf(err.to_string()))?;
    let argon =
        argon2::Argon2::new(argon2::Algorithm::Argon2id, argon2::Version::V0x13, params);
    let mut key = [0u8; 32];
    argon
        .hash_password_into(passphrase.as_bytes(), salt, &mut key)
        .map_err(|err| BundleError::Kdf(err.to_string()))?;
    // Separated under its own domain, so the bundle's key is not the same value
    // as anything else a passphrase might be used for.
    Ok(*intranet_crypto::keyed_hash(&key, KEY_DOMAIN.as_bytes()).as_bytes())
}

impl From<AccountError> for BundleError {
    fn from(err: AccountError) -> Self {
        Self::Malformed(err.to_string())
    }
}
