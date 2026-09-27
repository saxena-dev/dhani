//! Fixture loader: envelope unwrap of upstream fixtures, raw byte access and manifest lookup.
//!
//! Every path resolves from `CARGO_MANIFEST_DIR`, never from the working directory. Upstream
//! fixtures are read in place from the pinned `vendor/DhanHQ-py` submodule and never copied. A missing
//! file is a setup failure that panics with the path; nothing is downloaded or substituted.

use std::path::{Path, PathBuf};

const UPSTREAM_DIR: &str = "vendor/DhanHQ-py/tests/data";
const SYNTH_DIR: &str = "tests/fixtures/synth";
const MANIFEST: &str = "tests/fixtures/MANIFEST.toml";

fn root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

fn read(path: &Path) -> Vec<u8> {
    std::fs::read(path).unwrap_or_else(|e| panic!("cannot read fixture {}: {e}", path.display()))
}

/// The server payload of an upstream fixture: the file's `data` member, re-serialised. This is
/// the same JSON value the server sends; whitespace and key order may differ, so never assert
/// on its raw bytes.
pub fn upstream_payload(name: &str) -> Vec<u8> {
    let dir = root().join(UPSTREAM_DIR);
    assert!(
        dir.is_dir(),
        "{} is missing; run `git submodule update --init vendor/DhanHQ-py`",
        dir.display()
    );
    let path = dir.join(name);
    let value: serde_json::Value = serde_json::from_slice(&read(&path))
        .unwrap_or_else(|e| panic!("{} is not JSON: {e}", path.display()));
    let data = value
        .get("data")
        .unwrap_or_else(|| panic!("{} has no `data` member", path.display()));
    serde_json::to_vec(data).expect("a JSON value re-serialises")
}

/// A synthesised fixture's bytes, unchanged.
pub fn synth(name: &str) -> Vec<u8> {
    read(&root().join(SYNTH_DIR).join(name))
}

/// Any fixture's bytes, unchanged, by repository-relative path (for byte-exact fixtures such as
/// feed binaries and the order-update sample with its duplicate key).
pub fn raw_bytes(path: &str) -> Vec<u8> {
    read(&root().join(path))
}

/// One `[[fixture]]` entry of the manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FixtureEntry {
    pub path: String,
    pub class: String,
    pub origin: String,
    pub sha256: String,
    pub bytes: u64,
    pub endpoint: String,
}

/// The manifest entry for a repository-relative fixture path.
pub fn manifest_entry(path: &str) -> FixtureEntry {
    let file = root().join(MANIFEST);
    let text = String::from_utf8(read(&file)).expect("the manifest is UTF-8");
    let table: toml::Table = text.parse().expect("the manifest is TOML");
    let entries = table
        .get("fixture")
        .and_then(toml::Value::as_array)
        .cloned()
        .unwrap_or_default();
    let entry = entries
        .iter()
        .find(|e| e.get("path").and_then(toml::Value::as_str) == Some(path))
        .unwrap_or_else(|| panic!("{path} is not listed in {MANIFEST}"));
    let text = |key: &str| {
        entry
            .get(key)
            .and_then(toml::Value::as_str)
            .unwrap_or_else(|| panic!("{path}: no `{key}`"))
            .to_owned()
    };
    FixtureEntry {
        path: text("path"),
        class: text("class"),
        origin: text("origin"),
        sha256: text("sha256"),
        bytes: entry
            .get("bytes")
            .and_then(toml::Value::as_integer)
            .and_then(|n| u64::try_from(n).ok())
            .unwrap_or_else(|| panic!("{path}: no integer `bytes`")),
        endpoint: text("endpoint"),
    }
}

/// The directory holding the upstream fixtures.
pub fn upstream_dir() -> PathBuf {
    root().join(UPSTREAM_DIR)
}
