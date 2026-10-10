//! A presence beat is accepted once — spec 07 §6.4.
//!
//! The beat frame is unsigned and its seal deterministic, so anybody on the
//! topic can republish a beat byte for byte. Freshness is judged by when a
//! beat was *heard*, so without this each copy renewed its sender for another
//! freshness window, and replaying an older one restored a state they had left.
//! What tells a copy apart is the sender's own reading, signed inside the beat:
//! their real beats carry ever-later ones.

use kols_core::Beat;
use kols_node::workspace::Workspace;

struct Dir(std::path::PathBuf);

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn store(name: &str) -> (Dir, kols_node::store::Store) {
    let path = std::env::temp_dir().join(format!("kols-beat-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&path);
    std::fs::create_dir_all(&path).expect("a workspace");
    let dir = Dir(path);
    let store = Workspace::at(dir.0.clone())
        .create("beats", Vec::new())
        .expect("creates");
    (dir, store)
}

const ALICE: &str = "a1";

fn heard(store: &kols_node::store::Store) -> Vec<(Beat, i64)> {
    store
        .beats()
        .into_iter()
        .filter(|(who, ..)| who == ALICE)
        .map(|(_, state, at)| (state, at))
        .collect()
}

#[test]
fn a_beat_heard_again_renews_nothing() {
    let (_dir, store) = store("again");
    assert!(store.record_beat(ALICE, Beat::Here, 1_000, 500).unwrap());
    // The same beat, a minute later: a replay, which must not move the heard
    // time that freshness is judged by.
    assert!(!store.record_beat(ALICE, Beat::Here, 61_000, 500).unwrap());
    assert_eq!(heard(&store), vec![(Beat::Here, 1_000)]);
}

#[test]
fn an_older_beat_cannot_restore_a_state_its_sender_left() {
    let (_dir, store) = store("older");
    assert!(store.record_beat(ALICE, Beat::Busy, 1_000, 500).unwrap());
    assert!(store.record_beat(ALICE, Beat::Here, 31_000, 530).unwrap());
    assert!(!store.record_beat(ALICE, Beat::Busy, 40_000, 500).unwrap());
    assert_eq!(heard(&store), vec![(Beat::Here, 31_000)]);
}

#[test]
fn a_restart_does_not_reopen_the_replay() {
    // A daemon forgets what it heard when it starts (presence is ephemeral),
    // and if it forgot the floor with it, every restart would accept one more
    // copy of each member's last beat.
    let (_dir, store) = store("restart");
    assert!(store.record_beat(ALICE, Beat::Here, 1_000, 500).unwrap());
    store.forget_beats().unwrap();
    assert!(heard(&store).is_empty(), "what was heard is forgotten");
    assert!(!store.record_beat(ALICE, Beat::Here, 90_000, 500).unwrap());
    assert!(
        heard(&store).is_empty(),
        "and the copy does not bring it back"
    );
    // The sender's next real beat is newer, and is heard as usual.
    assert!(store.record_beat(ALICE, Beat::Here, 91_000, 531).unwrap());
    assert_eq!(heard(&store), vec![(Beat::Here, 91_000)]);
}
