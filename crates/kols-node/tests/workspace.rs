//! A directory of networks — `design/09` §1.
//!
//! Creating a network moved out of `kols init` so that a window could do it
//! without a second copy of the genesis requirements, each of which is silent
//! when missed. These cover the moved path directly, because the interface that
//! calls it cannot be driven from a test.

use kols_node::workspace::Workspace;

struct Dir(std::path::PathBuf);

impl Dir {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!("kols-ws-{name}-{}", std::process::id()));
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

#[test]
fn an_empty_workspace_holds_nothing_and_is_not_an_error() {
    // Where somebody starts. The window shows a picker rather than a failure.
    let dir = Dir::new("empty");
    assert!(Workspace::at(dir.0.clone()).list().is_empty());
}

#[test]
fn a_created_network_is_replayable_and_its_founder_is_a_member() {
    // The check `kols init` has always made and the reason creation is one path:
    // a genesis that replays but grants nothing looks like success until the
    // first post is refused by its own author's node.
    let dir = Dir::new("create");
    let workspace = Workspace::at(dir.0.clone());

    let store = workspace
        .create("the workshop", vec!["/ip4/198.51.100.7/tcp/4001".to_owned()])
        .expect("creates");
    let identity = store.identity().expect("an identity");
    let state = store.state().expect("replays");

    assert!(state.is_member(&identity.id()));
    assert_eq!(
        state.policy.bootstrap_relays,
        vec!["/ip4/198.51.100.7/tcp/4001".to_owned()]
    );
    // Everything a member needs, registered at genesis — the part that is silent
    // when missed.
    assert!(state.identity_holds(
        &identity.id(),
        &intranet_governance::Capability::extension("chat:post:*".to_owned())
    ));
}

#[test]
fn networks_are_listed_and_reachable_by_the_start_of_their_id() {
    let dir = Dir::new("list");
    let workspace = Workspace::at(dir.0.clone());
    workspace.create("first", Vec::new()).expect("creates");
    workspace.create("second", Vec::new()).expect("creates");

    let listed = workspace.list();
    assert_eq!(listed.len(), 2);
    // Stable order, so a list does not reshuffle between renders.
    assert_eq!(listed[0].label, "first");
    assert_eq!(listed[1].label, "second");

    let opened = workspace.open(&listed[0].id[..8]).expect("opens");
    assert_eq!(opened.network().as_bytes()[..], hex(&listed[0].id)[..]);
}

#[test]
fn an_ambiguous_or_unknown_prefix_is_refused_rather_than_guessed() {
    let dir = Dir::new("ambiguous");
    let workspace = Workspace::at(dir.0.clone());
    workspace.create("one", Vec::new()).expect("creates");
    workspace.create("two", Vec::new()).expect("creates");

    assert!(workspace.open("ffffffffffff").is_err(), "unknown prefix");
    // Every id starts with the empty string, so this is the ambiguous case
    // without needing two ids that happen to share a prefix.
    let complaint = match workspace.open("") {
        Err(why) => why,
        Ok(_) => panic!("an ambiguous prefix opened a network"),
    };
    assert!(complaint.contains("give more"), "{complaint}");
}

#[test]
fn a_home_that_is_itself_one_store_still_reads_as_one_network() {
    // What `kols --home` has always meant, and still does: a terminal is told
    // which network to work with, a window is given the directory holding many.
    // Both shapes have to be legible or the two front ends disagree about what a
    // path means.
    let dir = Dir::new("legacy");
    let inner = dir.0.join("single");
    let workspace = Workspace::at(dir.0.clone());
    workspace
        .create_at(inner.clone(), "alone", Vec::new())
        .expect("creates");

    let direct = Workspace::at(inner);
    let listed = direct.list();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].label, "alone");

    // And creating inside it is refused rather than nesting a network in a
    // network, which nothing would ever find.
    assert!(direct.create("nested", Vec::new()).is_err());
}

fn hex(text: &str) -> Vec<u8> {
    intranet_crypto::from_hex(text).expect("hex")
}
