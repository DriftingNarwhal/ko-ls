//! Invites — Core §5.6, `design/02` §6.1.
//!
//! # What an invite is for
//!
//! Getting a stranger from knowing nothing to holding a per-network identity and
//! a connection. Everything after that is ordinary post-connection sync (§5.7),
//! which is why this module is short: the protocol's `intranet-invite` crate
//! already issues, encodes, validates and presents them, and the job here is to
//! make one shareable and to know what to do with one somebody sends you.
//!
//! # The URI is a container, not a format
//!
//! `intranet-chat://join/<base32 of the invite's canonical bytes>`. The bytes
//! are the protocol's and are normative; the scheme and the base32 are this
//! client's, so that an invite survives being pasted into a chat message, an
//! email or a terminal. Base32 without padding, upper-cased, because it survives
//! case-insensitive handling and double-click selection in a way base64 does
//! not — an invite that breaks when somebody copies it out of a message has
//! failed at its only job.

use intranet_invite::{Invite, decode_invite, encode_invite};

/// The URI scheme an invite travels under.
pub const SCHEME: &str = "intranet-chat://join/";

/// Renders an invite as something a person can paste.
pub fn to_uri(invite: &Invite) -> String {
    format!("{SCHEME}{}", base32(&encode_invite(invite)))
}

/// Renders already-encoded invite bytes as something a person can paste.
///
/// The boundary hands back bytes rather than a URI, deliberately: how an invite
/// is *carried* is presentation, and this is the terminal deciding.
pub fn to_uri_from_bytes(encoded: &[u8]) -> String {
    format!("{SCHEME}{}", base32(encoded))
}

/// Reads an invite back, accepting the bare payload as well as a full URI.
///
/// Accepting both is deliberate. People paste what they were given, and what
/// they were given may have lost its scheme to a chat client that linkified it,
/// or gained whitespace from a terminal wrap.
pub fn from_uri(text: &str) -> Result<Invite, String> {
    let trimmed: String = text.trim().chars().filter(|c| !c.is_whitespace()).collect();
    let payload = trimmed
        .strip_prefix(SCHEME)
        .or_else(|| trimmed.strip_prefix("intranet-chat:/join/"))
        .unwrap_or(&trimmed);

    let bytes = unbase32(payload).ok_or("that does not look like an invite")?;
    decode_invite(&bytes).map_err(|err| format!("that invite did not decode: {err}"))
}

const ALPHABET: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";

/// RFC 4648 base32, unpadded.
///
/// Hand-rolled rather than pulled in: it is fifteen lines, and a dependency
/// whose output feeds a user-visible identifier is one more thing that can
/// change under the project without anybody noticing.
fn base32(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(5) * 8);
    let (mut buffer, mut bits) = (0u32, 0u32);
    for byte in bytes {
        buffer = (buffer << 8) | u32::from(*byte);
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            out.push(ALPHABET[((buffer >> bits) & 0x1F) as usize] as char);
        }
    }
    if bits > 0 {
        out.push(ALPHABET[((buffer << (5 - bits)) & 0x1F) as usize] as char);
    }
    out
}

fn unbase32(text: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(text.len() * 5 / 8);
    let (mut buffer, mut bits) = (0u32, 0u32);
    for character in text.chars() {
        let upper = character.to_ascii_uppercase();
        let value = ALPHABET.iter().position(|c| *c as char == upper)? as u32;
        buffer = (buffer << 5) | value;
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            out.push(((buffer >> bits) & 0xFF) as u8);
        }
    }
    // Any remaining bits are the encoder's tail padding and must be zero. A
    // non-zero remainder means the text was truncated or edited, which is worth
    // refusing rather than decoding into a shorter invite that fails a signature
    // check later for a reason nobody can trace back to here.
    if bits > 0 && (buffer & ((1 << bits) - 1)) != 0 {
        return None;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use intranet_crypto::Timestamp;
    use intranet_identity::{MasterSeed, NetworkId};
    use intranet_invite::InviteSubject;

    fn an_invite() -> Invite {
        let network = NetworkId::from_bytes([8u8; 32]);
        let issuer = MasterSeed::from_entropy([1u8; 32])
            .identity_for(&network)
            .expect("derives");
        Invite::issue(
            &issuer,
            vec!["/ip4/127.0.0.1/tcp/4001/p2p/12D3KooWTest".to_owned()],
            InviteSubject::Bearer,
            Timestamp::from_millis(1_000),
            Timestamp::from_millis(9_000),
            3,
        )
    }

    #[test]
    fn an_invite_survives_the_round_trip() {
        let invite = an_invite();
        assert_eq!(from_uri(&to_uri(&invite)).expect("decodes"), invite);
    }

    #[test]
    fn it_survives_being_pasted_badly() {
        // The failure this exists to prevent: an invite that works when typed
        // carefully and not when copied out of a chat message.
        let invite = an_invite();
        let uri = to_uri(&invite);
        for mangled in [
            format!("  {uri}  "),
            uri.replace(SCHEME, ""),
            uri.to_lowercase(),
            format!("{}\n{}", &uri[..30], &uri[30..]),
        ] {
            assert_eq!(
                from_uri(&mangled).expect("decodes"),
                invite,
                "failed on {mangled:?}"
            );
        }
    }

    #[test]
    fn a_truncated_invite_is_refused_here_rather_than_later() {
        let uri = to_uri(&an_invite());
        let short = &uri[..uri.len() - 3];
        assert!(
            from_uri(short).is_err(),
            "a truncated invite decoded into something"
        );
    }

    #[test]
    fn nonsense_is_refused() {
        assert!(from_uri("hello").is_err());
        assert!(from_uri("").is_err());
        assert!(from_uri("intranet-chat://join/!!!!").is_err());
    }
}
