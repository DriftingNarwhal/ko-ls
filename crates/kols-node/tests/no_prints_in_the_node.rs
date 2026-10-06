//! Nothing the window runs prints: no print macro, no `io::stdout()` or `io::stderr()`.
//!
//! # Why this is a source rule
//!
//! The window runs `kols_node::serve::serve` in-process for every network, with
//! a report that goes nowhere (`kols_node::quiet`). A line printed directly
//! instead skips that seam, and a print panics on a write error — a stdout that
//! is a closed pipe, for one — which in the window takes down that network's
//! node. A console-less Windows process is *not* such an error: std treats a
//! null handle as a successful write. So the failure needs a particular stdout
//! to appear, and no test run here has one. What is checkable everywhere is the
//! shape of the code, which is what this reads.
//!
//! Four lines in `serve.rs` went round the seam until 2026-10-06 — a relay
//! reservation, a failed exclusion, a live broadcast and an epoch catch-up —
//! and `join::redeem` had a second, ad-hoc switch for its own. This keeps it
//! from happening again.
//!
//! # What it can and cannot see
//!
//! It reads every `.rs` file under this crate's `src/` (subdirectories
//! included) except `main.rs`, the `kols` binary — the test harness, which
//! prints by design — and the window shell's `kols-app/src/main.rs`. Each print
//! is attributed to the top-level item it sits in: a top-level `fn` by name,
//! anything else unindented (`impl`, `mod`, `struct`, a `const`, a macro) as
//! `<not a function>`, so a print in an `impl` placed after an allowed function
//! is not allowed with it. Three functions may print, all serving only the
//! harness: `lib.rs`'s `printing_report`, `serve.rs`'s `render` (the printing
//! sink `kols serve` uses) and `join.rs`'s `run` (`kols join`).
//!
//! "Prints" means one of the print macros, or naming `io::stdout()` or
//! `io::stderr()` at all. A write reached some other way — a raw handle, a
//! dependency that prints — is not seen, and a function nested inside an
//! allowed one is allowed with it. A `//` inside a string literal ends the line
//! early for this reader. A guard against the mistake that was made rather than
//! a proof.

use std::path::{Path, PathBuf};

const ALLOWED: &[(&str, &str)] = &[
    ("lib.rs", "printing_report"),
    ("serve.rs", "render"),
    ("join.rs", "run"),
];

const PRINTS: &[&str] = &[
    "println!",
    "print!",
    "eprintln!",
    "eprint!",
    "dbg!",
    "stdout()",
    "stderr()",
];

/// What a top-level line opens: a function by name, another item, or nothing.
enum Opens<'a> {
    Function(&'a str),
    OtherItem,
    Nothing,
}

fn opens(line: &str) -> Opens<'_> {
    // Indented lines, blank lines, comments, attributes and closing braces
    // continue whatever item is open.
    let starts = line.chars().next();
    if !starts.is_some_and(|c| c.is_alphabetic()) {
        return Opens::Nothing;
    }
    let mut rest = line;
    loop {
        let before = rest;
        for qualifier in [
            "pub(crate) ",
            "pub(super) ",
            "pub ",
            "async ",
            "const ",
            "unsafe ",
        ] {
            rest = rest.strip_prefix(qualifier).unwrap_or(rest);
        }
        if let Some(after) = rest.strip_prefix("extern ") {
            rest = after.trim_start_matches(|c| c != ' ').trim_start();
        }
        if rest == before {
            break;
        }
    }
    match rest.strip_prefix("fn ") {
        Some(name) => Opens::Function(
            name.split(|c: char| !(c.is_alphanumeric() || c == '_'))
                .next()
                .unwrap_or(""),
        ),
        None => Opens::OtherItem,
    }
}

fn sources(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).expect("a readable source directory") {
        let path = entry.expect("an entry").path();
        if path.is_dir() {
            sources(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// Every direct print in `files`, as (file name, line number, enclosing item).
fn prints_in(files: &[PathBuf]) -> Vec<(String, usize, String)> {
    let mut found = Vec::new();
    for path in files {
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let text = std::fs::read_to_string(path).expect("a source file");
        let mut current = String::from("<file scope>");
        for (n, line) in text.lines().enumerate() {
            match opens(line) {
                Opens::Function(f) => current = f.to_owned(),
                Opens::OtherItem => current = String::from("<not a function>"),
                Opens::Nothing => {}
            }
            let code = line.split("//").next().unwrap_or("");
            // `print!` is a suffix of `println!` and `eprint!`, so match each
            // as a whole word: the character before it must not be part of an
            // identifier.
            let printing = PRINTS.iter().any(|p| {
                code.match_indices(p).any(|(at, _)| {
                    !code[..at]
                        .chars()
                        .next_back()
                        .is_some_and(|c| c.is_alphanumeric() || c == '_')
                })
            });
            if printing {
                found.push((name.clone(), n + 1, current.clone()));
            }
        }
    }
    found
}

/// What the window runs: this library and the window shell.
fn window_sources() -> Vec<PathBuf> {
    let crate_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let src = crate_dir.join("src");
    let mut files = Vec::new();
    sources(&src, &mut files);
    files.retain(|path| path != &src.join("main.rs"));
    files.push(crate_dir.join("../kols-app/src/main.rs"));
    files.sort();
    assert!(
        files.len() > 5,
        "found too few sources to be reading the right directory"
    );
    files
}

#[test]
fn nothing_the_window_runs_prints_directly() {
    let stray: Vec<_> = prints_in(&window_sources())
        .into_iter()
        .filter(|(file, _, function)| {
            !ALLOWED
                .iter()
                .any(|(f, func)| f == file && func == function)
        })
        .collect();
    assert!(
        stray.is_empty(),
        "these print directly rather than through `kols_node::Report`, so the window's \
         nodes would run them — pass a `&Report` and use `crate::say!` instead:\n{}",
        stray
            .iter()
            .map(|(file, line, function)| format!("  src/{file}:{line} in `{function}`"))
            .collect::<Vec<_>>()
            .join("\n")
    );
}

#[test]
fn the_allowed_printers_still_exist() {
    // If one is renamed, its prints would be reported under a name the list
    // does not know — which is the test failing correctly — but a list entry
    // naming nothing would silently allow a future function of that name.
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    for (file, function) in ALLOWED {
        let text = std::fs::read_to_string(src.join(file)).expect("a source file");
        assert!(
            text.lines()
                .any(|l| matches!(opens(l), Opens::Function(f) if f == *function)),
            "src/{file} no longer has a top-level `fn {function}`; update ALLOWED"
        );
    }
}
