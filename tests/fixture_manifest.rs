//! `tests/fixtures/MANIFEST.toml` lists every fixture file, and every listed file is intact.
//!
//! Checks: each `[[fixture]]` file exists with its recorded sha256 and byte length; every file
//! under `tests/fixtures/` other than the manifest is listed; the `vendor/DhanHQ-py` submodule is
//! checked out at the pinned commit (skipped with a message when the tree is not a git
//! checkout); every MVP endpoint row has a fixture, and a contract test in a `tests/rest_*.rs`
//! file marked with a `// row: <id>` comment.

mod support;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use sha2::{Digest, Sha256};

const ROOT: &str = env!("CARGO_MANIFEST_DIR");
const MANIFEST: &str = "tests/fixtures/MANIFEST.toml";
const FIXTURES_DIR: &str = "tests/fixtures";
const SUBMODULE: &str = "vendor/DhanHQ-py";
const PINNED_COMMIT: &str = "63b9030d700f0fd331c124fd948ba0b79a6e7fd8";
const CLASSES: &[&str] = &["upstream", "synthesized", "derived", "captured"];

/// One `[[fixture]]` entry.
#[derive(Debug)]
struct Fixture {
    path: String,
    sha256: String,
    bytes: u64,
    /// Endpoint-matrix row ids served by this fixture.
    endpoints: Vec<String>,
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
            let endpoints = text("endpoint")?
                .split(',')
                .map(|e| e.trim().to_owned())
                .collect();
            let bytes = e
                .get("bytes")
                .and_then(toml::Value::as_integer)
                .and_then(|n| u64::try_from(n).ok())
                .ok_or(format!("{path}: `bytes` missing or negative"))?;
            Ok(Fixture {
                path,
                sha256: text("sha256")?,
                bytes,
                endpoints,
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

/// The MVP rows of the REST endpoint matrix with the fixtures it names for each, matched by
/// file basename against the manifest paths. An empty list would mark a row whose fixture
/// column says "missing"; no MVP row is missing.
const MVP_ROWS: [(&str, &[&str]); 30] = [
    ("A5", &["auth_issued_token.json"]),
    ("A6", &["auth_issued_token.json"]),
    ("A7", &["profile.json"]),
    ("O1", &["place_order.json"]),
    ("O2", &["place_slice_order.json"]),
    ("O3", &["modify_pending_order.json"]),
    ("O4", &["cancel_given_order.json"]),
    ("O5", &["get-current-orders-list.json"]),
    ("O6", &["get-order-by-id.json"]),
    ("O7", &["get-order-by-correlation-id.json"]),
    ("O8", &["get_all_trades.json"]),
    ("O9", &["get_trade_book_by_orderid.json"]),
    ("P1", &["get-current-holdings.json"]),
    ("P2", &["get_positions.json"]),
    ("P3", &["convert_position.json", "empty"]),
    ("P4", &["empty"]),
    ("M1", &["get_fund_limits.json"]),
    ("M2", &["margin_calculator.json"]),
    ("M3", &["multi_margin.json"]),
    ("T1", &["get_ledger_report.json"]),
    ("T2", &["get_trade_history.json"]),
    ("Q1", &["quote_ltp.json"]),
    ("Q2", &["quote_ohlc.json"]),
    ("Q3", &["quote_full.json"]),
    ("H1", &["historical_daily_data.json", "candles.json"]),
    ("H2", &["intraday_minute_data.json", "candles.json"]),
    ("X1", &["option_chain_data.json"]),
    ("X2", &["expiry_list.json"]),
    ("I1", &["security_list.json", "scrip_master_compact.csv"]),
    ("I2", &["scrip_master_detailed.csv"]),
];

/// Problems with MVP-row coverage: a row with no fixture, or a named fixture that is not in the
/// manifest or does not list the row.
fn mvp_coverage(fixtures: &[Fixture], rows: &[(&str, &[&str])]) -> Vec<String> {
    let mut problems = Vec::new();
    for &(row, names) in rows {
        if names.is_empty() {
            continue;
        }
        if !fixtures
            .iter()
            .any(|f| f.endpoints.iter().any(|e| e == row))
        {
            problems.push(format!("MVP row {row} has no fixture in the manifest"));
        }
        for name in names {
            let listed = fixtures
                .iter()
                .find(|f| f.path.rsplit('/').next() == Some(name));
            match listed {
                None => problems.push(format!(
                    "MVP row {row}: fixture {name} is not in the manifest"
                )),
                Some(f) if !f.endpoints.iter().any(|e| e == row) => problems.push(format!(
                    "MVP row {row}: fixture {name} does not list the row"
                )),
                Some(_) => {}
            }
        }
    }
    problems
}

#[test]
fn every_mvp_row_has_a_fixture() {
    let fixtures = load();
    let problems = mvp_coverage(&fixtures, &MVP_ROWS);
    let missing = MVP_ROWS
        .iter()
        .filter(|(_, names)| names.is_empty())
        .count();
    println!(
        "MVP-row check: {} rows, {missing} missing, {} problems",
        MVP_ROWS.len(),
        problems.len()
    );
    assert!(
        problems.is_empty(),
        "MVP coverage problems:\n  {}",
        problems.join("\n  ")
    );
}

/// Every endpoint-matrix row id a marker may name: a prefix and its highest number.
const ROW_IDS: &[(&str, u32)] = &[
    ("A", 10),
    ("O", 9),
    ("S", 4),
    ("F", 4),
    ("C", 6),
    ("P", 4),
    ("M", 3),
    ("T", 2),
    ("K", 5),
    ("E", 4),
    ("Q", 3),
    ("H", 3),
    ("X", 2),
    ("I", 4),
    ("G", 12),
];

fn is_row_id(id: &str) -> bool {
    ROW_IDS.iter().any(|&(prefix, max)| {
        id.strip_prefix(prefix)
            .filter(|n| !n.starts_with('0'))
            .and_then(|n| n.parse::<u32>().ok())
            .is_some_and(|n| (1..=max).contains(&n))
    })
}

/// The row ids named by `// row: <id>` markers in `text`. A marker must name a known row and sit
/// directly above a `#[test]` or `#[tokio::test]` function named after it (`<id>_…`, lowercase),
/// with only attributes between; a marker above an `#[ignore]` or `#[cfg(any())]` test does not
/// count. Each misplaced or unknown marker is an error.
fn row_markers(text: &str) -> (Vec<String>, Vec<String>) {
    let lines: Vec<&str> = text.lines().map(str::trim).collect();
    let (mut rows, mut errors) = (Vec::new(), Vec::new());
    for (i, line) in lines.iter().enumerate() {
        let Some(id) = line.strip_prefix("// row: ").map(str::trim) else {
            continue;
        };
        if !is_row_id(id) {
            errors.push(format!("marker names no endpoint row: {line:?}"));
            continue;
        }
        let rest = &lines[i + 1..];
        let attrs = rest.iter().take_while(|l| l.starts_with("#[")).count();
        let ignored = rest[..attrs]
            .iter()
            .any(|l| l.starts_with("#[ignore") || l.starts_with("#[cfg(any())"));
        let is_test = rest[..attrs]
            .iter()
            .any(|l| *l == "#[test]" || l.starts_with("#[tokio::test"));
        let test = format!("fn {}_", id.to_lowercase());
        let named = rest
            .get(attrs)
            .is_some_and(|l| l.strip_prefix("async ").unwrap_or(l).starts_with(&test));
        if ignored || !is_test || !named {
            errors.push(format!(
                "marker {id} is not directly above an active {test}… test"
            ));
            continue;
        }
        rows.push(id.to_owned());
    }
    (rows, errors)
}

/// The MVP rows of `rows` that no marker in `marked` names.
fn unmarked<'a>(rows: &[(&'a str, &[&str])], marked: &[String]) -> Vec<&'a str> {
    rows.iter()
        .map(|&(row, _)| row)
        .filter(|row| !marked.iter().any(|m| m == row))
        .collect()
}

#[test]
fn every_mvp_row_has_a_contract_test() {
    let mut sources = Vec::new();
    for entry in fs::read_dir(Path::new(ROOT).join("tests")).expect("read tests/") {
        let path = entry.expect("a directory entry").path();
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if name.starts_with("rest_") && name.ends_with(".rs") {
            sources.push(fs::read_to_string(&path).expect("read a contract test file"));
        }
    }
    let (mut marked, mut errors) = (Vec::new(), Vec::new());
    for source in &sources {
        let (rows, problems) = row_markers(source);
        marked.extend(rows);
        errors.extend(problems);
    }
    assert!(
        errors.is_empty(),
        "bad row markers:\n  {}",
        errors.join("\n  ")
    );
    let missing = unmarked(&MVP_ROWS, &marked);
    println!(
        "contract-test check: {} MVP rows, {} markers",
        MVP_ROWS.len(),
        marked.len()
    );
    assert!(
        missing.is_empty(),
        "MVP rows without a `// row: <id>` contract test in tests/rest_*.rs: {missing:?}"
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
    fn mvp_coverage_reports_gaps() {
        let fixtures =
            parse_manifest(&ENTRY.replace("endpoint = \"O1\"", "endpoint = \"O1, O2\"")).unwrap();
        assert_eq!(fixtures[0].endpoints, ["O1", "O2"]);
        let rows: [(&str, &[&str]); 4] = [
            ("O1", &["a.json"]),
            ("O2", &["a.json"]),
            ("O3", &[]),
            ("O4", &["b.json"]),
        ];
        assert_eq!(
            mvp_coverage(&fixtures, &rows),
            [
                "MVP row O4 has no fixture in the manifest",
                "MVP row O4: fixture b.json is not in the manifest",
            ]
        );
        let rows: [(&str, &[&str]); 1] = [("O5", &["a.json"])];
        assert_eq!(
            mvp_coverage(&fixtures, &rows),
            [
                "MVP row O5 has no fixture in the manifest",
                "MVP row O5: fixture a.json does not list the row"
            ]
        );
    }

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

    #[test]
    fn row_markers_are_read_and_unmarked_rows_reported() {
        let text = "// row: O1\n#[tokio::test]\nasync fn o1_place() {}\n\
                    // row: O2\n#[test]\nfn o2_sliced() {}\n";
        let (marked, errors) = row_markers(text);
        assert_eq!(
            (marked.as_slice(), errors.len()),
            (&["O1".to_owned(), "O2".to_owned()][..], 0)
        );
        let rows: [(&str, &[&str]); 3] = [("O1", &[]), ("O2", &[]), ("O3", &[])];
        assert_eq!(unmarked(&rows, &marked), ["O3"]);
    }

    #[test]
    fn misplaced_ignored_and_unknown_markers_are_errors() {
        for text in [
            "// row: O1\nfn helper() {}\n",
            "// row: O1\n#[tokio::test]\nasync fn o2_list() {}\n",
            "// row: O1\n#[tokio::test]\n#[ignore]\nasync fn o1_place() {}\n",
            "// row: O10\n#[test]\nfn o10_x() {}\n",
            "// row: O01\n#[test]\nfn o01_x() {}\n",
            "// row: O1, O2\n#[test]\nfn o1_x() {}\n",
            "// row: O1\n",
            "// row: O1\nfn o1_helper() {}\n",
            "// row: O1\n#[allow(dead_code)]\nfn o1_helper() {}\n",
            "// row: O1\n#[cfg(any())]\n#[test]\nfn o1_x() {}\n",
        ] {
            let (marked, errors) = row_markers(text);
            assert!(
                marked.is_empty() && errors.len() == 1,
                "{text:?}: {errors:?}"
            );
        }
        assert!(is_row_id("G12") && is_row_id("A10") && !is_row_id("W1") && !is_row_id("K0"));
    }
}
