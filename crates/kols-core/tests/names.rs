//! Display names — spec 07 §3.9.
//!
//! # What is worth pinning here
//!
//! The name key, mostly. Uniqueness is only meaningful if every node computes
//! the same key from the same name, so these are conformance tests for a rule
//! rather than tests of a helper: each one names the step of §3.9.1 it holds.

use kols_core::*;

fn person(seed: u8) -> intranet_identity::PerNetworkIdentityId {
    intranet_identity::MasterSeed::from_entropy([seed; 32])
        .identity_for(&intranet_identity::NetworkId::from_bytes([4u8; 32]))
        .expect("derives")
        .id()
}

// ------------------------------------------------------------- normalization

#[test]
fn case_does_not_make_a_new_name() {
    let key = name_key("alice").expect("claimable");
    for spelling in ["Alice", "ALICE", "aLiCe"] {
        assert_eq!(name_key(spelling).expect("claimable"), key, "{spelling}");
    }
}

#[test]
fn surrounding_and_repeated_whitespace_does_not_make_a_new_name() {
    let key = name_key("ada lovelace").expect("claimable");
    for spelling in [
        "  ada lovelace",
        "ada lovelace  ",
        "ada   lovelace",
        "ada\u{00A0}lovelace", // no-break space, a separator
        "ada\u{2003}lovelace", // em space
    ] {
        assert_eq!(name_key(spelling).expect("claimable"), key, "{spelling:?}");
    }
}

#[test]
fn whitespace_that_is_a_control_character_is_refused_not_collapsed() {
    // The interaction between step 1 and step 3 of §3.9.1, which reads
    // ambiguously until you try it: a tab is a control character, so it is
    // refused before anything gets collapsed. Only *separators* are collapsed.
    // Refusing is the consistent answer — silently collapsing a character the
    // user cannot see is the behaviour step 1 exists to avoid.
    for control in ["ada\tlovelace", "ada\nlovelace", "ada\rlovelace"] {
        assert!(
            matches!(name_key(control), Err(NameRefusal::Invisible { .. })),
            "{control:?} was not refused"
        );
    }
}

#[test]
fn internal_spacing_still_distinguishes_names() {
    // Collapsing runs is not the same as deleting spaces: "alice" and "al ice"
    // are different names and stay so.
    assert_ne!(
        name_key("alice").expect("claimable"),
        name_key("al ice").expect("claimable")
    );
}

#[test]
fn compatibility_forms_fold_together() {
    // NFKC: the ligature and the fullwidth letters are the same name as the
    // plain one, which is the point of using a compatibility normalization for
    // a *key* while displaying whatever was typed.
    assert_eq!(
        name_key("ﬁona").expect("claimable"),
        name_key("fiona").expect("claimable")
    );
    assert_eq!(
        name_key("ALICE").expect("claimable"),
        name_key("alice").expect("claimable")
    );
}

#[test]
fn invisible_characters_are_refused_rather_than_stripped() {
    // Stripping would let two claims that look identical produce one key and
    // one holder, quietly. Refusing says so. A name nobody can see cannot be
    // checked by the person it misleads.
    for (label, name) in [
        ("zero-width space", "al\u{200B}ice"),
        ("zero-width joiner", "al\u{200D}ice"),
        ("right-to-left override", "al\u{202E}ice"),
        ("soft hyphen", "al\u{00AD}ice"),
        ("private use", "al\u{E000}ice"),
        ("control", "al\u{0007}ice"),
    ] {
        assert!(
            matches!(name_key(name), Err(NameRefusal::Invisible { .. })),
            "{label} was not refused"
        );
    }
}

#[test]
fn an_empty_or_whitespace_name_is_refused() {
    assert_eq!(name_key(""), Err(NameRefusal::Empty));
    assert_eq!(name_key("   "), Err(NameRefusal::Empty));
}

#[test]
fn both_bounds_are_checked_because_neither_implies_the_other() {
    // Bytes bound what a node stores and relays; graphemes bound what a name
    // occupies on a screen. A short-looking name can be long in bytes, and a
    // few code points can be one grapheme.
    let long_in_bytes = "é".repeat(40); // 80 bytes, 40 graphemes
    assert!(matches!(
        name_key(&long_in_bytes),
        Err(NameRefusal::TooManyBytes { .. })
    ));

    let many_graphemes = "a".repeat(MAX_NAME_GRAPHEMES + 1);
    assert!(many_graphemes.len() <= MAX_NAME_BYTES, "byte bound not the one under test");
    assert!(matches!(
        name_key(&many_graphemes),
        Err(NameRefusal::TooManyGraphemes { .. })
    ));

    // One family emoji is many code points and one grapheme.
    assert!(name_key("👩‍👩‍👧‍👦").is_err(), "the ZWJ inside it is refused");
}

#[test]
fn homoglyphs_are_not_caught_and_the_test_says_so() {
    // Deliberate, and spec 07 §3.9.1 says why: confusable tables are large, they
    // collide names across scripts with every right to exist, and two nodes on
    // different table versions would disagree about what is a duplicate. The
    // obligation this creates lands on the interface, which renders a name with
    // enough of its holder's identity to tell two apart.
    assert_ne!(
        name_key("alice").expect("claimable"),
        name_key("alicе").expect("claimable"), // Cyrillic 'е'
        "if this ever becomes equal, spec 07 §3.9.1 changed and §8 should too"
    );
}

// ---------------------------------------------------------------- uniqueness

#[test]
fn the_first_claim_binds_and_a_second_by_somebody_else_does_not() {
    let (alice, bob) = (person(2), person(3));
    let mut names = Names::new();

    assert!(names.apply(alice, &NameClaim::new("alice").expect("valid")));
    assert!(!names.apply(bob, &NameClaim::new("Alice").expect("valid")));

    assert_eq!(names.of(&alice), Some("alice"));
    assert_eq!(names.of(&bob), None);
    assert_eq!(names.holder("alice"), Some(&alice));
}

#[test]
fn a_holder_may_respell_their_own_name() {
    // Same key, different display form: the key is unchanged so nothing is
    // taken from anybody, and what renders is what was typed.
    let alice = person(2);
    let mut names = Names::new();
    names.apply(alice, &NameClaim::new("alice").expect("valid"));
    assert!(names.apply(alice, &NameClaim::new("ALICE").expect("valid")));
    assert_eq!(names.of(&alice), Some("ALICE"));
    assert_eq!(names.len(), 1, "respelling should not bind a second key");
}

#[test]
fn a_member_who_renames_keeps_the_name_they_left() {
    // Permanence, spec 07 §3.9.2: the old key stays bound to them, so nobody
    // else can pick it up and inherit their history.
    let (alice, bob) = (person(2), person(3));
    let mut names = Names::new();
    names.apply(alice, &NameClaim::new("alice").expect("valid"));
    names.apply(alice, &NameClaim::new("ada").expect("valid"));

    assert_eq!(names.of(&alice), Some("ada"));
    assert!(
        !names.apply(bob, &NameClaim::new("alice").expect("valid")),
        "the abandoned name was inheritable"
    );
    assert_eq!(names.holder("alice"), Some(&alice));
}

#[test]
fn claimable_answers_before_anything_is_written() {
    let (alice, bob) = (person(2), person(3));
    let mut names = Names::new();
    names.apply(alice, &NameClaim::new("alice").expect("valid"));

    assert!(names.claimable(&alice, "Alice").is_ok(), "their own key");
    assert!(matches!(
        names.claimable(&bob, "ALICE"),
        Err(NameRefusal::Taken { .. })
    ));
    assert!(names.claimable(&bob, "bob").is_ok());
}

// ------------------------------------------------------------------ encoding

#[test]
fn a_claim_round_trips_and_carries_no_identity() {
    let claim = NameClaim::new("ada lovelace").expect("valid");
    let bytes = claim.encode();
    assert_eq!(NameClaim::decode_payload(&bytes).expect("decodes"), claim);

    // The security property, as a test: there is no identity in the payload, so
    // the encoded form cannot be longer than the name plus its framing.
    assert!(
        bytes.len() < claim.name.len() + 64,
        "the payload grew — did something start carrying an identity?"
    );
}

#[test]
fn a_claim_is_only_readable_when_it_declared_the_right_capability() {
    let claim = NameClaim::new("ada").expect("valid");
    let body = claim.to_app_entry();
    assert_eq!(NameClaim::read(&body), Some(claim));

    // The reader obligation E2's generalisation moved onto clients: an author
    // holding `chat:post:*` must not be able to mint a name by declaring it.
    let intranet_governance::EntryBody::AppEntry {
        namespace,
        kind,
        payload,
        ..
    } = body
    else {
        unreachable!()
    };
    let forged = intranet_governance::EntryBody::AppEntry {
        namespace,
        kind,
        required: intranet_governance::Capability::extension("chat:post:*".to_owned()),
        payload,
    };
    assert_eq!(NameClaim::read(&forged), None);
}
