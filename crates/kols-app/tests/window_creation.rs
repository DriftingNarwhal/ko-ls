//! A command that creates a window may not be synchronous.
//!
//! # Why this is a source rule and not a test that opens a window
//!
//! `WebviewWindowBuilder::build()` deadlocks on Windows when it runs in a
//! synchronous command or an event handler — Tauri documents it under *Known
//! issues*, pointing at [wry#583]. A synchronous `#[tauri::command]` runs its
//! body **inline in the IPC handler**, and on Windows that handler is dispatched
//! on the window's message-loop thread: the thread that has to pump WebView2's
//! controller-created message is the one blocked waiting for it.
//!
//! `v0.13.2` shipped that. Opening a network gave a **blank white window that
//! could not be closed**, and the tray's quit did nothing — three symptoms of
//! one stuck thread, and none of them an error anybody could see.
//!
//! **Every gate this project runs executes on Linux**, where WebKitGTK creates
//! its webview synchronously on the calling thread and the same code is
//! correct. So there is no test that could have failed: the container is not a
//! weaker test of this property, it is a test of a different platform. What is
//! checkable everywhere is the *shape* of the code, which is what this reads.
//!
//! # What it can and cannot see
//!
//! It reads `src/main.rs` and follows **one** level of call: a command that
//! builds a window itself, and a command that calls a helper which does. That
//! covers the shape this shell has — `open_network` builds through
//! `show_network_window` — and would flag a new helper of the same kind. It
//! does not walk a deeper chain, and it only sees top-level functions rather
//! than methods on an `impl`. A window built two calls down, or from a method,
//! would pass this and still deadlock, so this is a guard against the mistake
//! that was actually made rather than a proof about the file.
//!
//! [wry#583]: https://github.com/tauri-apps/wry/issues/583

const SHELL: &str = include_str!("../src/main.rs");

/// One top-level function: whether it is a command, whether it is `async`, and
/// its body.
struct Function<'a> {
    name: &'a str,
    command: bool,
    is_async: bool,
    body: &'a str,
}

/// Every top-level `fn` in the shell.
///
/// Found by scanning for a signature at the start of a line, which is what a
/// top-level item looks like in this file, and brace-matching the body from
/// there. Deliberately textual: bringing in a parser to answer one question
/// about one file would be a dependency with more surface than the rule.
fn functions(source: &str) -> Vec<Function<'_>> {
    let mut found = Vec::new();
    let bytes = source.as_bytes();

    for (offset, _) in source.match_indices("fn ") {
        // The start of a line, optionally preceded by `async `, `pub ` or both.
        let line_start = source[..offset].rfind('\n').map(|at| at + 1).unwrap_or(0);
        let prefix = &source[line_start..offset];
        if !matches!(prefix, "" | "pub " | "async " | "pub async ") {
            continue;
        }
        let is_async = prefix.contains("async");

        let rest = &source[offset + 3..];
        let name_end = rest
            .find(|c: char| !c.is_alphanumeric() && c != '_')
            .unwrap_or(rest.len());
        let name = &rest[..name_end];
        if name.is_empty() {
            continue;
        }

        // Whatever is attached above the signature, back to the last blank line:
        // the attributes and doc comment, which is where `#[tauri::command]` is.
        let attached = source[..line_start].rfind("\n\n").map(|at| at + 2).unwrap_or(0);
        let command = source[attached..line_start].contains("#[tauri::command]");

        // The body, by brace matching. Good enough on this file, which has no
        // braces inside string literals at top level.
        let Some(open) = source[offset..].find('{').map(|at| offset + at) else {
            continue;
        };
        let mut depth = 0usize;
        let mut end = open;
        for (at, byte) in bytes[open..].iter().enumerate() {
            match byte {
                b'{' => depth += 1,
                b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        end = open + at;
                        break;
                    }
                }
                _ => {}
            }
        }

        found.push(Function {
            name,
            command,
            is_async,
            body: &source[open..=end],
        });
    }
    found
}

#[test]
fn no_synchronous_command_creates_a_window() {
    let functions = functions(SHELL);
    assert!(
        functions.len() > 40,
        "only {} functions found, so the scan is not reading the file",
        functions.len()
    );

    // The helpers that build a window. A command reaching one of these is
    // building a window as surely as if it held the builder itself.
    let builders: Vec<&str> = functions
        .iter()
        .filter(|function| function.body.contains("WebviewWindowBuilder"))
        .map(|function| function.name)
        .collect();
    assert!(
        !builders.is_empty(),
        "nothing in the shell builds a window, so this test is guarding nothing"
    );

    let mut offenders = Vec::new();
    for function in functions.iter().filter(|function| function.command) {
        let builds_here = function.body.contains("WebviewWindowBuilder");
        let builds_below = builders
            .iter()
            .any(|helper| *helper != function.name && function.body.contains(&format!("{helper}(")));
        if (builds_here || builds_below) && !function.is_async {
            offenders.push(function.name);
        }
    }

    assert!(
        offenders.is_empty(),
        "these commands create a window synchronously, which deadlocks on Windows \
         (wry#583): {offenders:?}. Make them `async fn` — Tauri's own remedy — so the \
         body runs on the async runtime instead of the message-loop thread."
    );
}

/// The same rule for the other half of Tauri's sentence: *not in event
/// handlers*.
///
/// The tray's menu handler and the window-event handler both run on the main
/// thread, so a window built from either deadlocks exactly as a synchronous
/// command does. Neither does today; this is here so that adding one is a
/// failing test rather than a report from somebody's desktop.
#[test]
fn no_event_handler_creates_a_window() {
    for handler in ["on_menu_event", "on_tray_icon_event", "on_window_event"] {
        let Some(at) = SHELL.find(handler) else {
            continue;
        };
        // The closure that follows, by brace matching from its first `{`.
        let Some(open) = SHELL[at..].find('{').map(|offset| at + offset) else {
            continue;
        };
        let mut depth = 0usize;
        let mut end = open;
        for (offset, byte) in SHELL.as_bytes()[open..].iter().enumerate() {
            match byte {
                b'{' => depth += 1,
                b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        end = open + offset;
                        break;
                    }
                }
                _ => {}
            }
        }
        let closure = &SHELL[open..=end];
        assert!(
            !closure.contains("WebviewWindowBuilder"),
            "{handler} builds a window on the main thread, which deadlocks on Windows \
             (wry#583). Hand it to the async runtime instead."
        );
    }
}
