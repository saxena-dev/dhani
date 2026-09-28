//! `Redactor` and `sanitize()`: scrubbing of secrets from diagnostic output.
//!
//! Every piece of external free text that the crate stores (broker error messages, decode
//! details, unknown feed messages) passes through [`Redactor::sanitize`], which runs in order:
//!
//! 1. **Exact values**: every registered secret value is replaced.
//! 2. **Keyed values**: the value after a credential key such as `pin=`, `access-token:` or
//!    `"totp" : "…"` is replaced (keys are matched case-insensitively, quoted or unquoted).
//! 3. **JWTs**: `eyJ…​.…​.…`-shaped tokens are replaced.
//! 4. **Long opaque runs**: 32 or more characters from `[A-Za-z0-9_\-+/=.]` are replaced.
//! 5. **Truncation**: the result is cut to at most 512 bytes on a character boundary, ending in
//!    a `…[truncated]` marker.
//!
//! Order and trade identifiers are not secrets and are left alone.

use std::fmt;

use secrecy::{ExposeSecret, SecretString};

const MASK: &str = "<redacted>";
const MAX_BYTES: usize = 512;
const TRUNCATED: &str = "…[truncated]";
/// Shortest run of opaque characters that is masked.
const LONG_RUN: usize = 32;
/// Keys whose value is always masked, lowercase.
const SECRET_KEYS: [&str; 18] = [
    "access-token",
    "access_token",
    "accesstoken",
    "token",
    "client-id",
    "clientid",
    "dhanclientid",
    "client_id",
    "app_secret",
    "app_id",
    "partner_secret",
    "partner_id",
    "pin",
    "totp",
    "tokenid",
    "consentappid",
    "consentid",
    "secret",
];

/// Masks secrets in free text before it is stored in an error, event or log.
///
/// Holds the exact secret values to mask, each as a [`SecretString`]. The configured client ID
/// and access token are registered once for the lifetime of a client or feed with
/// [`register`](Redactor::register); secrets used by a single call (PIN, TOTP, app or partner
/// credentials, token and consent IDs) are added for that call only with
/// [`for_call`](Redactor::for_call).
#[derive(Clone, Default)]
pub struct Redactor {
    secrets: Vec<SecretString>,
}

impl fmt::Debug for Redactor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Redactor")
            .field("secrets", &self.secrets.len())
            .finish()
    }
}

impl Redactor {
    /// A redactor with no registered values; it still applies the pattern rules.
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a long-lived secret value. Empty values are ignored.
    pub fn register(&mut self, secret: &str) {
        if !secret.is_empty() {
            self.secrets.push(SecretString::from(secret));
        }
    }

    /// A redactor for one call: this one's values plus `secrets`. Drop it when the call ends.
    pub fn for_call<'a>(&self, secrets: impl IntoIterator<Item = &'a str>) -> Self {
        let mut scoped = self.clone();
        for secret in secrets {
            scoped.register(secret);
        }
        scoped
    }

    /// Masks secrets in `text` and bounds its length.
    pub fn sanitize(&self, text: &str) -> String {
        let text = self.mask_exact(text);
        let text = mask_keyed(&text);
        let text = mask_jwts(&text);
        let text = mask_long_runs(&text);
        truncate(text)
    }

    /// One left-to-right pass masking the longest registered value at each position, so
    /// overlapping values cannot leave tails and later values cannot rewrite earlier masks.
    fn mask_exact(&self, text: &str) -> String {
        if self.secrets.is_empty() {
            return text.to_owned();
        }
        let values: Vec<&str> = self.secrets.iter().map(|s| s.expose_secret()).collect();
        let mut out = String::with_capacity(text.len());
        let mut rest = text;
        while let Some(c) = rest.chars().next() {
            let hit = values
                .iter()
                .filter(|v| rest.starts_with(**v))
                .map(|v| v.len())
                .max();
            match hit {
                Some(len) => {
                    out.push_str(MASK);
                    rest = &rest[len..];
                }
                None => {
                    out.push(c);
                    rest = &rest[c.len_utf8()..];
                }
            }
        }
        out
    }
}

/// [`Redactor::sanitize`] with no registered values.
pub fn sanitize(text: &str) -> String {
    Redactor::new().sanitize(text)
}

fn is_value_end(b: u8) -> bool {
    b.is_ascii_whitespace() || matches!(b, b'&' | b',' | b';' | b'"' | b'\'' | b'}')
}

/// Length of the quote at `rest[i..]`, optionally escaped (`\"` inside JSON text): 0 if none.
fn quote_len(rest: &[u8], i: usize) -> usize {
    match (rest.get(i), rest.get(i + 1)) {
        (Some(b'"' | b'\''), _) => 1,
        (Some(b'\\'), Some(b'"' | b'\'')) => 2,
        _ => 0,
    }
}

/// Length of the separator after a key at `rest`, up to the start of the value: an optional
/// closing quote, optional whitespace, `=` or `:`, optional whitespace and an optional opening
/// quote (quotes may be backslash-escaped). `None` if `rest` does not start with a separator.
fn separator_len(rest: &[u8]) -> Option<usize> {
    let mut i = quote_len(rest, 0);
    while rest.get(i).is_some_and(u8::is_ascii_whitespace) {
        i += 1;
    }
    if !matches!(rest.get(i), Some(b'=' | b':')) {
        return None;
    }
    i += 1;
    while rest.get(i).is_some_and(u8::is_ascii_whitespace) {
        i += 1;
    }
    Some(i + quote_len(rest, i))
}

/// Masks the value after each credential key.
fn mask_keyed(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut copied = 0;
    let mut i = 0;
    while i < bytes.len() {
        // No word-boundary requirement: `userToken=` or `clientSecret:` are masked too, at the
        // cost of harmless over-masking such as `spin=`.
        let hit = SECRET_KEYS.iter().find_map(|key| {
            let end = i + key.len();
            let matches = bytes
                .get(i..end)
                .is_some_and(|w| w.eq_ignore_ascii_case(key.as_bytes()));
            if !matches {
                return None;
            }
            separator_len(&bytes[end..]).map(|sep| end + sep)
        });
        let Some(value_start) = hit else {
            i += 1;
            continue;
        };
        let value_len = bytes[value_start..]
            .iter()
            .position(|&b| is_value_end(b))
            .unwrap_or(bytes.len() - value_start);
        if value_len == 0 {
            i = value_start;
            continue;
        }
        // Every index here sits next to an ASCII byte, so it is a char boundary.
        out.push_str(&text[copied..value_start]);
        out.push_str(MASK);
        copied = value_start + value_len;
        i = copied;
    }
    out.push_str(&text[copied..]);
    out
}

fn is_base64url(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'-'
}

/// Length of a JWT (`eyJ[A-Za-z0-9_-]{5,}\.[A-Za-z0-9_-]{5,}\.[A-Za-z0-9_-]*`) starting at the
/// beginning of `s`, if any.
fn jwt_len(s: &[u8]) -> Option<usize> {
    if !s.starts_with(b"eyJ") {
        return None;
    }
    let segment = |from: usize| s[from..].iter().take_while(|&&b| is_base64url(b)).count();
    let header = segment(3);
    let dot1 = 3 + header;
    if header < 5 || s.get(dot1) != Some(&b'.') {
        return None;
    }
    let payload = segment(dot1 + 1);
    let dot2 = dot1 + 1 + payload;
    if payload < 5 || s.get(dot2) != Some(&b'.') {
        return None;
    }
    Some(dot2 + 1 + segment(dot2 + 1))
}

fn mask_jwts(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut copied = 0;
    let mut i = 0;
    while i < bytes.len() {
        match jwt_len(&bytes[i..]) {
            Some(len) => {
                out.push_str(&text[copied..i]);
                out.push_str(MASK);
                i += len;
                copied = i;
            }
            // Any `eyJ` inside the header run just scanned fails the same way (its header ends
            // at the same byte and is shorter), so skip the run: this keeps the scan linear.
            None if bytes[i..].starts_with(b"eyJ") => {
                i += 3 + bytes[i + 3..]
                    .iter()
                    .take_while(|&&b| is_base64url(b))
                    .count();
            }
            None => i += 1,
        }
    }
    out.push_str(&text[copied..]);
    out
}

fn is_opaque(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'+' | b'/' | b'=' | b'.')
}

fn mask_long_runs(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut copied = 0;
    let mut i = 0;
    while i < bytes.len() {
        if !is_opaque(bytes[i]) {
            i += 1;
            continue;
        }
        let run = bytes[i..].iter().take_while(|&&b| is_opaque(b)).count();
        if run >= LONG_RUN {
            out.push_str(&text[copied..i]);
            out.push_str(MASK);
            copied = i + run;
        }
        i += run;
    }
    out.push_str(&text[copied..]);
    out
}

/// Cuts `text` so that it, including the marker, fits in [`MAX_BYTES`].
fn truncate(mut text: String) -> String {
    if text.len() <= MAX_BYTES {
        return text;
    }
    let mut end = MAX_BYTES - TRUNCATED.len();
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text.truncate(end);
    text.push_str(TRUNCATED);
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_values_are_masked_anywhere() {
        let mut r = Redactor::new();
        r.register("1000000009");
        assert_eq!(
            r.sanitize("client 1000000009 rejected"),
            "client <redacted> rejected"
        );
        assert_eq!(r.sanitize("x1000000009y"), "x<redacted>y");
        let call = r.for_call(["482913"]);
        assert_eq!(
            call.sanitize("otp 482913 invalid for 1000000009"),
            "otp <redacted> invalid for <redacted>"
        );
        // The per-call value does not outlive the call.
        assert_eq!(r.sanitize("otp 482913"), "otp 482913");
        r.register("");
        assert_eq!(r.sanitize("abc"), "abc");
    }

    #[test]
    fn longer_exact_values_win() {
        let mut r = Redactor::new();
        r.register("abc");
        r.register("abcdef");
        assert_eq!(r.sanitize("[abcdef]"), "[<redacted>]");
    }

    #[test]
    fn exact_values_are_masked_in_one_pass() {
        let mut r = Redactor::new();
        r.register("abcX");
        r.register("Xdef");
        assert_eq!(r.sanitize("abcXdef"), "<redacted>def");
        // A later short value does not rewrite the mask text.
        r.register("e");
        assert_eq!(r.sanitize("abcX e"), "<redacted> <redacted>");
    }

    #[test]
    fn keyed_values_in_query_header_and_json_forms() {
        assert_eq!(sanitize("pin=1234"), "pin=<redacted>");
        assert_eq!(
            sanitize(r#"{"access-token" : "abc"}"#),
            r#"{"access-token" : "<redacted>"}"#
        );
        assert_eq!(
            sanitize("Access-Token:abc&x=1"),
            "Access-Token:<redacted>&x=1"
        );
        assert_eq!(sanitize("TOTP = 123456; next"), "TOTP = <redacted>; next");
        assert_eq!(
            sanitize("dhanClientId=1000000009&pin=1&totp=2"),
            "dhanClientId=<redacted>&pin=<redacted>&totp=<redacted>"
        );
        assert_eq!(sanitize("tokenId: t0k, ok"), "tokenId: <redacted>, ok");
        assert_eq!(
            sanitize(r#"{'consentAppId':'c-1'}"#),
            r#"{'consentAppId':'<redacted>'}"#
        );
        assert_eq!(sanitize(r#""pin":1234}"#), r#""pin":<redacted>}"#);
        // JSON embedded as an escaped string.
        assert_eq!(
            sanitize(r#"{\"pin\":\"1234\"}"#),
            r#"{\"pin\":\"<redacted>"}"#
        );
    }

    #[test]
    fn keys_need_a_separator_but_no_boundary() {
        // Keys are matched inside longer names too; a key without a separator is plain text.
        assert_eq!(sanitize("userToken=abc&x=1"), "userToken=<redacted>&x=1");
        assert_eq!(sanitize("clientSecret: xyz"), "clientSecret: <redacted>");
        assert_eq!(sanitize("spin=3"), "spin=<redacted>");
        assert_eq!(sanitize("enter your pin now"), "enter your pin now");
        assert_eq!(sanitize("token expired"), "token expired");
        // An empty value leaves the text as it is.
        assert_eq!(sanitize("pin= "), "pin= ");
    }

    #[test]
    fn order_and_trade_identifiers_are_kept() {
        assert_eq!(sanitize("orderId=112111182198"), "orderId=112111182198");
        assert_eq!(
            sanitize(r#"{"correlationId":"my-order_1"}"#),
            r#"{"correlationId":"my-order_1"}"#
        );
    }

    #[test]
    fn jwts_are_masked() {
        assert_eq!(
            sanitize("bearer eyJhbGciOi.eyJzdWIiOi.sig end"),
            "bearer <redacted> end"
        );
        assert_eq!(sanitize("eyJhbGciOi.eyJzdWIiOi."), "<redacted>");
        assert_eq!(sanitize("eyJ.x"), "eyJ.x");
        assert_eq!(sanitize("eyJabcd.efghi.j"), "eyJabcd.efghi.j");
        assert_eq!(sanitize("eyJabcde.efgh.j"), "eyJabcde.efgh.j");
    }

    #[test]
    fn long_opaque_runs_are_masked() {
        let run32 = "Ab3".repeat(11)[..32].to_owned();
        let run31 = &run32[..31];
        assert_eq!(sanitize(&format!("key {run32} end")), "key <redacted> end");
        assert_eq!(
            sanitize(&format!("key {run31} end")),
            format!("key {run31} end")
        );
        assert_eq!(sanitize("a+b/c=d.e_f-g".repeat(3).as_str()), "<redacted>");
    }

    #[test]
    fn output_is_truncated_on_a_char_boundary() {
        // 700 bytes of 2-byte characters.
        let input = "\u{e9}".repeat(350);
        let out = sanitize(&input);
        assert!(out.len() <= MAX_BYTES, "{}", out.len());
        assert!(out.ends_with("…[truncated]"));
        let kept = out.strip_suffix("…[truncated]").unwrap();
        assert_eq!(kept, "\u{e9}".repeat(249));
        let short = "\u{e9}".repeat(256);
        assert_eq!(sanitize(&short), short);
    }

    #[test]
    fn masking_happens_before_truncation() {
        // A registered secret straddling the 498-byte cut: truncating first would leave its
        // first bytes behind, unmatched by the exact rule.
        let mut r = Redactor::new();
        r.register("SENTINELSECRET");
        let text = format!(
            "{}abcd SENTINELSECRET {}",
            ". ".repeat(245),
            "z ".repeat(50)
        );
        let out = r.sanitize(&text);
        assert!(out.ends_with("…[truncated]"), "{out}");
        assert!(!out.contains("SEN"), "{out}");
    }

    #[test]
    fn jwt_scan_is_linear_on_hostile_input() {
        // Quadratic scanning would make this take many seconds.
        let hostile = "eyJ".repeat(300_000);
        let out = sanitize(&hostile);
        assert!(out.len() <= MAX_BYTES);
    }

    #[test]
    fn debug_shows_only_the_count() {
        let mut r = Redactor::new();
        r.register("SENTINEL-SECRET");
        assert_eq!(format!("{r:?}"), "Redactor { secrets: 1 }");
    }
}
