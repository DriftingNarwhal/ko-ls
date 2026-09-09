//! What the reader-side limits made of a record — `design/01` §10.4.

/// Whether a record survived the rate pass.
///
/// **Only the refusal is stored, and the asymmetry is the point.** A record
/// refused for exceeding its author's ceiling is refused for good: the verdict
/// is a function of the records around it and never of the clock. A record
/// *held* for being dated ahead of now is a different thing — it is still in the
/// set, still counts toward its author's rate, and renders the moment local time
/// reaches its claim (`01` §4). Storing that would freeze an answer whose whole
/// nature is to expire, so held is computed per render and never written here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// The record renders.
    Admitted,
    /// The record's author was already at their ceiling for its class.
    OverRate,
    /// The channel is in slowmode and the author's last message was too recent.
    TooSoon,
}

impl Verdict {
    /// How it is stored, and the numbers are permanent.
    ///
    /// A projection outlives the build that wrote it — it is rebuilt when the
    /// schema changes and not when a variant is added — so renumbering these
    /// would silently reinterpret rows written yesterday.
    pub const fn code(self) -> i64 {
        match self {
            Self::Admitted => 0,
            Self::OverRate => 1,
            Self::TooSoon => 2,
        }
    }

    /// Reads a stored code, treating anything unknown as admitted.
    ///
    /// Fails **open** rather than closed, which is the opposite of this
    /// project's usual direction and is right here: an unrecognised verdict is a
    /// row written by a newer build, and the harm of showing a message that
    /// should have been refused for rate is smaller than the harm of hiding
    /// somebody's messages because their client was older. The refusal is a
    /// flood control, not an access control.
    pub const fn from_code(code: i64) -> Self {
        match code {
            1 => Self::OverRate,
            2 => Self::TooSoon,
            _ => Self::Admitted,
        }
    }

    /// Whether a record with this verdict is rendered.
    pub const fn renders(self) -> bool {
        matches!(self, Self::Admitted)
    }

    /// How a reader is told about it.
    ///
    /// A refusal is surfaced rather than dropped (`design/05` §3): a record this
    /// node refuses is one another client may be showing, and silence would make
    /// the two look like they agree. So a stored verdict has to be able to say
    /// *why*, in the same vocabulary the per-record checks use.
    pub const fn rejection(self) -> Option<kols_core::Rejection> {
        match self {
            Self::Admitted => None,
            Self::OverRate => Some(kols_core::Rejection::TooFast),
            Self::TooSoon => Some(kols_core::Rejection::Slowmode),
        }
    }
}

/// Decides one record's verdict from what the projection already holds.
///
/// # Why this is a query and not a walk
///
/// `design/01` §10.4's pass is a greedy fold in merge order: a record is refused
/// when its author's ceiling is already met by the records *already admitted* in
/// its trailing window. Read as a loop that is what makes rendering cost the
/// whole channel — but the state the loop carries is **bounded**. Only readings
/// inside the window count, and only admitted ones occupy it, so a verdict is a
/// function of the stored verdicts of that author's records in the last minute.
/// The projection can answer that with an index.
///
/// # What it does not decide
///
/// Whether a record is *held*. That compares its reading to now, so it is a
/// per-record question with an expiring answer and belongs to the render rather
/// than to the index — see [`Verdict`].
pub fn decide(
    projection: &crate::Projection,
    record: &kols_core::Record,
    limits: &kols_core::ReaderLimits,
) -> Result<Verdict, crate::ProjectionError> {
    use kols_core::RecordClass;

    let class = record.body.class();
    let ceiling = match class {
        RecordClass::Message => limits.message_rate_per_minute,
        RecordClass::Reaction => limits.reaction_rate_per_minute,
        // Control records are governed by capability instead, and a reserved one
        // never reaches here.
        RecordClass::Control | RecordClass::Reserved => return Ok(Verdict::Admitted),
    };

    // Slowmode first, since where it is set at all it is the stricter of the two
    // (`01` §10.3) — and it costs the record its slot either way.
    if class == RecordClass::Message && limits.slowmode_seconds > 0 {
        let interval = i64::from(limits.slowmode_seconds).saturating_mul(1_000);
        let since = record.hlc.wall_millis.saturating_sub(interval);
        let recent = projection.admitted_since(
            &record.channel,
            &record.author,
            RecordClass::Message,
            since,
        )?;
        if recent.iter().any(|hlc| *hlc < record.hlc) {
            return Ok(Verdict::TooSoon);
        }
    }

    if ceiling <= 0 {
        return Ok(Verdict::Admitted);
    }

    let since = record.hlc.wall_millis.saturating_sub(WINDOW_MILLIS);
    let admitted = projection.admitted_since(&record.channel, &record.author, class, since)?;
    // Strictly earlier, because a record does not count against itself — and a
    // re-fold walks records it has already stored, so its own row is in there.
    let occupied = admitted.iter().filter(|hlc| **hlc < record.hlc).count();
    if occupied as i64 >= ceiling {
        return Ok(Verdict::OverRate);
    }
    Ok(Verdict::Admitted)
}

/// The window a ceiling counts over — `design/01` §10.1's "per minute".
const WINDOW_MILLIS: i64 = 60_000;
