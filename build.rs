// SPDX-License-Identifier: MIT

//! Writes down which tests there are, for the documentation to show.
//!
//! A test here is named for what it holds to be so
//! (`a_swept_note_is_asked_of_the_relays_again`), so the names, read one
//! after the other, are an account of what the code is held to. This finds
//! every test under `src/`, by its `#[test]`, and writes two pieces of
//! Markdown to `OUT_DIR`, which `src/testing.rs` and `src/lib.rs` take into
//! their docs:
//!
//! - `test_summary.md`: how many tests there are, as a sentence;
//! - `test_inventory.md`: how many each module has, and each module's tests
//!   by name, as sentences.
//!
//! Nothing is run to find them: the sources are read as text, which is all
//! a build script can count on. A test written some other way than
//! `#[test]` over an `fn` is not found.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::{env, fs, io};

/// One test: what it is called, and whether it only runs when asked for.
struct Test {
    name: String,
    ignored: bool,
}

/// Every `.rs` file under `directory`, however deep, in order.
fn sources(directory: &Path) -> io::Result<Vec<PathBuf>> {
    let mut found = Vec::new();
    let mut entries = fs::read_dir(directory)?.collect::<Result<Vec<_>, _>>()?;
    entries.sort_by_key(fs::DirEntry::path);
    for entry in entries {
        let path = entry.path();
        if path.is_dir() {
            found.extend(sources(&path)?);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            found.push(path);
        }
    }
    Ok(found)
}

/// The tests in `text`, a source file: each `fn` under a `#[test]`, with
/// whatever other attributes stand between the two.
fn tests_in(text: &str) -> Vec<Test> {
    let mut found = Vec::new();
    // Whether a `#[test]` was read and its `fn` not yet, and whether an
    // `#[ignore]` was among its attributes.
    let mut awaited: Option<bool> = None;
    for line in text.lines().map(str::trim) {
        if line == "#[test]" || line.ends_with("::test]") {
            awaited = Some(false);
        } else if let Some(ignored) = awaited {
            if line.starts_with("#[ignore") {
                awaited = Some(true);
            } else if let Some(name) = line.strip_prefix("fn ").or_else(|| line.strip_prefix("async fn ")) {
                let name: String = name.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
                found.push(Test { name, ignored });
                awaited = None;
            } else if !line.starts_with("#[") && !line.starts_with("//") {
                awaited = None;
            }
        }
    }
    found
}

/// The module the source file at `path`, under `src`, is: `nostr::client`
/// for `src/nostr/client.rs`, `nostr` for `src/nostr/mod.rs`.
fn module_of(path: &Path, src: &Path) -> String {
    let within = path.strip_prefix(src).unwrap_or(path).with_extension("");
    let mut parts: Vec<String> = within.iter().map(|part| part.to_string_lossy().into_owned()).collect();
    if parts.last().is_some_and(|last| last == "mod" || last == "lib") {
        parts.pop();
    }
    parts.join("::")
}

/// A test's name as the sentence it was written as.
fn sentence(name: &str) -> String {
    let words = name.replace('_', " ");
    let mut letters = words.chars();
    match letters.next() {
        Some(first) => first.to_uppercase().chain(letters).collect(),
        None => words,
    }
}

fn main() -> io::Result<()> {
    let root = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap_or_default());
    let src = root.join("src");
    let out = PathBuf::from(env::var_os("OUT_DIR").unwrap_or_default());
    println!("cargo:rerun-if-changed=src");
    println!("cargo:rerun-if-changed=build.rs");

    let mut by_module: BTreeMap<String, Vec<Test>> = BTreeMap::new();
    for path in sources(&src)? {
        let tests = tests_in(&fs::read_to_string(&path)?);
        if !tests.is_empty() {
            by_module.entry(module_of(&path, &src)).or_default().extend(tests);
        }
    }
    let all = by_module.values().map(Vec::len).sum::<usize>();
    let ignored = by_module.values().flatten().filter(|test| test.ignored).count();

    let asked_for = if ignored == 0 { String::new() } else { format!(", {ignored} of which run only when asked for") };
    let summary = format!("**{all} tests** in {} modules{asked_for}.", by_module.len());
    fs::write(out.join("test_summary.md"), &summary)?;

    // Writing to a `String` can't fail.
    let mut inventory = String::new();
    let _ = writeln!(inventory, "{summary}\n");
    let _ = writeln!(inventory, "| Module | Tests |\n|---|---:|");
    for (module, tests) in &by_module {
        // Named as a module: some are called what a function in them is.
        let _ = writeln!(inventory, "| [`{module}`](mod@crate::{module}) | {} |", tests.len());
    }
    for (module, tests) in &by_module {
        let _ = writeln!(inventory, "\n### `{module}`\n");
        for test in tests {
            let mark = if test.ignored { " *(only when asked for)*" } else { "" };
            let _ = writeln!(inventory, "- {}{mark}", sentence(&test.name));
        }
    }
    fs::write(out.join("test_inventory.md"), inventory)
}
