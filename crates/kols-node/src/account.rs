//! The local account whose password wraps the seeds — `design/02` §6.3.
//!
//! # The one thing this must not do
//!
//! **The password wraps the seeds. It never derives them.** Deriving a seed from
//! the password and the network id needs no storage at all and is badly wrong:
//! the network id is public — it travels in every invite and every address — and
//! member ids are in the governance log, so an attacker could derive a candidate
//! identity from a guessed password and **check it offline against a value the
//! network publishes**. That is a brainwallet with a verification oracle.
//!
//! A random seed wrapped under a password-derived key has no such oracle. There
//! is nothing public to check a guess against, and the seed keeps its full
//! entropy however weak the password is.
//!
//! # This is not a new mechanism
//!
//! Everything else this store keeps at rest — the MLS group state, the epoch
//! keys, every DEK wrapping — is already sealed under a *seed*-derived key. The
//! seed was the one file left in the clear. So this is the same construction one
//! level up, and the only genuinely new part is where the secret comes from,
//! because there is no seed above the seed.

use crate::secret;
use intranet_storage::Dek;
use std::fs;
use std::path::{Path, PathBuf};

/// How the account file is laid out, so a later change can tell.
const VERSION: u8 = 1;

/// The domain the account key is separated under.
const WRAP_DOMAIN: &str = "kols.account.seed-wrap.v1";

/// What a wrong password produces, so it can be told from a broken file.
const CHECK_DOMAIN: &str = "kols.account.check.v1";

/// Argon2id parameters.
///
/// # Why these numbers
///
/// 64 MiB and three passes is the `OWASP` second-choice profile, and it is the
/// right corner of the trade for a desktop client: the memory cost is what makes
/// a guessing rig expensive, and 64 MiB is affordable on any machine that can
/// run a webview while still costing an attacker real silicon per guess. One
/// lane, because parallelism here would buy the attacker as much as the user.
///
/// **Stored in the file rather than assumed**, so raising them later does not
/// make every existing account unopenable — a reader uses what its own file
/// says, not what this build prefers.
const MEMORY_KIB: u32 = 64 * 1024;
const PASSES: u32 = 3;
const LANES: u32 = 1;

/// What can go wrong opening or making an account.
#[derive(Debug)]
pub enum AccountError {
    /// No account has been made on this installation yet.
    None,
    /// One already exists, and making a second would strand the first's seeds.
    Exists,
    /// The password did not open it.
    WrongPassword,
    /// The account file is not readable as one.
    Malformed(String),
    /// The key could not be derived.
    Kdf(String),
    /// The disk refused.
    Io(std::io::Error),
}

impl std::fmt::Display for AccountError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::None => write!(f, "this installation has no account yet"),
            Self::Exists => write!(f, "this installation already has an account"),
            Self::WrongPassword => write!(f, "that password does not unlock this installation"),
            Self::Malformed(what) => write!(f, "the account file is unreadable: {what}"),
            Self::Kdf(what) => write!(f, "could not derive a key from that password: {what}"),
            Self::Io(err) => write!(f, "{err}"),
        }
    }
}

impl std::error::Error for AccountError {}

impl From<std::io::Error> for AccountError {
    fn from(err: std::io::Error) -> Self {
        Self::Io(err)
    }
}

/// An unlocked account: the key that opens this installation's seeds.
///
/// Deliberately holds no password and no username. It is the derived key and
/// nothing else, so nothing downstream can be tempted to re-derive, log, or
/// compare the secret a person typed.
#[derive(Clone)]
pub struct Account {
    key: [u8; 32],
}

impl std::fmt::Debug for Account {
    /// Never prints the key.
    ///
    /// A `derive(Debug)` here would put the thing that opens every identity on
    /// this disk into any log line that formatted a struct containing one.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Account(unlocked)")
    }
}

impl Account {
    /// Where the account lives, given a workspace.
    pub fn path(workspace: &Path) -> PathBuf {
        workspace.join("account")
    }

    /// Whether this installation has an account yet.
    pub fn exists(workspace: &Path) -> bool {
        Self::path(workspace).is_file()
    }

    /// Makes one, and refuses if there already is one.
    ///
    /// Refusing matters: a second account would derive a different key, so the
    /// seeds wrapped under the first would stop opening and every identity on
    /// this disk would be gone with no error that said so.
    pub fn create(workspace: &Path, username: &str, password: &str) -> Result<Self, AccountError> {
        Self::create_with(workspace, username, password, MEMORY_KIB, PASSES, LANES)
    }

    /// Makes one at chosen cost.
    ///
    /// **For the harness, and it is honest rather than a shortcut.** The
    /// parameters live in the account file and a reader uses what the file says,
    /// so an account made cheaply is opened cheaply and one made at full cost is
    /// opened at full cost — the code path is identical and is the one shipped.
    ///
    /// It exists because the cost is deliberate: 64 MiB and three passes is what
    /// makes a guessing rig expensive, and it is also 64 MiB and three passes
    /// *per process*, paid by a daemon suite that starts a great many of them. A
    /// harness account protects nothing, so paying to protect it buys nothing
    /// and risks turning a known-fragile suite (O20) fragile for a new reason.
    pub fn create_with(
        workspace: &Path,
        username: &str,
        password: &str,
        memory: u32,
        passes: u32,
        lanes: u32,
    ) -> Result<Self, AccountError> {
        if Self::exists(workspace) {
            return Err(AccountError::Exists);
        }
        let mut salt = [0u8; 16];
        intranet_crypto::random_bytes(&mut salt)
            .map_err(|err| AccountError::Kdf(format!("{err:?}")))?;

        let key = derive(password, &salt, memory, passes, lanes)?;
        let account = Self { key };

        let mut raw = Vec::new();
        raw.push(VERSION);
        raw.extend_from_slice(&memory.to_be_bytes());
        raw.extend_from_slice(&passes.to_be_bytes());
        raw.extend_from_slice(&lanes.to_be_bytes());
        raw.extend_from_slice(&salt);
        // **A check value, not the password and not the key.** Unlocking has to
        // be able to say *wrong password* rather than handing back a key that
        // silently opens nothing — which would present as every network on the
        // disk being corrupt.
        raw.extend_from_slice(account.check().as_bytes());
        let name = username.as_bytes();
        raw.extend_from_slice(&(name.len() as u32).to_be_bytes());
        raw.extend_from_slice(name);

        fs::create_dir_all(workspace)?;
        secret::write_private(&Self::path(workspace), &raw)?;
        Ok(account)
    }

    /// Opens it with a password.
    pub fn unlock(workspace: &Path, password: &str) -> Result<Self, AccountError> {
        let raw = match fs::read(Self::path(workspace)) {
            Ok(raw) => raw,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                return Err(AccountError::None);
            }
            Err(err) => return Err(err.into()),
        };
        let stored = Stored::parse(&raw)?;
        let key = derive(password, &stored.salt, stored.memory, stored.passes, stored.lanes)?;
        let account = Self { key };
        if account.check().as_bytes() != &stored.check {
            return Err(AccountError::WrongPassword);
        }
        Ok(account)
    }

    /// The username this installation was set up with.
    pub fn username(workspace: &Path) -> Option<String> {
        let raw = fs::read(Self::path(workspace)).ok()?;
        let stored = Stored::parse(&raw).ok()?;
        String::from_utf8(stored.username).ok()
    }

    /// The key one network's seed is wrapped under.
    ///
    /// Separated per network, so the same ciphertext key never protects two of
    /// them. That buys little against somebody holding the password and is free,
    /// and it keeps the shape honest: `design/00` D28's whole point is that no
    /// single object should tie a member's networks together, and one wrapping
    /// key over all of them is such an object even when the account above it is.
    fn wrapping(&self, network: &intranet_identity::NetworkId) -> Dek {
        let mut context = WRAP_DOMAIN.as_bytes().to_vec();
        context.extend_from_slice(network.as_bytes());
        Dek::from_bytes(*intranet_crypto::keyed_hash(&self.key, &context).as_bytes())
    }

    /// Wraps a network's seed for storage.
    pub fn wrap_seed(&self, network: &intranet_identity::NetworkId, seed: &[u8; 32]) -> Vec<u8> {
        self.wrapping(network).seal_chunk(seed)
    }

    /// Opens a wrapped seed.
    pub fn open_seed(
        &self,
        network: &intranet_identity::NetworkId,
        wrapped: &[u8],
    ) -> Result<[u8; 32], AccountError> {
        let raw = self
            .wrapping(network)
            .open_chunk(wrapped)
            .map_err(|_| AccountError::WrongPassword)?;
        <[u8; 32]>::try_from(raw.as_slice())
            .map_err(|_| AccountError::Malformed("a seed is not 32 bytes".to_owned()))
    }

    /// The value stored to recognise the right password.
    ///
    /// Derived from the account key under its own domain, so it cannot be used
    /// to open anything: knowing it tells an attacker whether a guess is right
    /// only if they already have the file, which is the same position they were
    /// in before.
    fn check(&self) -> intranet_crypto::Hash {
        intranet_crypto::keyed_hash(&self.key, CHECK_DOMAIN.as_bytes())
    }
}

/// The account file's fields.
struct Stored {
    memory: u32,
    passes: u32,
    lanes: u32,
    salt: [u8; 16],
    check: [u8; 32],
    username: Vec<u8>,
}

impl Stored {
    fn parse(raw: &[u8]) -> Result<Self, AccountError> {
        let bad = |what: &str| AccountError::Malformed(what.to_owned());
        if raw.first() != Some(&VERSION) {
            return Err(bad("written by a different version"));
        }
        if raw.len() < 1 + 12 + 16 + 32 + 4 {
            return Err(bad("too short"));
        }
        let u32_at = |at: usize| {
            u32::from_be_bytes([raw[at], raw[at + 1], raw[at + 2], raw[at + 3]])
        };
        let memory = u32_at(1);
        let passes = u32_at(5);
        let lanes = u32_at(9);
        let salt: [u8; 16] = raw[13..29].try_into().map_err(|_| bad("salt"))?;
        let check: [u8; 32] = raw[29..61].try_into().map_err(|_| bad("check value"))?;
        let name_len = u32_at(61) as usize;
        let username = raw
            .get(65..65 + name_len)
            .ok_or_else(|| bad("username runs past the end"))?
            .to_vec();
        Ok(Self {
            memory,
            passes,
            lanes,
            salt,
            check,
            username,
        })
    }
}

/// Turns a password into a key.
fn derive(
    password: &str,
    salt: &[u8; 16],
    memory: u32,
    passes: u32,
    lanes: u32,
) -> Result<[u8; 32], AccountError> {
    let params = argon2::Params::new(memory, passes, lanes, Some(32))
        .map_err(|err| AccountError::Kdf(err.to_string()))?;
    let argon = argon2::Argon2::new(
        argon2::Algorithm::Argon2id,
        argon2::Version::V0x13,
        params,
    );
    let mut key = [0u8; 32];
    argon
        .hash_password_into(password.as_bytes(), salt, &mut key)
        .map_err(|err| AccountError::Kdf(err.to_string()))?;
    Ok(key)
}

/// The account this process has unlocked, if any.
///
/// # Why a process-wide slot rather than a threaded argument
///
/// Every path that opens a store would otherwise carry a key through it, and
/// there are a great many of them — the executor, the daemon, the workspace,
/// every test. Threading it is the cleaner layering and was weighed against this
/// one; what decided it is that the key is a property of *the running process*
/// rather than of any call, and that a slot has exactly one place to look at
/// when asking whether this process is unlocked.
///
/// It holds derived keys and never a password.
///
/// **Keyed by workspace, and that is a correctness point rather than tidiness.**
/// Each installation has its own salt, so the key derived from one password at
/// one workspace does not open another's seeds. A single slot was wrong for
/// exactly that reason: it handed the first workspace's key to the second, which
/// presented as *that password does not unlock this installation* against a
/// password that was right. One installation per machine hides this; a process
/// that touches two does not.
static UNLOCKED: std::sync::RwLock<
    Option<std::collections::BTreeMap<PathBuf, Account>>,
> = std::sync::RwLock::new(None);

/// The environment variable the terminal and the test suite unlock from.
///
/// **There is deliberately no unwrapped path**, not even for tests: a code path
/// that read a seed without a secret would be a second security posture, and
/// D30 already holds that the terminal is a test harness rather than a second
/// interface. So the harness unlocks exactly as the window does and differs only
/// in where the password comes from.
pub const PASSWORD_VAR: &str = "KOLS_PASSWORD";

/// Unlocks this process with a password.
pub fn unlock_process(workspace: &Path, password: &str) -> Result<Account, AccountError> {
    let account = Account::unlock(workspace, password)?;
    remember(workspace, account.clone());
    Ok(account)
}

/// Makes the account and leaves this process unlocked with it.
pub fn create_process(
    workspace: &Path,
    username: &str,
    password: &str,
) -> Result<Account, AccountError> {
    let account = Account::create(workspace, username, password)?;
    remember(workspace, account.clone());
    Ok(account)
}

/// Locks this process again.
///
/// What logging out does to key material. It does **not** stop the node
/// (`design/02` §6.3): *nobody at my keyboard can act as me* and *I want to
/// disappear from the network* are different requests, and only the first is
/// being made.
pub fn lock_process() {
    *UNLOCKED
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
}

fn remember(workspace: &Path, account: Account) {
    UNLOCKED
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get_or_insert_with(Default::default)
        .insert(workspace.to_path_buf(), account);
}

fn recall(workspace: &Path) -> Option<Account> {
    UNLOCKED
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .as_ref()?
        .get(workspace)
        .cloned()
}

/// Whether this process has unlocked a given installation.
pub fn is_unlocked(workspace: &Path) -> bool {
    recall(workspace).is_some()
}

/// The account a store should open its seed with.
///
/// The slot first; failing that, the environment, which is how the terminal and
/// the test suite unlock. An installation with neither is *locked*, and saying so
/// is the point — every other answer would be a way to read a seed without the
/// secret that wraps it.
pub fn for_workspace(workspace: &Path) -> Result<Account, AccountError> {
    if let Some(account) = recall(workspace) {
        return Ok(account);
    }
    let Ok(password) = std::env::var(PASSWORD_VAR) else {
        return Err(AccountError::None);
    };
    // **Provisioned rather than merely opened**, because a harness starts from
    // an empty directory far more often than a person does. This happens only
    // when the variable is set, so a shipped client never reaches it — and
    // anybody who can set this process's environment can already run code as its
    // owner, so it gives away nothing that was being protected.
    let account = if Account::exists(workspace) {
        Account::unlock(workspace, &password)?
    } else {
        // Cheap on purpose — see `create_with`.
        Account::create_with(workspace, "harness", &password, 8 * 1024, 1, 1)?
    };
    remember(workspace, account.clone());
    Ok(account)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Dir(PathBuf);

    impl Dir {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir()
                .join(format!("kols-account-{name}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(&path).expect("a workspace");
            Self(path)
        }
    }

    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn network(n: u8) -> intranet_identity::NetworkId {
        intranet_identity::NetworkId::from_bytes([n; 32])
    }

    #[test]
    fn the_right_password_opens_it_and_a_wrong_one_says_so() {
        let dir = Dir::new("open");
        Account::create(&dir.0, "corey", "correct horse").expect("creates");

        assert!(Account::unlock(&dir.0, "correct horse").is_ok());
        // Told apart from a broken file, because "wrong password" is actionable
        // and "corrupt" is not.
        assert!(matches!(
            Account::unlock(&dir.0, "wrong horse"),
            Err(AccountError::WrongPassword)
        ));
    }

    #[test]
    fn a_seed_survives_a_round_trip_and_only_under_the_right_password() {
        let dir = Dir::new("seed");
        let account = Account::create(&dir.0, "corey", "pw").expect("creates");
        let seed = [7u8; 32];

        let wrapped = account.wrap_seed(&network(1), &seed);
        assert_ne!(&wrapped[..], &seed[..], "the seed must not be stored as itself");
        assert_eq!(account.open_seed(&network(1), &wrapped).expect("opens"), seed);
    }

    #[test]
    fn one_networks_wrapping_does_not_open_another() {
        // D28's argument one level down: no single object ties a member's
        // networks together, so the wrapping key is separated per network even
        // though the account above it is shared.
        let dir = Dir::new("apart");
        let account = Account::create(&dir.0, "corey", "pw").expect("creates");
        let wrapped = account.wrap_seed(&network(1), &[3u8; 32]);
        assert!(account.open_seed(&network(2), &wrapped).is_err());
    }

    #[test]
    fn a_second_account_is_refused_rather_than_stranding_the_first() {
        // The failure this prevents has no symptom: a second account derives a
        // different key, so every seed wrapped under the first stops opening and
        // nothing says why.
        let dir = Dir::new("second");
        Account::create(&dir.0, "corey", "pw").expect("creates");
        assert!(matches!(
            Account::create(&dir.0, "somebody", "else"),
            Err(AccountError::Exists)
        ));
    }

    #[test]
    fn an_installation_with_no_account_says_so_rather_than_failing_obscurely() {
        let dir = Dir::new("none");
        assert!(matches!(Account::unlock(&dir.0, "pw"), Err(AccountError::None)));
        assert!(!Account::exists(&dir.0));
    }

    #[test]
    fn the_username_reads_back_without_the_password() {
        // The window needs a name to greet somebody with before they have typed
        // anything, and a username is not a secret.
        let dir = Dir::new("name");
        Account::create(&dir.0, "corey", "pw").expect("creates");
        assert_eq!(Account::username(&dir.0).as_deref(), Some("corey"));
    }

    #[test]
    fn the_stored_parameters_are_used_rather_than_this_builds_preferences() {
        // Raising the cost later must not make existing accounts unopenable, so
        // a reader uses what the file says. Asserted by unlocking a file written
        // with deliberately different parameters.
        let dir = Dir::new("params");
        let salt = [9u8; 16];
        let weak = derive("pw", &salt, 8 * 1024, 1, 1).expect("derives");
        let account = Account { key: weak };

        let mut raw = vec![VERSION];
        raw.extend_from_slice(&(8u32 * 1024).to_be_bytes());
        raw.extend_from_slice(&1u32.to_be_bytes());
        raw.extend_from_slice(&1u32.to_be_bytes());
        raw.extend_from_slice(&salt);
        raw.extend_from_slice(account.check().as_bytes());
        raw.extend_from_slice(&0u32.to_be_bytes());
        secret::write_private(&Account::path(&dir.0), &raw).expect("writes");

        assert!(
            Account::unlock(&dir.0, "pw").is_ok(),
            "a file written under weaker parameters still has to open"
        );
    }

    #[test]
    fn the_debug_form_never_carries_the_key() {
        let dir = Dir::new("debug");
        let account = Account::create(&dir.0, "corey", "pw").expect("creates");
        let shown = format!("{account:?}");
        assert!(!shown.contains(&intranet_crypto::to_hex(&account.key)));
        assert_eq!(shown, "Account(unlocked)");
    }
}
