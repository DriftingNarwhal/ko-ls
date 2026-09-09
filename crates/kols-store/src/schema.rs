//! The projection's tables, and the three questions they exist to answer.

use crate::Verdict;
use intranet_identity::PerNetworkIdentityId;
use kols_core::{ChannelId, Cursor, Hlc, MessageId, Record, RecordClass};
use rusqlite::Connection;

/// What can go wrong reading or writing the projection.
#[derive(Debug)]
pub enum ProjectionError {
    /// SQLite refused.
    Sqlite(rusqlite::Error),
    /// A stored row was not the shape this build expects.
    Corrupt(String),
}

impl std::fmt::Display for ProjectionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Sqlite(err) => write!(f, "the projection refused: {err}"),
            Self::Corrupt(what) => write!(f, "the projection holds something unreadable: {what}"),
        }
    }
}

impl std::error::Error for ProjectionError {}

impl From<rusqlite::Error> for ProjectionError {
    fn from(err: rusqlite::Error) -> Self {
        Self::Sqlite(err)
    }
}

/// The version this build writes.
///
/// **Bumped rather than migrated, deliberately.** A projection holds nothing
/// that is not derivable from the records beside it, so the cheapest correct
/// answer to a schema change is to drop it and fold the records again — and a
/// migration is a second description of the schema that has to stay in step with
/// the first. `design/05` §5 lists migrations among what this crate owns; this
/// is what they turn out to be when nothing here is a source of truth.
const SCHEMA: i64 = 1;

/// An index over the records of one network's channels.
pub struct Projection {
    conn: Connection,
}

impl Projection {
    /// Opens the projection at `path`, creating or rebuilding it as needed.
    ///
    /// Returns whether it came up empty, which is a caller's cue to fold the
    /// records in: this crate deliberately does not read them, because it holds
    /// no opinion about where a record lives.
    pub fn open(path: &std::path::Path) -> Result<(Self, bool), ProjectionError> {
        let conn = Connection::open(path)?;
        Self::prepare(conn)
    }

    /// Opens a projection that lives only as long as this process.
    ///
    /// For tests, and for a caller that wants the index without the file.
    pub fn in_memory() -> Result<(Self, bool), ProjectionError> {
        Self::prepare(Connection::open_in_memory()?)
    }

    fn prepare(conn: Connection) -> Result<(Self, bool), ProjectionError> {
        // Durability is deliberately relaxed: losing this to a power cut costs a
        // rebuild and never a record, because the records are elsewhere and are
        // the truth. The same trade `design/05` §1.1 makes for the store's own
        // writes, and easier to make here, where there is nothing to lose.
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;

        let found: i64 = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;
        if found != SCHEMA {
            // Everything, not only the tables this build knows: a projection
            // from another version may carry tables this one does not, and
            // leaving them would make the file a mixture of two schemas.
            conn.execute_batch("DROP TABLE IF EXISTS records; DROP TABLE IF EXISTS folds;")?;
        }

        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS records (
                 channel     BLOB    NOT NULL,
                 hlc_wall    INTEGER NOT NULL,
                 hlc_counter INTEGER NOT NULL,
                 id          BLOB    NOT NULL,
                 author      BLOB    NOT NULL,
                 class       INTEGER NOT NULL,
                 target      BLOB,
                 verdict     INTEGER NOT NULL,
                 PRIMARY KEY (id)
             );
             -- A page: the records of a channel in merge order. The order is
             -- (wall, counter, id), which is `design/01` §4's, so a page taken
             -- from this index is a page of the same sequence every node
             -- computes.
             CREATE INDEX IF NOT EXISTS records_in_order
                 ON records (channel, hlc_wall, hlc_counter, id);
             -- What acts on a page's messages, which may sit anywhere later.
             CREATE INDEX IF NOT EXISTS records_by_target
                 ON records (channel, target);
             -- The trailing window a verdict is decided against.
             CREATE INDEX IF NOT EXISTS records_by_author
                 ON records (channel, author, class, hlc_wall);
             CREATE TABLE IF NOT EXISTS folds (
                 channel       BLOB    NOT NULL PRIMARY KEY,
                 message_rate  INTEGER NOT NULL,
                 reaction_rate INTEGER NOT NULL,
                 slowmode      INTEGER NOT NULL
             );",
        )?;
        conn.pragma_update(None, "user_version", SCHEMA)?;

        let empty: i64 =
            conn.query_row("SELECT COUNT(*) FROM records", [], |row| row.get(0))?;
        Ok((Self { conn }, empty == 0))
    }

    /// Folds one record in, with the verdict the caller decided.
    ///
    /// The verdict is the caller's because deciding it needs the network's
    /// limits, which are replayed policy and no business of an index.
    pub fn insert(&self, record: &Record, verdict: Verdict) -> Result<(), ProjectionError> {
        self.conn.execute(
            "INSERT OR REPLACE INTO records
                 (channel, hlc_wall, hlc_counter, id, author, class, target, verdict)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            rusqlite::params![
                record.channel.as_bytes().to_vec(),
                record.hlc.wall_millis,
                i64::from(record.hlc.counter),
                record.id().as_bytes().to_vec(),
                record.author.verifying_key().as_bytes().to_vec(),
                class_code(record.body.class()),
                target_of(&record.body).map(|id| id.as_bytes().to_vec()),
                verdict.code(),
            ],
        )?;
        Ok(())
    }

    /// The records of a channel *before* a position, oldest first.
    ///
    /// Oldest first because that is the order a reader renders in, and taking
    /// them from the *end* is what makes a page cheap: the index is walked
    /// backwards from `before` and stopped, rather than read and then trimmed.
    ///
    /// The bound is a [`Cursor`] rather than a reading, and that is a
    /// correctness fix rather than a tidying: two records can share a reading,
    /// so a boundary that named only the reading excluded both of them —
    /// including the one that had never been drawn (`design/09` §4.4).
    pub fn before(
        &self,
        channel: &ChannelId,
        before: Option<Cursor>,
        limit: usize,
    ) -> Result<Vec<MessageId>, ProjectionError> {
        // **Past the newest record rather than at it**, so `None` means "the
        // tail" without a special case in the predicate below.
        let end = before.unwrap_or(Cursor::new(Hlc::new(i64::MAX, u32::MAX), MessageId::from_bytes([0xFF; 32])));
        let mut statement = self.conn.prepare_cached(
            "SELECT id FROM records
              WHERE channel = ?1
                AND (hlc_wall < ?2
                  OR (hlc_wall = ?2 AND hlc_counter < ?3)
                  OR (hlc_wall = ?2 AND hlc_counter = ?3 AND id < ?4))
              ORDER BY hlc_wall DESC, hlc_counter DESC, id DESC
              LIMIT ?5",
        )?;
        let mut ids = Self::collect(&mut statement, channel, end, limit)?;
        // Walked backwards to find them and handed back forwards, which is the
        // order everything above reads in.
        ids.reverse();
        Ok(ids)
    }

    /// The records of a channel *after* a position, oldest first.
    ///
    /// The other half of [`Self::before`], and the two together compose every
    /// shape `design/09` §4.4 asks for: a page back is one, a page around a
    /// cursor is both, and re-reading a loaded range is this one with a
    /// stopping point.
    pub fn after(
        &self,
        channel: &ChannelId,
        after: Option<Cursor>,
        limit: usize,
    ) -> Result<Vec<MessageId>, ProjectionError> {
        // Before the oldest possible record, so `None` means "from the start".
        let start = after.unwrap_or(Cursor::new(Hlc::new(i64::MIN, 0), MessageId::from_bytes([0; 32])));
        let mut statement = self.conn.prepare_cached(
            "SELECT id FROM records
              WHERE channel = ?1
                AND (hlc_wall > ?2
                  OR (hlc_wall = ?2 AND hlc_counter > ?3)
                  OR (hlc_wall = ?2 AND hlc_counter = ?3 AND id > ?4))
              ORDER BY hlc_wall ASC, hlc_counter ASC, id ASC
              LIMIT ?5",
        )?;
        Self::collect(&mut statement, channel, start, limit)
    }

    /// The records from `from` up to and including `to`, oldest first.
    ///
    /// `to` absent means the tail, which is what makes a *live* range live: it
    /// picks up arrivals without anything having to ask for them, and it picks
    /// up backfill landing inside the range, which a tail-only read never would
    /// (`design/09` §4.4).
    pub fn between(
        &self,
        channel: &ChannelId,
        from: Cursor,
        to: Option<Cursor>,
        limit: usize,
    ) -> Result<Vec<MessageId>, ProjectionError> {
        let end = to.unwrap_or(Cursor::new(Hlc::new(i64::MAX, u32::MAX), MessageId::from_bytes([0xFF; 32])));
        let mut statement = self.conn.prepare_cached(
            "SELECT id FROM records
              WHERE channel = ?1
                AND (hlc_wall > ?2
                  OR (hlc_wall = ?2 AND hlc_counter > ?3)
                  OR (hlc_wall = ?2 AND hlc_counter = ?3 AND id >= ?4))
                AND (hlc_wall < ?5
                  OR (hlc_wall = ?5 AND hlc_counter < ?6)
                  OR (hlc_wall = ?5 AND hlc_counter = ?6 AND id <= ?7))
              ORDER BY hlc_wall ASC, hlc_counter ASC, id ASC
              LIMIT ?8",
        )?;
        let rows = statement.query_map(
            rusqlite::params![
                channel.as_bytes().to_vec(),
                from.hlc.wall_millis,
                i64::from(from.hlc.counter),
                from.id.as_bytes().to_vec(),
                end.hlc.wall_millis,
                i64::from(end.hlc.counter),
                end.id.as_bytes().to_vec(),
                limit as i64
            ],
            |row| row.get::<_, Vec<u8>>(0),
        )?;
        let mut ids = Vec::new();
        for row in rows {
            ids.push(message_id(&row?)?);
        }
        Ok(ids)
    }

    /// Runs one of the one-bound walks above.
    fn collect(
        statement: &mut rusqlite::CachedStatement<'_>,
        channel: &ChannelId,
        bound: Cursor,
        limit: usize,
    ) -> Result<Vec<MessageId>, ProjectionError> {
        let rows = statement.query_map(
            rusqlite::params![
                channel.as_bytes().to_vec(),
                bound.hlc.wall_millis,
                i64::from(bound.hlc.counter),
                bound.id.as_bytes().to_vec(),
                limit as i64
            ],
            |row| row.get::<_, Vec<u8>>(0),
        )?;
        let mut ids = Vec::new();
        for row in rows {
            ids.push(message_id(&row?)?);
        }
        Ok(ids)
    }

    /// How many distinct authors have written in a channel.
    ///
    /// Whole-channel rather than per page, and answered from the index rather
    /// than by reading records. `design/09` §4.4: a number that changed as
    /// somebody scrolled would be worse than the query it saved.
    pub fn authors(&self, channel: &ChannelId) -> Result<usize, ProjectionError> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(DISTINCT author) FROM records WHERE channel = ?1",
            rusqlite::params![channel.as_bytes().to_vec()],
            |row| row.get(0),
        )?;
        Ok(count.max(0) as usize)
    }

    /// The ids of records acting on any of `targets`.
    pub fn acting_on(
        &self,
        channel: &ChannelId,
        targets: &[MessageId],
    ) -> Result<Vec<MessageId>, ProjectionError> {
        let mut found = Vec::new();
        let mut statement = self
            .conn
            .prepare_cached("SELECT id FROM records WHERE channel = ?1 AND target = ?2")?;
        for target in targets {
            let rows = statement.query_map(
                rusqlite::params![channel.as_bytes().to_vec(), target.as_bytes().to_vec()],
                |row| row.get::<_, Vec<u8>>(0),
            )?;
            for row in rows {
                found.push(message_id(&row?)?);
            }
        }
        Ok(found)
    }

    /// Whether a record renders, as far as the rate pass is concerned.
    pub fn verdict(&self, id: &MessageId) -> Result<Option<Verdict>, ProjectionError> {
        let mut statement = self
            .conn
            .prepare_cached("SELECT verdict FROM records WHERE id = ?1")?;
        let found = statement
            .query_row(rusqlite::params![id.as_bytes().to_vec()], |row| {
                row.get::<_, i64>(0)
            })
            .map(Verdict::from_code);
        match found {
            Ok(verdict) => Ok(Some(verdict)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(err) => Err(err.into()),
        }
    }

    /// The author's admitted readings of `class` at or after `since`.
    ///
    /// The fold state a verdict is decided against (`design/01` §10.4): the
    /// window is what the ceiling counts, and only admitted records occupy it —
    /// a refused one costs its author nothing beyond itself.
    pub fn admitted_since(
        &self,
        channel: &ChannelId,
        author: &PerNetworkIdentityId,
        class: RecordClass,
        since: i64,
    ) -> Result<Vec<Hlc>, ProjectionError> {
        let mut statement = self.conn.prepare_cached(
            "SELECT hlc_wall, hlc_counter FROM records
              WHERE channel = ?1 AND author = ?2 AND class = ?3
                AND hlc_wall > ?4 AND verdict = 0
              ORDER BY hlc_wall, hlc_counter",
        )?;
        let rows = statement.query_map(
            rusqlite::params![
                channel.as_bytes().to_vec(),
                author.verifying_key().as_bytes().to_vec(),
                class_code(class),
                since,
            ],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
        )?;
        let mut readings = Vec::new();
        for row in rows {
            let (wall, counter) = row?;
            readings.push(Hlc::new(
                wall,
                u32::try_from(counter).map_err(|_| {
                    ProjectionError::Corrupt("a stored counter does not fit".to_owned())
                })?,
            ));
        }
        Ok(readings)
    }

    /// Every record this channel holds, in merge order.
    ///
    /// For the paths that genuinely want all of it — a rebuild, and the fold
    /// that follows an out-of-order arrival.
    pub fn all(&self, channel: &ChannelId) -> Result<Vec<MessageId>, ProjectionError> {
        self.before(channel, None, usize::MAX)
    }

    /// How many of a channel's records are folded in.
    pub fn count(&self, channel: &ChannelId) -> Result<usize, ProjectionError> {
        let found: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM records WHERE channel = ?1",
            rusqlite::params![channel.as_bytes().to_vec()],
            |row| row.get(0),
        )?;
        Ok(usize::try_from(found).unwrap_or(0))
    }

    /// Every record id folded in for a channel.
    ///
    /// For deciding what is missing. Ids rather than rows, because that is the
    /// whole of the comparison and the rows are wider.
    pub fn ids(
        &self,
        channel: &ChannelId,
    ) -> Result<std::collections::BTreeSet<MessageId>, ProjectionError> {
        let mut statement = self
            .conn
            .prepare_cached("SELECT id FROM records WHERE channel = ?1")?;
        let rows = statement.query_map(
            rusqlite::params![channel.as_bytes().to_vec()],
            |row| row.get::<_, Vec<u8>>(0),
        )?;
        let mut ids = std::collections::BTreeSet::new();
        for row in rows {
            ids.insert(message_id(&row?)?);
        }
        Ok(ids)
    }

    /// The newest reading folded in for a channel, if any.
    ///
    /// What decides whether an arrival extends the fold or reaches into it. A
    /// record that sorts after everything stored changes no verdict already
    /// decided; one that sorts before may change every verdict after it.
    pub fn newest(&self, channel: &ChannelId) -> Result<Option<Cursor>, ProjectionError> {
        let found = self.conn.query_row(
            "SELECT hlc_wall, hlc_counter, id FROM records WHERE channel = ?1
              ORDER BY hlc_wall DESC, hlc_counter DESC, id DESC LIMIT 1",
            rusqlite::params![channel.as_bytes().to_vec()],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, Vec<u8>>(2)?,
                ))
            },
        );
        match found {
            Ok((wall, counter, id)) => Ok(Some(Cursor::new(
                Hlc::new(wall, u32::try_from(counter).unwrap_or(u32::MAX)),
                message_id(&id)?,
            ))),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(err) => Err(err.into()),
        }
    }

    /// Forgets a channel's rows, so they can be folded again.
    pub fn forget(&self, channel: &ChannelId) -> Result<(), ProjectionError> {
        self.conn.execute(
            "DELETE FROM records WHERE channel = ?1",
            rusqlite::params![channel.as_bytes().to_vec()],
        )?;
        self.conn.execute(
            "DELETE FROM folds WHERE channel = ?1",
            rusqlite::params![channel.as_bytes().to_vec()],
        )?;
        Ok(())
    }
}

/// The message a record acts on, where it acts on one.
///
/// Matched here rather than asked of `RecordBody`, because "what does this act
/// on" is a question the index has and the record type does not: a message acts
/// on nothing, and every other kind names its target in its own field.
const fn target_of(body: &kols_core::RecordBody) -> Option<&MessageId> {
    match body {
        kols_core::RecordBody::Message { .. } => None,
        kols_core::RecordBody::Edit { target, .. }
        | kols_core::RecordBody::Tombstone { target }
        | kols_core::RecordBody::Reaction { target, .. }
        | kols_core::RecordBody::Pin { target, .. }
        | kols_core::RecordBody::Redaction { target, .. } => Some(target),
    }
}

const fn class_code(class: RecordClass) -> i64 {
    match class {
        RecordClass::Message => 0,
        RecordClass::Reaction => 1,
        RecordClass::Control => 2,
        RecordClass::Reserved => 3,
    }
}

fn message_id(bytes: &[u8]) -> Result<MessageId, ProjectionError> {
    let fixed: [u8; 32] = bytes
        .try_into()
        .map_err(|_| ProjectionError::Corrupt("a stored id is not 32 bytes".to_owned()))?;
    Ok(MessageId::from_bytes(fixed))
}

/// The limits a channel's verdicts were folded under.
///
/// Stored so a change can be noticed. A verdict is only true of the rules that
/// produced it, and the ceilings are network policy while slowmode belongs to
/// the channel — so a `define-policy` change or a moderator calming a channel
/// makes every verdict in scope stale, and a stale refusal is a message that
/// stays hidden after the rule that hid it was relaxed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FoldedUnder {
    /// The message-class ceiling.
    pub message_rate: i64,
    /// The reaction-class ceiling.
    pub reaction_rate: i64,
    /// The channel's slowmode, in seconds.
    pub slowmode: u32,
}

impl Projection {
    /// What a channel's verdicts were folded under, if anything has been.
    pub fn folded_under(
        &self,
        channel: &ChannelId,
    ) -> Result<Option<FoldedUnder>, ProjectionError> {
        let mut statement = self.conn.prepare_cached(
            "SELECT message_rate, reaction_rate, slowmode FROM folds WHERE channel = ?1",
        )?;
        let found = statement.query_row(rusqlite::params![channel.as_bytes().to_vec()], |row| {
            Ok(FoldedUnder {
                message_rate: row.get(0)?,
                reaction_rate: row.get(1)?,
                slowmode: u32::try_from(row.get::<_, i64>(2)?).unwrap_or(0),
            })
        });
        match found {
            Ok(under) => Ok(Some(under)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(err) => Err(err.into()),
        }
    }

    /// Records what a channel's verdicts are now folded under.
    pub fn folded(&self, channel: &ChannelId, under: FoldedUnder) -> Result<(), ProjectionError> {
        self.conn.execute(
            "INSERT OR REPLACE INTO folds (channel, message_rate, reaction_rate, slowmode)
             VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![
                channel.as_bytes().to_vec(),
                under.message_rate,
                under.reaction_rate,
                i64::from(under.slowmode),
            ],
        )?;
        Ok(())
    }
}
