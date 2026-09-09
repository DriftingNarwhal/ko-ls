//! The export that makes an identity survive its machine — `design/02` §6.3.
//!
//! **Portability, not recovery.** There is no organisation and no reset; what
//! this is for is that the seed *is* the member and lives on exactly one disk.
//! The claim these tests exist to hold is that a member restoring on a new
//! machine comes back as **the same member** rather than as a stranger the
//! governance log has never seen.

mod common;

use kols_node::bundle::{self, Entry};
use kols_node::workspace::Workspace;

struct Dir(std::path::PathBuf);

impl Dir {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "kols-bundle-{name}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("a workspace");
        Self(path)
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn entry(n: u8, label: &str) -> Entry {
    Entry {
        network: intranet_identity::NetworkId::from_bytes([n; 32]),
        seed: [n.wrapping_add(100); 32],
        label: label.to_owned(),
        relays: vec![format!("/dns4/relay{n}.example/tcp/443")],
    }
}

#[test]
fn a_bundle_round_trips_under_its_passphrase() {
    let entries = vec![entry(1, "the workshop"), entry(2, "a conversation")];
    let sealed = bundle::seal(&entries, "correct horse battery").expect("seals");
    assert_eq!(
        bundle::open(&sealed, "correct horse battery").expect("opens"),
        entries
    );
}

#[test]
fn the_seeds_are_not_in_the_file() {
    // The point of sealing it. A bundle sits in a cloud drive for years, and one
    // that carried its seeds in the clear would be every identity its holder has
    // — which is the problem the keyring exists to fix, relocated.
    let entries = vec![entry(3, "private")];
    let sealed = bundle::seal(&entries, "pw").expect("seals");
    let needle = entries[0].seed;
    assert!(
        !sealed.windows(needle.len()).any(|w| w == needle),
        "a seed appears verbatim in the exported file"
    );
}

#[test]
fn a_wrong_passphrase_is_refused_and_a_wrong_file_says_which() {
    // Two different mistakes needing two different answers. Somebody who picked
    // the wrong file should be told that rather than sent looking for a
    // passphrase that was never going to work.
    let sealed = bundle::seal(&[entry(4, "x")], "right").expect("seals");
    assert!(matches!(
        bundle::open(&sealed, "wrong"),
        Err(bundle::BundleError::WrongPassphrase)
    ));
    assert!(matches!(
        bundle::open(b"not a bundle at all, just some bytes", "right"),
        Err(bundle::BundleError::NotABundle)
    ));
}

#[test]
fn an_empty_bundle_is_a_bundle() {
    // An installation with no networks exports something openable rather than
    // failing. Exporting before joining anything is an odd thing to do and is
    // not an error.
    let sealed = bundle::seal(&[], "pw").expect("seals");
    assert!(bundle::open(&sealed, "pw").expect("opens").is_empty());
}

#[test]
fn restoring_brings_back_the_same_member_and_not_a_stranger() {
    // **The claim the whole feature rests on.** A restored seed must derive the
    // identity the governance log already names; a fresh one would be somebody
    // new, who would have to be admitted again and whose old messages would
    // stay authored by a person they can no longer act as.
    let first = Dir::new("origin");
    let workspace = Workspace::at(first.0.clone());
    let store = workspace.create("the workshop", vec![]).expect("creates");
    let was = store.identity().expect("an identity").id();
    let network = *store.network();
    drop(store);

    let entries = workspace.to_bundle().expect("exports");
    assert_eq!(entries.len(), 1, "one network, one entry");
    let sealed = bundle::seal(&entries, "paper passphrase").expect("seals");

    // A different machine: a different workspace, a different account.
    let second = Dir::new("elsewhere");
    let restored_ws = Workspace::at(second.0.clone());
    let opened = bundle::open(&sealed, "paper passphrase").expect("opens");
    let outcome = restored_ws.from_bundle(&opened).expect("restores");
    assert_eq!(outcome.added.len(), 1, "{outcome:?}");

    let back = kols_node::store::Store::open(restored_ws.path_for(&network)).expect("opens");
    assert_eq!(
        back.identity().expect("an identity").id(),
        was,
        "a restored member has to be the same member"
    );
    assert_eq!(back.label().as_deref(), Some("the workshop"));
}

#[test]
fn a_network_already_here_is_skipped_rather_than_overwritten() {
    // Same reasoning as a second `init` refusing: the seed at that path cannot
    // be recovered if it is lost. Skipping can leave somebody as the wrong
    // member, which is visible and fixable; overwriting is neither.
    let dir = Dir::new("collide");
    let workspace = Workspace::at(dir.0.clone());
    let store = workspace.create("mine", vec![]).expect("creates");
    let network = *store.network();
    let mine = store.identity().expect("an identity").id();
    drop(store);

    let intruder = Entry {
        network,
        seed: [42u8; 32],
        label: "somebody else".to_owned(),
        relays: vec![],
    };
    let outcome = workspace.from_bundle(&[intruder]).expect("restores");
    assert_eq!(outcome.skipped.len(), 1, "{outcome:?}");
    assert!(outcome.added.is_empty());

    let still = kols_node::store::Store::open(workspace.path_for(&network)).expect("opens");
    assert_eq!(
        still.identity().expect("an identity").id(),
        mine,
        "the seed that was already there must be untouched"
    );
}

#[test]
fn a_relay_travels_with_the_network_it_belongs_to() {
    // Without one there is nobody to sync from: a restored store holds an
    // identity and an empty log, and the history comes back off the network.
    let dir = Dir::new("relays");
    let workspace = Workspace::at(dir.0.clone());
    let relay = "/dns4/relay.example/tcp/443/p2p/12D3KooWfake".to_owned();
    let store = workspace
        .create("with a relay", vec![relay.clone()])
        .expect("creates");
    drop(store);

    let entries = workspace.to_bundle().expect("exports");
    assert_eq!(entries[0].relays, vec![relay]);
}
