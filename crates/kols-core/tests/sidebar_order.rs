//! The network's default sidebar order — spec 07 §1.6.
//!
//! The claim under test is that the order is **total and identical on every
//! node**. That is not something a client can be trusted to arrive at by
//! agreeing to sort nicely: it has to fall out of the rule regardless of the
//! order records happened to arrive in, which is why the permutation test at the
//! bottom is the one that matters most here.

use kols_core::*;

fn channel(byte: u8) -> ChannelId {
    ChannelId::from_bytes([byte; 32])
}

fn category(byte: u8) -> CategoryId {
    CategoryId::from_bytes([byte; 32])
}

fn ch(byte: u8, cat: Option<u8>, position: Option<u32>) -> SidebarChannel {
    SidebarChannel {
        id: channel(byte),
        category: cat.map(category),
        position,
    }
}

fn cat(byte: u8, position: Option<u32>) -> SidebarCategory {
    SidebarCategory {
        id: category(byte),
        position,
    }
}

#[test]
fn uncategorised_channels_come_before_every_category() {
    let rows = sidebar_order(
        &[ch(0x20, Some(0x90), Some(0)), ch(0x10, None, Some(5))],
        &[cat(0x90, Some(0))],
    );

    // Even with the loose channel positioned 5 and the category positioned 0:
    // positions compare among siblings, and these two are not siblings.
    assert_eq!(
        rows,
        vec![
            SidebarRow::Channel(channel(0x10)),
            SidebarRow::Category {
                id: category(0x90),
                channels: vec![channel(0x20)],
            },
        ],
    );
}

#[test]
fn a_sibling_never_positioned_sorts_after_every_sibling_that_was() {
    let rows = sidebar_order(
        &[ch(0x11, None, None), ch(0x12, None, Some(9))],
        &[cat(0x91, None), cat(0x92, Some(9))],
    );

    // Position 9 beats "no position", at both levels. Sorting an absent position
    // as zero would put brand-new structure at the top of everybody's sidebar.
    assert_eq!(rows[0], SidebarRow::Channel(channel(0x12)));
    assert_eq!(rows[1], SidebarRow::Channel(channel(0x11)));
    assert!(matches!(&rows[2], SidebarRow::Category { id, .. } if *id == category(0x92)));
    assert!(matches!(&rows[3], SidebarRow::Category { id, .. } if *id == category(0x91)));
}

#[test]
fn channels_sort_within_their_own_category() {
    let rows = sidebar_order(
        &[
            ch(0x21, Some(0x90), Some(2)),
            ch(0x22, Some(0x91), Some(1)),
            ch(0x23, Some(0x90), Some(1)),
        ],
        &[cat(0x90, Some(0)), cat(0x91, Some(1))],
    );

    assert_eq!(
        rows,
        vec![
            SidebarRow::Category {
                id: category(0x90),
                channels: vec![channel(0x23), channel(0x21)],
            },
            SidebarRow::Category {
                id: category(0x91),
                channels: vec![channel(0x22)],
            },
        ],
    );
}

#[test]
fn equal_positions_break_by_id_rather_than_being_refused() {
    // Two managers setting the same position concurrently is not preventable by
    // a log with no locks, so it must not be an error. What keeps every reader
    // agreeing is the tie-break, not a refusal nobody could act on.
    let rows = sidebar_order(
        &[ch(0x31, None, Some(4)), ch(0x30, None, Some(4))],
        &[cat(0x81, Some(7)), cat(0x80, Some(7))],
    );

    assert_eq!(rows[0], SidebarRow::Channel(channel(0x30)));
    assert_eq!(rows[1], SidebarRow::Channel(channel(0x31)));
    assert!(matches!(&rows[2], SidebarRow::Category { id, .. } if *id == category(0x80)));
    assert!(matches!(&rows[3], SidebarRow::Category { id, .. } if *id == category(0x81)));
}

#[test]
fn a_channel_may_name_a_category_nothing_ever_defined() {
    // Spec 07 §1.8: not an error. It sorts as a category with no position, and
    // what a client calls it is a client's business.
    let rows = sidebar_order(
        &[ch(0x40, Some(0xAA), Some(0)), ch(0x41, Some(0x90), Some(0))],
        &[cat(0x90, Some(3))],
    );

    assert_eq!(
        rows,
        vec![
            SidebarRow::Category {
                id: category(0x90),
                channels: vec![channel(0x41)],
            },
            SidebarRow::Category {
                id: category(0xAA),
                channels: vec![channel(0x40)],
            },
        ],
    );
}

#[test]
fn a_defined_category_holding_no_channels_still_appears() {
    // A folder somebody made and has not filled is structure, not nothing. If it
    // vanished, creating one would look like it failed.
    let rows = sidebar_order(&[], &[cat(0x90, Some(0))]);
    assert_eq!(
        rows,
        vec![SidebarRow::Category {
            id: category(0x90),
            channels: vec![],
        }],
    );
}

#[test]
fn the_order_does_not_depend_on_the_order_it_was_given_in() {
    // The whole normative claim in one test. Every node computes this from a log
    // it may have received in any order, so a rule that is merely *usually*
    // stable is a rule two members disagree about.
    let channels = vec![
        ch(0x10, None, Some(1)),
        ch(0x11, None, None),
        ch(0x20, Some(0x90), Some(2)),
        ch(0x21, Some(0x90), Some(2)),
        ch(0x22, Some(0x91), None),
        ch(0x23, Some(0xAA), Some(0)),
        ch(0x12, None, Some(1)),
    ];
    let categories = vec![cat(0x90, Some(5)), cat(0x91, None), cat(0x92, Some(5))];

    let expected = sidebar_order(&channels, &categories);

    // Enough shapes to catch a comparator that leans on input order: reversal,
    // every rotation, and a swap of each adjacent pair.
    let mut permutations: Vec<(Vec<SidebarChannel>, Vec<SidebarCategory>)> = Vec::new();

    let mut reversed = channels.clone();
    reversed.reverse();
    let mut reversed_cats = categories.clone();
    reversed_cats.reverse();
    permutations.push((reversed, reversed_cats));

    for shift in 1..channels.len() {
        let mut rotated = channels.clone();
        rotated.rotate_left(shift);
        let mut rotated_cats = categories.clone();
        rotated_cats.rotate_left(shift % categories.len());
        permutations.push((rotated, rotated_cats));
    }

    for i in 0..channels.len() - 1 {
        let mut swapped = channels.clone();
        swapped.swap(i, i + 1);
        permutations.push((swapped, categories.clone()));
    }

    for (n, (c, k)) in permutations.iter().enumerate() {
        assert_eq!(
            sidebar_order(c, k),
            expected,
            "permutation {n} produced a different order",
        );
    }
}
