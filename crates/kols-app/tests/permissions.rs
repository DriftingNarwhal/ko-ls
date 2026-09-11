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

/// The window declared in the configuration and opened at launch.
const WORKSPACE: &str = "workspace";

/// The window a network is drawn in, created at runtime — `design/09` §1.5.
///
/// **Created rather than declared, which is exactly why it is listed here.** A
/// capability is scoped to labels, so a window whose label no capability names
/// gets an empty allow-list — the silent denial above, per window rather than
/// per application. A runtime window has no entry in `tauri.conf.json` for
/// anybody to notice is missing, so this test is the only thing that would.
const NETWORK: &str = "network";

/// A conversation window, one per conversation — `design/09` §1.6.
const CONVERSATION: &str = "conversation-aabbccdd";

/// A second network window, which `design/09` §1.5 allows on request.
///
/// Covered by the `network-*` pattern ahead of the window existing, so that
/// building it is not a second encounter with this.
const SECOND_NETWORK: &str = "network-2";

/// Every `plugin:` command the interface reaches for, by name.
const NEEDED: &[&str] = &[
    "plugin:event|listen",
    "plugin:event|unlisten",
    "plugin:window|title",
    "plugin:window|set_title",
    "plugin:window|is_focused",
    "plugin:window|request_user_attention",
];

#[test]
fn the_webview_may_call_what_the_interface_calls_and_may_drag() {
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
        [WORKSPACE],
        "the configuration declares one window at launch, and `09` §1.13 makes it \
         the workspace — the network window is created when a network is opened"
    );

    // **Tauri's native drag-and-drop handler is on by default and swallows HTML5
    // drag events**, which is what kept channel reordering from ever working —
    // the wiring was right, the drag never began. Tauri's own documentation on
    // the field says disabling it is *required* to use HTML5 drag and drop on
    // the frontend on Windows.
    //
    // Asserted here rather than trusted to a config file nobody reads, because
    // this is the second time a Tauri default has silently removed a feature
    // from this application and the first cost the whole event path. It also
    // records the trade: with this off the window cannot receive files dropped
    // from the desktop, which costs nothing while there are no attachments and
    // is a decision to revisit when there are.
    assert!(
        context
            .config()
            .app
            .windows
            .iter()
            .all(|window| !window.drag_drop_enabled),
        "the native drag handler is on, so nothing in the interface can be dragged"
    );

    let authority = context.runtime_authority_mut();

    // **Every label this application creates, not just the declared one.** The
    // network window is built at runtime and the second one is not built yet;
    // both are checked, because the failure they would produce is the one this
    // whole file exists for and it produces no output.
    for window in [WORKSPACE, NETWORK, SECOND_NETWORK, CONVERSATION] {
        let refused: Vec<&str> = NEEDED
            .iter()
            .copied()
            .filter(|command| {
                authority
                    .resolve_access(command, window, window, &Origin::Local)
                    .is_none()
            })
            .collect();

        assert!(
            refused.is_empty(),
            "the ACL refuses {refused:?} for the `{window}` window — the interface calls \
             these there and would be silently denied"
        );
    }
}

/// A label no capability names gets nothing, which is the property being relied on.
///
/// The example used to be `conversation`, which stopped being an unnamed label
/// the moment conversation windows existed — so it is a label this application
/// will never create. A negative test whose subject quietly becomes real is a
/// negative test that passes for the wrong reason.
///
/// Asserted rather than assumed, because the list above is only protection if
/// an unlisted label genuinely fails — a pattern that matched everything would
/// make that test pass for the wrong reason and hide the next missing label.
#[test]
fn a_window_no_capability_names_is_refused_everything() {
    let mut context: tauri::Context<tauri::Wry> = tauri::generate_context!();
    let authority = context.runtime_authority_mut();
    assert!(
        NEEDED.iter().all(|command| authority
            .resolve_access(command, "not-a-window", "not-a-window", &Origin::Local)
            .is_none()),
        "an unnamed label resolves, so the capability list is not what is granting \
         access and a missing label would not be caught"
    );
}
