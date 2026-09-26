//! `tests/fixtures/MANIFEST.toml` lists every fixture file, and every listed file is intact.
//!
//! Checks: each `[[fixture]]` file exists with its recorded sha256 and byte length; every file
//! under `tests/fixtures/` other than the manifest is listed; the `DhanHQ-py` submodule is
//! checked out at the pinned commit (skipped with a message when the tree is not a git
//! checkout).

mod support;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use sha2::{Digest, Sha256};

const ROOT: &str = env!("CARGO_MANIFEST_DIR");
const MANIFEST: &str = "tests/fixtures/MANIFEST.toml";
const FIXTURES_DIR: &str = "tests/fixtures";
const SUBMODULE: &str = "DhanHQ-py";
const PINNED_COMMIT: &str = "63b9030d700f0fd331c124fd948ba0b79a6e7fd8";
const CLASSES: &[&str] = &["upstream", "synthesized", "derived", "captured"];

/// One `[[fixture]]` entry.
#[derive(Debug)]
struct Fixture {
    path: String,
    sha256: String,
    bytes: u64,
}

fn parse_manifest(text: &str) -> Result<Vec<Fixture>, String> {
    let table: toml::Table = text.parse().map_err(|e| format!("invalid TOML: {e}"))?;
    let Some(entries) = table.get("fixture") else {
        return Ok(Vec::new());
    };
    let entries = entries
        .as_array()
        .ok_or("`fixture` must be an array of tables")?;
    let fixtures = entries
        .iter()
        .map(|e| {
            let text = |key: &str| {
                e.get(key)
                    .and_then(toml::Value::as_str)
                    .map(str::to_owned)
                    .ok_or(format!("[[fixture]] without a string `{key}`: {e}"))
            };
            let path = text("path")?;
            // Paths are compared as strings, so only one spelling of each path is accepted.
            let canonical = !path.starts_with('/')
                && !path.contains('\\')
                && path.split('/').all(|c| !c.is_empty() && c != "." && c != "..");
            if !canonical {
                return Err(format!(
                    "{path:?}: path must be repository-relative and `/`-separated, without empty, `.` or `..` components"
                ));
            }
            let class = text("class")?;
            if !CLASSES.contains(&class.as_str()) {
                return Err(format!("{path}: unknown class {class:?}"));
            }
            text("origin")?;
            text("endpoint")?;
            let bytes = e
                .get("bytes")
                .and_then(toml::Value::as_integer)
                .and_then(|n| u64::try_from(n).ok())
                .ok_or(format!("{path}: `bytes` missing or negative"))?;
            Ok(Fixture {
                path,
                sha256: text("sha256")?,
                bytes,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    for (i, f) in fixtures.iter().enumerate() {
        if fixtures[..i].iter().any(|g| g.path == f.path) {
            return Err(format!("{}: listed twice", f.path));
        }
    }
    Ok(fixtures)
}

/// Problems with one listed file's content: size and sha256 against the manifest.
fn check_content(f: &Fixture, data: &[u8]) -> Option<String> {
    let sha = hex::encode(Sha256::digest(data));
    (data.len() as u64 != f.bytes || sha != f.sha256).then(|| {
        format!(
            "{}: {} bytes, sha256 {sha} (manifest: {} bytes, sha256 {})",
            f.path,
            data.len(),
            f.bytes,
            f.sha256
        )
    })
}

/// Files present on disk but absent from the manifest.
fn unlisted<'a>(listed: &[Fixture], found: &'a [String]) -> Vec<&'a str> {
    found
        .iter()
        .filter(|p| !listed.iter().any(|f| &f.path == *p))
        .map(String::as_str)
        .collect()
}

fn files_under(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(entries) = fs::read_dir(dir) else {
        return out;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if entry.file_type().is_ok_and(|t| t.is_dir()) {
            out.extend(files_under(&path));
        } else {
            out.push(path);
        }
    }
    out.sort();
    out
}

/// Repository-relative, `/`-separated path.
fn relative(path: &Path) -> String {
    let rel = path.strip_prefix(ROOT).unwrap_or(path);
    rel.components()
        .map(|c| c.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

fn load() -> Vec<Fixture> {
    let text =
        fs::read_to_string(Path::new(ROOT).join(MANIFEST)).expect("read the fixture manifest");
    parse_manifest(&text).unwrap_or_else(|e| panic!("{MANIFEST}: {e}"))
}

#[test]
fn every_listed_fixture_exists_and_matches() {
    let problems: Vec<String> = load()
        .iter()
        .filter_map(|f| match fs::read(Path::new(ROOT).join(&f.path)) {
            Ok(data) => check_content(f, &data),
            Err(e) => Some(format!("{}: {e}", f.path)),
        })
        .collect();
    assert!(
        problems.is_empty(),
        "fixture problems:\n  {}",
        problems.join("\n  ")
    );
}

#[test]
fn every_fixture_file_is_listed() {
    let found: Vec<String> = files_under(&Path::new(ROOT).join(FIXTURES_DIR))
        .iter()
        .map(|p| relative(p))
        .filter(|p| p != MANIFEST)
        .collect();
    let missing = unlisted(&load(), &found);
    assert!(
        missing.is_empty(),
        "files under {FIXTURES_DIR}/ missing from {MANIFEST}: {missing:?}"
    );
}

#[test]
fn submodule_is_at_the_pinned_commit() {
    let root = Path::new(ROOT);
    if !root.join(".git").exists() {
        println!("skipped: {ROOT} is not a git checkout, so the {SUBMODULE} pin cannot be checked");
        return;
    }
    let sub = root.join(SUBMODULE);
    // An uninitialised submodule directory would make git report the superproject instead.
    assert!(
        sub.join(".git").exists(),
        "{SUBMODULE} is not checked out; run `git submodule update --init {SUBMODULE}`"
    );
    let git = |args: &[&str]| {
        // Inherited GIT_* variables (set inside git hooks) would redirect git elsewhere.
        let out = Command::new("git")
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .env_remove("GIT_INDEX_FILE")
            .arg("-C")
            .arg(&sub)
            .args(args)
            .output()
            .expect("run git");
        assert!(
            out.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout)
            .expect("git output is UTF-8")
            .trim()
            .to_owned()
    };
    let top = fs::canonicalize(git(&["rev-parse", "--show-toplevel"])).expect("canonical toplevel");
    assert_eq!(
        top,
        fs::canonicalize(&sub).expect("canonical submodule path")
    );
    assert_eq!(
        git(&["rev-parse", "HEAD"]),
        PINNED_COMMIT,
        "{SUBMODULE} HEAD moved off the pin"
    );
}

/// Checker self-tests on synthetic input.
mod checker {
    use super::*;

    const ENTRY: &str = r#"
        [[fixture]]
        path = "tests/fixtures/synth/a.json"
        class = "synthesized"
        origin = "DOC table"
        sha256 = "2c26b46b68ffc68ff99b453c1d30413413422d706483bfa0f98a5e886266e7ae"
        bytes = 3
        endpoint = "O1"
    "#;

    #[test]
    fn empty_manifest_has_no_fixtures() {
        assert!(parse_manifest("# header only\n").unwrap().is_empty());
    }

    #[test]
    fn content_mismatches_are_reported() {
        let f = &parse_manifest(ENTRY).unwrap()[0];
        assert_eq!(check_content(f, b"foo"), None);
        let bad = check_content(f, b"fob").expect("changed content is reported");
        assert!(
            bad.starts_with("tests/fixtures/synth/a.json: 3 bytes, sha256 "),
            "{bad}"
        );
        assert!(check_content(f, b"fooo").is_some());
    }

    #[test]
    fn unlisted_files_are_reported() {
        let listed = parse_manifest(ENTRY).unwrap();
        let found = vec![
            "tests/fixtures/synth/a.json".to_owned(),
            "tests/fixtures/synth/b.json".to_owned(),
        ];
        assert_eq!(
            unlisted(&listed, &found),
            vec!["tests/fixtures/synth/b.json"]
        );
        assert_eq!(unlisted(&[], &found).len(), 2);
    }

    #[test]
    fn malformed_entries_are_rejected() {
        assert!(parse_manifest(&ENTRY.replace("synthesized", "invented")).is_err());
        assert!(parse_manifest(&ENTRY.replace("bytes = 3", "bytes = -3")).is_err());
        assert!(parse_manifest(&ENTRY.replace("endpoint = \"O1\"", "")).is_err());
        for bad in [
            "/abs/a.json",
            "./tests/a.json",
            "tests/../a.json",
            "tests//a.json",
            "tests\\\\a.json",
            "",
        ] {
            let entry = ENTRY.replace("tests/fixtures/synth/a.json", bad);
            assert!(parse_manifest(&entry).is_err(), "{bad:?} accepted");
        }
        let twice = parse_manifest(&format!("{ENTRY}{ENTRY}")).unwrap_err();
        assert!(twice.ends_with("listed twice"), "{twice}");
    }
}
