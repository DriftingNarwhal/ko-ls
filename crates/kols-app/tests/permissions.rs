//! What the webview is allowed to call.
//!
//! Tauri v2 gates every `plugin:` command on the ACL, and an application that
//! defines no capabilities gets an empty one — so `listen` is refused and every
//! push from the node is dropped on the floor. Nothing says so: `listen`
//! rejects a promise the interface does not await, the polls keep drawing, and
//! the window looks alive while none of the events reach it.
//!
//! This asserts the commands the interface actually calls, resolved against the
//! real configuration rather than against a copy of it.

use tauri::ipc::Origin;

/// The window `capabilities/default.json` grants these to.
const WINDOW: &str = "main";

/// Every `plugin:` command `app.js` reaches for, by name.
const NEEDED: &[&str] = &[
    "plugin:event|listen",
    "plugin:event|unlisten",
    "plugin:window|title",
    "plugin:window|set_title",
    "plugin:window|is_focused",
    "plugin:window|request_user_attention",
];

#[test]
fn the_webview_may_call_what_the_interface_calls() {
    let mut context: tauri::Context<tauri::Wry> = tauri::generate_context!();

    // A capability names the windows it applies to, and a window's label
    // defaults rather than being written down — so the two can drift apart
    // without either file looking wrong, and the ACL would then refuse
    // everything again in exactly the way that is invisible from the outside.
    let labels: Vec<&str> = context
        .config()
        .app
        .windows
        .iter()
        .map(|window| window.label.as_str())
        .collect();
    assert_eq!(
        labels,
        [WINDOW],
        "capabilities/default.json names the window `{WINDOW}`"
    );

    let authority = context.runtime_authority_mut();

    let refused: Vec<&str> = NEEDED
        .iter()
        .copied()
        .filter(|command| {
            authority
                .resolve_access(command, WINDOW, WINDOW, &Origin::Local)
                .is_none()
        })
        .collect();

    assert!(
        refused.is_empty(),
        "the ACL refuses {refused:?} — the interface calls these and would be silently denied"
    );
}
