//! Unread counts, per network and channel, kept by the shell — `design/09` §1.3, §4.3.
//!
//! # Why the shell keeps them and not a window
//!
//! They were kept by the network window, in its own storage, from the records
//! events it received. That was right while one node ran, for whichever network
//! was in view. Once every network ran a node (§2), most of what arrived was for
//! networks no window was showing, and the network window threw it away as not
//! its own — so a message in a network you were not looking at was counted by
//! nobody, and opening that network later showed nothing unread. The events
//! reach the shell whichever window is open, so the shell is the one place that
//! sees all of them.
//!
//! # What counts
//!
//! A message, one per record, arriving **live** or from the **head** of an
//! author's log — the conversation as it happens, and the backlog a node learns
//! on its next start (§4.3). Never one arriving by **backfill**: that is history
//! being recovered, and a message from last year is not news (`kols-api`'s
//! `Arrival`). Never one in a channel a window is showing, because somebody is
//! looking at it.
//!
//! # Local, and only ever local
//!
//! What somebody has read is not a fact about the network, and nothing here goes
//! anywhere but this machine's store: one small file per network, written
//! atomically like everything else the store keeps.

use crate::store::StoreError;
use intranet_identity::NetworkId;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The file a network's counts live in, inside its store.
const FILE: &str = "unread";

/// Unread counts for every network this shell knows about.
///
/// Channels are keyed by their hex id, which is the form every consumer already
/// holds, so nothing here has to parse one back.
#[derive(Debug, Default)]
pub struct Unread {
    counts: BTreeMap<NetworkId, BTreeMap<String, u32>>,
    roots: BTreeMap<NetworkId, PathBuf>,
    /// What each window is showing, by window label: one channel, or a whole
    /// network where the window shows it as one (a conversation). A channel
    /// somebody is looking at gains no count.
    viewing: BTreeMap<String, (NetworkId, Option<String>)>,
}

impl Unread {
    /// Starts tracking `network`, whose store is at `root`, loading what that
    /// store kept. Tracking one already tracked changes nothing, so this is safe
    /// to call on every reconcile.
    pub fn track(&mut self, network: NetworkId, root: &Path) {
        if self.roots.contains_key(&network) {
            return;
        }
        self.counts.insert(network, load(root));
        self.roots.insert(network, root.to_path_buf());
    }

    /// Stops tracking a network that has been forgotten. Its file went with its
    /// store.
    pub fn forget(&mut self, network: &NetworkId) {
        self.counts.remove(network);
        self.roots.remove(network);
        self.viewing.retain(|_, (shown, _)| shown != network);
    }

    /// Counts `messages` that arrived in `channel`, and says whether anything
    /// changed. Nothing changes for a channel a window is showing, or for a
    /// network not tracked.
    ///
    /// The caller decides which arrivals count (see the module docs); this only
    /// adds.
    pub fn arrived(&mut self, network: &NetworkId, channel: &str, messages: u32) -> bool {
        if messages == 0 || self.is_viewed(network, channel) {
            return false;
        }
        let Some(counts) = self.counts.get_mut(network) else {
            return false;
        };
        let count = counts.entry(channel.to_owned()).or_default();
        *count = count.saturating_add(messages);
        self.persist(network);
        true
    }

    /// Records that `window` is showing `channel` of `network` — every channel
    /// of it, for `None` — or nothing, and clears what it shows. Says whether a
    /// count changed.
    pub fn viewing(&mut self, window: &str, shown: Option<(NetworkId, Option<String>)>) -> bool {
        let Some((network, channel)) = shown else {
            self.viewing.remove(window);
            return false;
        };
        let cleared = self.counts.get_mut(&network).is_some_and(|counts| {
            let before = counts.len();
            match &channel {
                Some(channel) => {
                    counts.remove(channel);
                }
                None => counts.clear(),
            }
            counts.len() != before
        });
        if cleared {
            self.persist(&network);
        }
        self.viewing.insert(window.to_owned(), (network, channel));
        cleared
    }

    /// Stops treating anything `window` showed as being looked at, because it
    /// closed or hid.
    pub fn window_gone(&mut self, window: &str) {
        self.viewing.remove(window);
    }

    /// Per-channel counts for one network, leaving out channels with none.
    pub fn channels(&self, network: &NetworkId) -> BTreeMap<String, u32> {
        self.counts.get(network).cloned().unwrap_or_default()
    }

    /// How many unread messages one network holds across every channel.
    pub fn total(&self, network: &NetworkId) -> u32 {
        self.counts
            .get(network)
            .map(|counts| counts.values().fold(0u32, |sum, n| sum.saturating_add(*n)))
            .unwrap_or(0)
    }

    /// How many unread messages every network holds together.
    pub fn grand_total(&self) -> u32 {
        self.counts
            .keys()
            .fold(0u32, |sum, network| sum.saturating_add(self.total(network)))
    }

    fn is_viewed(&self, network: &NetworkId, channel: &str) -> bool {
        self.viewing.values().any(|(shown, open)| {
            shown == network && open.as_deref().is_none_or(|open| open == channel)
        })
    }

    /// Writes one network's counts back to its store.
    ///
    /// A failure is swallowed, because the count in memory is still right for
    /// this session and the cost of losing it is a stale badge after a restart —
    /// which is not worth stopping anything over.
    fn persist(&self, network: &NetworkId) {
        let (Some(root), Some(counts)) = (self.roots.get(network), self.counts.get(network)) else {
            return;
        };
        let _ = save(root, counts);
    }
}

/// Counts what a records event contributes: one per message, and only for an
/// arrival that is news (see the module docs).
pub fn news(records: &[kols_core::Record], arrival: &kols_api::Arrival) -> u32 {
    if matches!(arrival, kols_api::Arrival::Backfill { .. }) {
        return 0;
    }
    records
        .iter()
        .filter(|record| matches!(record.body, kols_core::RecordBody::Message { .. }))
        .count()
        .try_into()
        .unwrap_or(u32::MAX)
}

/// Reads a store's counts. Anything unreadable is treated as nothing unread,
/// since the alternative is a badge built from a file that does not parse.
fn load(root: &Path) -> BTreeMap<String, u32> {
    std::fs::read_to_string(root.join(FILE))
        .map(|text| {
            text.lines()
                .filter_map(|line| {
                    let (channel, count) = line.trim().split_once(' ')?;
                    let count: u32 = count.parse().ok()?;
                    (count > 0).then(|| (channel.to_owned(), count))
                })
                .collect()
        })
        .unwrap_or_default()
}

fn save(root: &Path, counts: &BTreeMap<String, u32>) -> Result<(), StoreError> {
    let text: String = counts
        .iter()
        .filter(|(_, count)| **count > 0)
        .map(|(channel, count)| format!("{channel} {count}\n"))
        .collect();
    crate::store::write_atomically(root, root.join(FILE), text.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn network(n: u8) -> NetworkId {
        NetworkId::from_bytes([n; 32])
    }

    /// A scratch store root, removed on drop, in the shape the other tests here
    /// use.
    struct Dir(PathBuf);

    impl Dir {
        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn root(name: &str) -> Dir {
        let path = std::env::temp_dir().join(format!("kols-unread-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("a store root");
        Dir(path)
    }

    #[test]
    fn a_network_nobody_is_looking_at_still_counts() {
        // The defect this module exists for: a message in a network no window
        // shows was counted by nobody.
        let dir = root("nobody");
        let mut unread = Unread::default();
        unread.track(network(1), dir.path());
        unread.track(network(2), dir.path());
        unread.viewing("network", Some((network(1), Some("aa".into()))));

        assert!(unread.arrived(&network(2), "bb", 2));
        assert_eq!(unread.total(&network(2)), 2);
        assert_eq!(unread.grand_total(), 2);
    }

    #[test]
    fn a_channel_somebody_is_looking_at_gains_nothing() {
        let dir = root("looking");
        let mut unread = Unread::default();
        unread.track(network(1), dir.path());
        unread.viewing("network", Some((network(1), Some("aa".into()))));
        assert!(!unread.arrived(&network(1), "aa", 3));
        assert!(unread.arrived(&network(1), "bb", 1), "another channel does");
        unread.window_gone("network");
        assert!(
            unread.arrived(&network(1), "aa", 1),
            "and so does it once nobody is"
        );
        assert_eq!(unread.total(&network(1)), 2);
    }

    #[test]
    fn opening_a_channel_reads_it() {
        let dir = root("reads");
        let mut unread = Unread::default();
        unread.track(network(1), dir.path());
        unread.arrived(&network(1), "aa", 4);
        unread.arrived(&network(1), "bb", 1);
        assert!(unread.viewing("network", Some((network(1), Some("aa".into())))));
        assert_eq!(unread.channels(&network(1)), [("bb".to_owned(), 1)].into());
        assert!(
            !unread.viewing("network", Some((network(1), Some("aa".into())))),
            "nothing left to clear"
        );
    }

    #[test]
    fn a_window_showing_a_whole_network_reads_all_of_it() {
        // A conversation window shows its network's one channel without naming
        // it, so it views the network.
        let dir = root("whole");
        let mut unread = Unread::default();
        unread.track(network(1), dir.path());
        unread.arrived(&network(1), "aa", 2);
        assert!(unread.viewing("conversation-x", Some((network(1), None))));
        assert_eq!(unread.total(&network(1)), 0);
        assert!(
            !unread.arrived(&network(1), "aa", 1),
            "and it stays read while shown"
        );
    }

    #[test]
    fn counts_survive_a_restart() {
        // §4.3: unread survives the application closing. The node learns the
        // backlog on its next start; what was already counted must not be lost.
        let dir = root("restart");
        let mut unread = Unread::default();
        unread.track(network(1), dir.path());
        unread.arrived(&network(1), "aa", 3);
        drop(unread);

        let mut again = Unread::default();
        again.track(network(1), dir.path());
        assert_eq!(again.total(&network(1)), 3);
    }

    #[test]
    fn a_forgotten_network_holds_nothing() {
        let dir = root("forgotten");
        let mut unread = Unread::default();
        unread.track(network(1), dir.path());
        unread.arrived(&network(1), "aa", 3);
        unread.viewing("network", Some((network(1), Some("bb".into()))));
        unread.forget(&network(1));
        assert_eq!(unread.grand_total(), 0);
        assert!(!unread.arrived(&network(1), "aa", 1), "untracked");
    }

    #[test]
    fn history_being_recovered_is_not_news() {
        assert_eq!(news(&[], &kols_api::Arrival::Backfill { segments: 3 }), 0);
    }
}
