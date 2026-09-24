//! The error messages that tell a member what to do next.
//!
//! Most error text is free to change. These are not: each one names an action
//! (a command to run, a variable to set) or a consequence (a seed that cannot
//! be recovered), and a member follows them literally. They are derived by
//! `thiserror` from an attribute a few lines from the variant, where an edit
//! looks like tidying, so they are pinned here instead of trusted.

use kols_node::account::{AccountError, PASSWORD_VAR};
use kols_node::bundle::BundleError;
use kols_node::store::StoreError;
use std::path::PathBuf;

#[test]
fn a_locked_installation_names_the_variable_a_terminal_user_must_set() {
    let text = StoreError::Locked("no password given".into()).to_string();
    assert_eq!(
        text,
        format!(
            "this installation is locked: no password given. Log in to open it, or set \
             {PASSWORD_VAR} for the terminal"
        )
    );
    assert!(text.contains("KOLS_PASSWORD"), "{text}");
}

#[test]
fn a_missing_network_names_both_ways_to_get_one() {
    assert_eq!(
        StoreError::NotInitialised(PathBuf::from("/home/a/.kols")).to_string(),
        "no network at /home/a/.kols. Run `kols init <name>` to create one, or `kols join <invite>`"
    );
}

#[test]
fn refusing_to_overwrite_says_the_seed_cannot_be_recovered() {
    assert_eq!(
        StoreError::AlreadyInitialised(PathBuf::from("/home/a/.kols")).to_string(),
        "a network already exists at /home/a/.kols. Refusing to overwrite it — the seed there \
         cannot be recovered if it is lost"
    );
}

#[test]
fn a_wrong_password_and_a_wrong_passphrase_are_told_apart() {
    assert_eq!(
        AccountError::WrongPassword.to_string(),
        "that password does not unlock this installation"
    );
    assert_eq!(
        BundleError::WrongPassphrase.to_string(),
        "that passphrase does not open this backup"
    );
}
