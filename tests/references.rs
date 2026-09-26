//! Offline, structural checks of upstream citations against `docs/dhan-sources.toml`.
//!
//! Every `DOC:`, `OAS:` and `LEGACY:` citation under `src/`, `tests/` and `docs/`, and every
//! `Endpoint.doc` string, must resolve against the source manifest. The checks never read a
//! Dhan file, hash anything or touch the network, so a change to Dhan's published docs can
//! never break the build; `scripts/verify-dhan-sources.sh` detects such drift manually.
//!
//! A citation is recognised only where its prefix is followed by the start of a value: a digit
//! after `DOC:`, a letter or `#` after `OAS:`, a letter after `LEGACY:`. A prefix followed by
//! anything else (`<`, a backtick, whitespace) is prose describing the citation syntax.

mod support;

use std::fs;
use std::path::{Path, PathBuf};

const ROOT: &str = env!("CARGO_MANIFEST_DIR");
const MANIFEST: &str = "docs/dhan-sources.toml";
const PAGE_URL_PREFIX: &str = "https://docs.dhanhq.co/markdown/";
/// Directories scanned for citations.
const CITATION_DIRS: &[&str] = &["src", "tests", "docs"];
/// Paths that must never name the local-only upstream mirror.
const MIRROR_FREE_DIRS: &[&str] = &["src", "tests", "docs", "examples", "benches"];
const MIRROR_FREE_FILES: &[&str] = &["README.md", "CHANGELOG.md"];
/// Built by concatenation so that this file does not contain the needle it searches for.
const MIRROR_NEEDLE: &str = concat!(".ignore", "/");
const OAS_METHODS: &[&str] = &["GET", "POST", "PUT", "DELETE"];
const OAS_SCHEMA_PREFIX: &str = "#/components/schemas/";

/// One parsed citation.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Citation {
    /// `DOC:<a>` or `DOC:<a>-<b>`, 1-based inclusive export lines.
    Doc(u64, u64),
    /// `OAS:<METHOD> <path>` or `OAS:#/components/schemas/<Name>`.
    Oas(String),
    /// `LEGACY:<name>`.
    Legacy(String),
}

/// A `[[page]]` entry: name, url and inclusive `export_lines` ranges.
type Page = (String, String, Vec<(u64, u64)>);

/// The parts of the source manifest the structural checks need.
#[derive(Debug)]
struct Manifest {
    snapshot_lines: u64,
    /// `(page name, url, export_lines ranges)`.
    pages: Vec<Page>,
    legacy: Vec<String>,
}

fn load_manifest() -> Manifest {
    let text =
        fs::read_to_string(Path::new(ROOT).join(MANIFEST)).expect("read the source manifest");
    parse_manifest(&text).unwrap_or_else(|e| panic!("{MANIFEST}: {e}"))
}

fn parse_manifest(text: &str) -> Result<Manifest, String> {
    let table: toml::Table = text.parse().map_err(|e| format!("invalid TOML: {e}"))?;
    let snapshot_lines = table
        .get("snapshot")
        .and_then(|s| s.get("lines"))
        .and_then(toml::Value::as_integer)
        .and_then(|n| u64::try_from(n).ok())
        .ok_or("[snapshot] lines missing or not a positive integer")?;
    let mut pages = Vec::new();
    for page in table
        .get("page")
        .and_then(toml::Value::as_array)
        .ok_or("no [[page]] entries")?
    {
        let field = |key: &str| {
            page.get(key)
                .and_then(toml::Value::as_str)
                .map(str::to_owned)
                .ok_or(format!("[[page]] without a string `{key}`: {page}"))
        };
        let name = field("name")?;
        let url = field("url")?;
        let ranges = page
            .get("export_lines")
            .and_then(toml::Value::as_array)
            .ok_or(format!("page {name}: export_lines missing"))?
            .iter()
            .map(|r| {
                r.as_str()
                    .and_then(parse_range)
                    .ok_or(format!("page {name}: bad export_lines entry {r}"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        pages.push((name, url, ranges));
    }
    let legacy = table
        .get("legacy")
        .and_then(toml::Value::as_array)
        .map(|entries| {
            entries
                .iter()
                .filter_map(|e| {
                    e.get("name")
                        .and_then(toml::Value::as_str)
                        .map(str::to_owned)
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(Manifest {
        snapshot_lines,
        pages,
        legacy,
    })
}

/// Parses `a` or `a-b` into an inclusive range.
fn parse_range(s: &str) -> Option<(u64, u64)> {
    match s.split_once('-') {
        Some((a, b)) => Some((a.parse().ok()?, b.parse().ok()?)),
        None => s.parse().ok().map(|a| (a, a)),
    }
}

/// Structural problems in the manifest itself: page URLs and `export_lines` layout.
fn check_manifest(m: &Manifest) -> Vec<String> {
    let mut problems = Vec::new();
    for (name, url, _) in &m.pages {
        if !url.starts_with(PAGE_URL_PREFIX) {
            problems.push(format!(
                "page {name}: url {url} does not start with {PAGE_URL_PREFIX}"
            ));
        }
    }
    let mut ranges: Vec<(u64, u64, &str)> = m
        .pages
        .iter()
        .flat_map(|(name, _, rs)| rs.iter().map(move |&(a, b)| (a, b, name.as_str())))
        .collect();
    ranges.sort_unstable();
    for &(a, b, name) in &ranges {
        if a == 0 || a > b || b > m.snapshot_lines {
            problems.push(format!(
                "page {name}: export_lines {a}-{b} outside 1..={}",
                m.snapshot_lines
            ));
        }
    }
    for pair in ranges.windows(2) {
        let ((_, prev_end, prev), (next_start, _, next)) = (pair[0], pair[1]);
        if next_start <= prev_end {
            problems.push(format!(
                "export_lines of {prev} and {next} overlap at line {next_start}"
            ));
        } else if next_start != prev_end + 1 {
            problems.push(format!(
                "export lines {}-{} between {prev} and {next} belong to no page",
                prev_end + 1,
                next_start - 1
            ));
        }
    }
    match ranges.last() {
        Some(&(_, end, name)) if end != m.snapshot_lines => problems.push(format!(
            "last page {name} ends at line {end}, but the snapshot has {} lines",
            m.snapshot_lines
        )),
        None => problems.push("no export_lines at all".to_owned()),
        _ => {}
    }
    problems
}

/// Problems with one citation, resolved against the manifest.
fn check_citation(m: &Manifest, c: &Citation) -> Option<String> {
    match c {
        Citation::Doc(a, b) => {
            if *a == 0 || a > b || *b > m.snapshot_lines {
                return Some(format!("DOC:{a}-{b} is outside 1..={}", m.snapshot_lines));
            }
            let first_uncovered = (*a..=*b).find(|line| {
                !m.pages
                    .iter()
                    .any(|(_, _, rs)| rs.iter().any(|&(s, e)| (s..=e).contains(line)))
            });
            first_uncovered.map(|line| format!("DOC:{a}-{b}: line {line} belongs to no [[page]]"))
        }
        Citation::Legacy(name) => (!m.legacy.iter().any(|l| l == name))
            .then(|| format!("LEGACY:{name} is not a [[legacy]] entry")),
        // Well-formedness is established by the parser; resolution against the spec is the
        // drift script's job, because this test never reads a Dhan file.
        Citation::Oas(_) => None,
    }
}

fn is_word_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// Length of the longest prefix of `s` whose bytes satisfy `f`.
fn span(s: &str, f: impl Fn(u8) -> bool) -> usize {
    s.bytes().position(|b| !f(b)).unwrap_or(s.len())
}

/// Parses the citation value after `prefix` (which includes the colon) at the start of `rest`.
/// Returns `Ok(None)` for prose, `Ok(Some((citation, consumed)))` for a citation, and `Err` for
/// a malformed one. A value running straight into a word character, a non-ASCII character or
/// (after a DOC range) another `-` is malformed rather than silently truncated.
fn parse_value(prefix: &str, rest: &str) -> Result<Option<(Citation, usize)>, String> {
    let Some((citation, n)) = parse_raw(prefix, rest)? else {
        return Ok(None);
    };
    match rest.as_bytes().get(n) {
        Some(&b) if is_word_byte(b) || !b.is_ascii() || (prefix == "DOC:" && b == b'-') => {
            let shown: String = rest
                .chars()
                .take_while(|c| !c.is_whitespace())
                .take(40)
                .collect();
            Err(format!("malformed citation `{prefix}{shown}`"))
        }
        _ => Ok(Some((citation, n))),
    }
}

fn parse_raw(prefix: &str, rest: &str) -> Result<Option<(Citation, usize)>, String> {
    let first = rest.bytes().next();
    match prefix {
        "DOC:" => {
            if rest.starts_with(' ') && rest.as_bytes().get(1).is_some_and(u8::is_ascii_digit) {
                let digits = 1 + span(&rest[1..], |b| b.is_ascii_digit());
                return Err(format!(
                    "malformed DOC citation `DOC:{}`: space after the colon",
                    &rest[..digits]
                ));
            }
            if !first.is_some_and(|b| b.is_ascii_digit()) {
                return if first.is_some_and(is_word_byte) {
                    Err(format!(
                        "malformed DOC citation `DOC:{}`",
                        &rest[..span(rest, is_word_byte)]
                    ))
                } else {
                    Ok(None)
                };
            }
            let n = span(rest, |b| b.is_ascii_digit());
            let a: u64 = rest[..n]
                .parse()
                .map_err(|e| format!("DOC:{}: {e}", &rest[..n]))?;
            let Some(upper) = rest[n..].strip_prefix('-') else {
                return Ok(Some((Citation::Doc(a, a), n)));
            };
            let m = span(upper, |b| b.is_ascii_digit());
            if m == 0 {
                return Ok(Some((Citation::Doc(a, a), n)));
            }
            let b: u64 = upper[..m]
                .parse()
                .map_err(|e| format!("DOC:{a}-{}: {e}", &upper[..m]))?;
            Ok(Some((Citation::Doc(a, b), n + 1 + m)))
        }
        "OAS:" => {
            if let Some(schema) = rest.strip_prefix(OAS_SCHEMA_PREFIX) {
                let n = span(schema, is_word_byte);
                if n == 0 {
                    // `OAS:#/components/schemas/<Name>` and similar are prose.
                    return Ok(None);
                }
                return Ok(Some((
                    Citation::Oas(format!("{OAS_SCHEMA_PREFIX}{}", &schema[..n])),
                    OAS_SCHEMA_PREFIX.len() + n,
                )));
            }
            if !first.is_some_and(|b| is_word_byte(b) || b == b'#') {
                return Ok(None);
            }
            let word = &rest[..span(rest, is_word_byte)];
            let path = rest[word.len()..]
                .strip_prefix(' ')
                .filter(|p| p.starts_with('/'));
            match (OAS_METHODS.contains(&word), path) {
                (true, Some(path)) => {
                    let n = span(path, |b| is_word_byte(b) || b"{}/.-".contains(&b));
                    let p = path[..n].trim_end_matches('.');
                    Ok(Some((
                        Citation::Oas(format!("{word} {p}")),
                        word.len() + 1 + p.len(),
                    )))
                }
                _ => Err(format!(
                    "malformed OAS citation `OAS:{}`: expected `OAS:<GET|POST|PUT|DELETE> /path` or `OAS:{OAS_SCHEMA_PREFIX}<Name>`",
                    rest.lines()
                        .next()
                        .unwrap_or("")
                        .chars()
                        .take(40)
                        .collect::<String>()
                )),
            }
        }
        "LEGACY:" => {
            if !first.is_some_and(is_word_byte) {
                return Ok(None);
            }
            let n = span(rest, |b| {
                b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-'
            });
            let name = rest[..n].trim_end_matches('-');
            if name.is_empty() || rest.as_bytes().get(n).is_some_and(|&b| is_word_byte(b)) {
                Err(format!(
                    "malformed LEGACY citation `LEGACY:{}`",
                    &rest[..span(rest, is_word_byte)]
                ))
            } else {
                Ok(Some((Citation::Legacy(name.to_owned()), name.len())))
            }
        }
        _ => unreachable!("unknown citation prefix {prefix}"),
    }
}

const PREFIXES: &[&str] = &["DOC:", "OAS:", "LEGACY:"];

/// Every citation in `text`, with its 1-based line number; malformed ones are `Err`.
fn scan(text: &str) -> Vec<(usize, Result<Citation, String>)> {
    let mut found = Vec::new();
    for (lineno, line) in text.lines().enumerate() {
        for prefix in PREFIXES {
            for (at, _) in line.match_indices(prefix) {
                if at > 0 && is_word_byte(line.as_bytes()[at - 1]) {
                    continue;
                }
                match parse_value(prefix, &line[at + prefix.len()..]) {
                    Ok(Some((c, _))) => found.push((lineno + 1, Ok(c))),
                    Ok(None) => {}
                    Err(e) => found.push((lineno + 1, Err(e))),
                }
            }
        }
    }
    found
}

/// Parses an `Endpoint.doc` string: one or more citations separated by commas, semicolons or
/// whitespace, and nothing else.
fn parse_doc_string(s: &str) -> Result<Vec<Citation>, String> {
    let mut out = Vec::new();
    let mut rest = s;
    loop {
        rest = rest.trim_start_matches([' ', ',', ';']);
        if rest.is_empty() {
            break;
        }
        let prefix = PREFIXES
            .iter()
            .find(|p| rest.starts_with(**p))
            .ok_or_else(|| format!("doc string {s:?}: `{rest}` is not a citation"))?;
        match parse_value(prefix, &rest[prefix.len()..])? {
            Some((c, n)) => {
                out.push(c);
                rest = &rest[prefix.len() + n..];
            }
            None => return Err(format!("doc string {s:?}: `{prefix}` without a value")),
        }
        if !rest.is_empty() && !rest.starts_with([' ', ',', ';']) {
            return Err(format!(
                "doc string {s:?}: unexpected `{rest}` after a citation"
            ));
        }
    }
    if out.is_empty() {
        return Err(format!("doc string {s:?} contains no citation"));
    }
    Ok(out)
}

/// Every `doc: "…"` string literal in `text` (the `Endpoint.doc` field), with its line.
fn endpoint_doc_strings(text: &str) -> Vec<(usize, &str)> {
    let mut out = Vec::new();
    for (lineno, line) in text.lines().enumerate() {
        for (at, m) in line.match_indices("doc: \"") {
            if at > 0 && is_word_byte(line.as_bytes()[at - 1]) {
                continue;
            }
            let body = &line[at + m.len()..];
            if let Some(end) = body.find('"') {
                out.push((lineno + 1, &body[..end]));
            }
        }
    }
    out
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

fn text_files(dirs: &[&str]) -> Vec<(String, String)> {
    dirs.iter()
        .flat_map(|d| files_under(&Path::new(ROOT).join(d)))
        .filter_map(|p| {
            let text = fs::read_to_string(&p).ok()?;
            Some((
                p.strip_prefix(ROOT).unwrap_or(&p).display().to_string(),
                text,
            ))
        })
        .collect()
}

/// Whether `bytes` contain the local-only mirror's directory name.
fn names_mirror(bytes: &[u8]) -> bool {
    bytes
        .windows(MIRROR_NEEDLE.len())
        .any(|w| w == MIRROR_NEEDLE.as_bytes())
}

fn assert_no_problems(what: &str, problems: &[String]) {
    assert!(problems.is_empty(), "{what}:\n  {}", problems.join("\n  "));
}

#[test]
fn manifest_is_structurally_sound() {
    let m = load_manifest();
    assert!(!m.pages.is_empty(), "manifest lists no pages");
    assert_no_problems("source manifest problems", &check_manifest(&m));
}

#[test]
fn every_citation_resolves() {
    let m = load_manifest();
    let mut problems = Vec::new();
    for (path, text) in text_files(CITATION_DIRS) {
        for (line, c) in scan(&text) {
            match c.map(|c| check_citation(&m, &c)) {
                Ok(None) => {}
                Ok(Some(e)) | Err(e) => problems.push(format!("{path}:{line}: {e}")),
            }
        }
    }
    assert_no_problems("unresolved or malformed citations", &problems);
}

#[test]
fn every_endpoint_doc_string_parses_and_resolves() {
    let m = load_manifest();
    let mut problems = Vec::new();
    for (path, text) in text_files(&["src"]) {
        for (line, doc) in endpoint_doc_strings(&text) {
            match parse_doc_string(doc) {
                Ok(cites) => problems.extend(
                    cites
                        .iter()
                        .filter_map(|c| check_citation(&m, c))
                        .map(|e| format!("{path}:{line}: {e}")),
                ),
                Err(e) => problems.push(format!("{path}:{line}: {e}")),
            }
        }
    }
    assert_no_problems("bad Endpoint.doc strings", &problems);
}

#[test]
fn nothing_names_the_local_mirror() {
    let mut paths: Vec<PathBuf> = MIRROR_FREE_DIRS
        .iter()
        .flat_map(|d| files_under(&Path::new(ROOT).join(d)))
        .collect();
    paths.extend(
        MIRROR_FREE_FILES
            .iter()
            .map(|f| Path::new(ROOT).join(f))
            .filter(|p| p.is_file()),
    );
    let offenders: Vec<String> = paths
        .iter()
        .filter(|p| fs::read(p).is_ok_and(|bytes| names_mirror(&bytes)))
        .map(|p| p.strip_prefix(ROOT).unwrap_or(p).display().to_string())
        .collect();
    assert!(
        offenders.is_empty(),
        "files naming the local-only mirror `{MIRROR_NEEDLE}`: {offenders:?}"
    );
}

/// Checker self-tests on synthetic input. Citation strings are assembled at run time so that
/// the scan of this file does not see them.
mod checker {
    use super::*;

    fn cite(prefix: &str, value: &str) -> String {
        format!("{prefix}:{value}")
    }

    fn manifest() -> Manifest {
        parse_manifest(
            r#"
            [snapshot]
            lines = 30
            [[page]]
            name = "a"
            url = "https://docs.dhanhq.co/markdown/a.md"
            export_lines = ["5-10"]
            [[page]]
            name = "b"
            url = "https://docs.dhanhq.co/markdown/b.md"
            export_lines = ["11-30"]
            [[legacy]]
            name = "forever"
            "#,
        )
        .expect("synthetic manifest parses")
    }

    fn problems_in(text: &str) -> Vec<String> {
        let m = manifest();
        scan(text)
            .into_iter()
            .filter_map(|(_, c)| match c {
                Ok(c) => check_citation(&m, &c),
                Err(e) => Some(e),
            })
            .collect()
    }

    #[test]
    fn synthetic_manifest_is_sound() {
        assert_eq!(check_manifest(&manifest()), Vec::<String>::new());
    }

    #[test]
    fn scan_recognises_each_citation_form() {
        let text = format!(
            "see {} and {}; also {}, {} and {}.",
            cite("DOC", "12"),
            cite("DOC", "5-30"),
            cite("OAS", "GET /orders/{orderId}"),
            cite("OAS", "#/components/schemas/Order"),
            cite("LEGACY", "forever"),
        );
        let found: Vec<Citation> = scan(&text).into_iter().map(|(_, c)| c.unwrap()).collect();
        assert_eq!(
            found,
            vec![
                Citation::Doc(12, 12),
                Citation::Doc(5, 30),
                Citation::Oas("GET /orders/{orderId}".to_owned()),
                Citation::Oas("#/components/schemas/Order".to_owned()),
                Citation::Legacy("forever".to_owned()),
            ]
        );
        assert_eq!(problems_in(&text), Vec::<String>::new());
    }

    #[test]
    fn prose_mentions_are_not_citations() {
        let text = format!(
            "cite with `{}` or {} or {}",
            cite("DOC", "<line>"),
            cite("OAS", "<METHOD path>"),
            cite("LEGACY", " x")
        );
        assert_eq!(scan(&text), vec![]);
        assert_eq!(scan(&format!("X{}", cite("DOC", "99999"))), vec![]);
    }

    #[test]
    fn out_of_range_doc_line_is_rejected() {
        assert_eq!(
            problems_in(&cite("DOC", "99999")),
            vec![format!("{} is outside 1..=30", cite("DOC", "99999-99999"))]
        );
        assert_eq!(
            problems_in(&cite("DOC", "20-12")),
            vec![format!("{} is outside 1..=30", cite("DOC", "20-12"))]
        );
        assert_eq!(
            problems_in(&cite("DOC", "0")),
            vec![format!("{} is outside 1..=30", cite("DOC", "0-0"))]
        );
    }

    #[test]
    fn doc_line_outside_every_page_is_rejected() {
        assert_eq!(
            problems_in(&cite("DOC", "3-6")),
            vec![format!(
                "{}: line 3 belongs to no [[page]]",
                cite("DOC", "3-6")
            )]
        );
    }

    #[test]
    fn unknown_legacy_page_is_rejected() {
        assert_eq!(
            problems_in(&cite("LEGACY", "market-quote")),
            vec![format!(
                "{} is not a [[legacy]] entry",
                cite("LEGACY", "market-quote")
            )]
        );
    }

    #[test]
    fn malformed_citations_are_rejected() {
        for bad in [
            cite("OAS", "PATCH /orders"),
            cite("OAS", "GET orders"),
            cite("DOC", "abc"),
            cite("LEGACY", "Forever"),
        ] {
            let problems = problems_in(&bad);
            assert_eq!(problems.len(), 1, "{bad}: {problems:?}");
            assert!(problems[0].starts_with("malformed"), "{bad}: {problems:?}");
        }
    }

    #[test]
    fn manifest_layout_problems_are_reported() {
        let mut m = manifest();
        m.pages[1].2 = vec![(10, 29)];
        m.pages[0].1 = "https://example.com/a.md".to_owned();
        assert_eq!(
            check_manifest(&m),
            vec![
                format!(
                    "page a: url https://example.com/a.md does not start with {PAGE_URL_PREFIX}"
                ),
                "export_lines of a and b overlap at line 10".to_owned(),
                "last page b ends at line 29, but the snapshot has 30 lines".to_owned(),
            ]
        );
        m.pages[1].2 = vec![(12, 30)];
        m.pages[0].1 = "https://docs.dhanhq.co/markdown/a.md".to_owned();
        assert_eq!(
            check_manifest(&m),
            vec!["export lines 11-11 between a and b belong to no page".to_owned()]
        );
    }

    #[test]
    fn endpoint_doc_strings_must_be_citations_only() {
        let good = format!(
            "{}, {}",
            cite("DOC", "3712-3768"),
            cite("OAS", "POST /orders")
        );
        assert_eq!(
            parse_doc_string(&good),
            Ok(vec![
                Citation::Doc(3712, 3768),
                Citation::Oas("POST /orders".to_owned())
            ])
        );
        assert!(parse_doc_string("").is_err());
        assert!(parse_doc_string("see the orders page").is_err());
        assert!(parse_doc_string(&format!("{} trailing", cite("DOC", "1-2"))).is_err());
        let src = format!("    doc: \"{}\",\n    undoc: \"x\",", cite("DOC", "5-6"));
        assert_eq!(
            endpoint_doc_strings(&src),
            vec![(1, cite("DOC", "5-6").as_str())]
        );
    }

    #[test]
    fn mirror_references_are_detected() {
        let comment = format!("// see {}docs/dhan-api-docs.md", concat!(".ign", "ore/"));
        assert!(names_mirror(comment.as_bytes()));
        assert!(!names_mirror(
            b"// the ignored directory: .ignore alone, or ignore/"
        ));
        assert!(!names_mirror(b""));
    }

    #[test]
    fn values_running_into_other_characters_are_malformed() {
        for bad in [
            cite("DOC", "1-2-3"),
            cite("DOC", "1-\u{e9}"),
            cite("DOC", "12a"),
            format!("{} {}", cite("DOC", ""), 99999),
            cite("LEGACY", "ab\u{e9}"),
            cite("OAS", "GET /a\u{e9}"),
            cite("OAS", "#/components/schemas/Order\u{e9}"),
        ] {
            let problems = problems_in(&bad);
            assert_eq!(problems.len(), 1, "{bad}: {problems:?}");
            assert!(problems[0].starts_with("malformed"), "{bad}: {problems:?}");
        }
        assert_eq!(
            problems_in(&format!("({}).", cite("DOC", "5-6"))),
            Vec::<String>::new()
        );
        assert_eq!(
            problems_in(&format!("{}.", cite("OAS", "GET /orders"))),
            Vec::<String>::new()
        );
    }
}
