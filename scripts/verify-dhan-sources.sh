#!/usr/bin/env bash
# Compare every DhanHQ source listed in docs/dhan-sources.toml with the recorded
# fingerprints, and check that every OAS: citation in the repository still resolves.
#
#   scripts/verify-dhan-sources.sh [download-dir]
#   scripts/verify-dhan-sources.sh --offline <mirror-dir>
#   scripts/verify-dhan-sources.sh --rebase <new-export> [--offline <mirror-dir> | download-dir]
#
# Default mode needs network access to docs.dhanhq.co and dhanhq.co. It fetches the export,
# every [[page]], the OpenAPI spec and every [[legacy]] page into download-dir (a temporary
# directory by default) so that a changed source can be diffed and its citations re-checked.
#
# --offline <mirror-dir> runs the same comparisons against already-downloaded files laid out
# as <mirror-dir>/docs-export.md, <mirror-dir>/markdown/<page>.md,
# <mirror-dir>/dhan-api-v2.yaml and <mirror-dir>/legacy/<name>.html. Nothing is fetched.
#
# --rebase <new-export> re-derives every page's export_lines inside a regenerated export by
# locating each page's heading and opening lines, and prints the old and new ranges with
# their offset so that DOC: citations can be shifted in one pass. Page Markdown is read from
# the mirror with --offline, otherwise fetched into download-dir.
#
# Output is one line per source: "ok", "DRIFT" (content changed) or "FAIL" (missing or
# unreadable). Exits non-zero on any DRIFT or FAIL; drift prompts a human review of the
# affected citations. This script is manual only and never runs in CI.
#
# Needs bash, curl (online mode) and python3 >= 3.11 (for tomllib; no third-party modules).
set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
manifest="$root/docs/dhan-sources.toml"

usage() { sed -n '5,7p' "$0" | sed 's/^# *//' >&2; exit 2; }

offline=""
rebase=""
out=""
while [ $# -gt 0 ]; do
  case "$1" in
    --offline) [ $# -ge 2 ] || usage; offline=$2; shift 2;;
    --rebase) [ $# -ge 2 ] || usage; rebase=$2; shift 2;;
    -h|--help) usage;;
    -*) usage;;
    *) [ -z "$out" ] || usage; out=$1; shift;;
  esac
done

if [ -n "$offline" ]; then
  # Offline mode reads the mirror in place and downloads nothing.
  [ -z "$out" ] || usage
  [ -d "$offline" ] || { echo "FAIL  mirror directory not found: $offline" >&2; exit 2; }
  offline=$(cd "$offline" && pwd)
fi
if [ -n "$rebase" ]; then
  [ -f "$rebase" ] || { echo "FAIL  export not found: $rebase" >&2; exit 2; }
fi
if [ -z "$offline" ]; then
  out=${out:-$(mktemp -d)}
  mkdir -p "$out"
  out=$(cd "$out" && pwd)
fi

python3 - "$root" "$manifest" "${offline:-}" "${out:-}" "${rebase:-}" <<'PY'
import hashlib
import html
import os
import re
import subprocess
import sys

if sys.version_info < (3, 11):
    sys.exit("verify-dhan-sources.sh needs python3 >= 3.11 (tomllib)")
import tomllib

root, manifest_path, offline, out, rebase = sys.argv[1:6]
with open(manifest_path, "rb") as f:
    manifest = tomllib.load(f)

status = 0


def report(kind, name, detail=""):
    global status
    if kind != "ok":
        status = 1
    print(f"{kind:<5} {name}{': ' + detail if detail else ''}")


def obtain(url, rel):
    """Return the path of the local copy of url: the mirror file, or a fresh download."""
    if offline:
        path = os.path.join(offline, rel)
        return path if os.path.isfile(path) else None
    path = os.path.join(out, rel)
    os.makedirs(os.path.dirname(path), exist_ok=True)
    if subprocess.run(["curl", "-fsSL", "-o", path, url]).returncode != 0:
        return None
    return path


def fingerprint(path):
    data = open(path, "rb").read()
    return hashlib.sha256(data).hexdigest(), len(data), data.count(b"\n")


def compare(name, url, rel, sha, size, lines):
    path = obtain(url, rel)
    if path is None:
        report("FAIL", name, f"could not {'find' if offline else 'fetch'} {url}")
        return None
    got = fingerprint(path)
    if got == (sha, size, lines):
        report("ok", name)
    else:
        report("DRIFT", name, f"sha256 {got[0]}, {got[1]} bytes, {got[2]} lines "
                              f"(recorded {sha}, {size}, {lines})")
    return path


def article_text(raw):
    """Whitespace-normalised text of the first <article> element of an HTML page."""
    m = re.search(r"<article[^>]*>(.*?)</article>", raw, re.S)
    if m is None:
        return None
    text = re.sub(r"<[^>]+>", " ", m.group(1))
    return " ".join(html.unescape(text).split())


def oas_index(path):
    """Operations and schema names of an OpenAPI YAML file, read by indentation."""
    ops, schemas = set(), set()
    section, sub, current_path = None, None, None
    for line in open(path, encoding="utf-8"):
        if not line.strip() or line.lstrip().startswith("#"):
            continue
        indent = len(line) - len(line.lstrip(" "))
        key = line.strip().split(":", 1)[0].strip("'\"")
        if indent == 0:
            section, sub = key, None
        elif section == "paths":
            if indent == 2:
                current_path = key
            elif indent == 4 and key in ("get", "post", "put", "delete", "patch"):
                ops.add(f"{key.upper()} {current_path}")
        elif section == "components":
            if indent == 2:
                sub = key
            elif indent == 4 and sub == "schemas":
                schemas.add(key)
    return ops, schemas


OAS_RE = re.compile(r"(?<![A-Za-z0-9_])OAS:(?:(GET|POST|PUT|DELETE) (/[A-Za-z0-9_{}/.-]*)|#/components/schemas/([A-Za-z0-9_]+))")


def oas_citations():
    found = {}
    for top in ("src", "tests", "docs"):
        for dirpath, _, files in os.walk(os.path.join(root, top)):
            for fn in files:
                path = os.path.join(dirpath, fn)
                try:
                    text = open(path, encoding="utf-8").read()
                except (UnicodeDecodeError, OSError):
                    continue
                for m in OAS_RE.finditer(text):
                    if m.group(1):
                        cite = f"{m.group(1)} {m.group(2).rstrip('.')}"
                    else:
                        cite = f"#/components/schemas/{m.group(3)}"
                    found.setdefault(cite, os.path.relpath(path, root))
    return found


def page_opening(path):
    """Title and first three non-blank lines of a page, with heading markers removed."""
    lines = [l.lstrip("#").strip() for l in open(path, encoding="utf-8").read().split("\n") if l.strip()]
    return (lines[0], lines[1:4]) if lines else (None, [])


def do_rebase():
    exp = open(rebase, encoding="utf-8").read().split("\n")
    if exp and exp[-1] == "":
        exp.pop()
    pages = sorted(manifest["page"], key=lambda p: int(p["export_lines"][0].split("-")[0]))
    heads, cursor = [], 0
    for p in pages:
        path = obtain(p["url"], f"markdown/{p['name']}.md")
        title, opening = page_opening(path) if path else (None, [])
        found = None
        if title is not None:
            for i in range(cursor, len(exp)):
                if exp[i].startswith("#") and exp[i].lstrip("#").strip() == title:
                    nxt = [l.lstrip("#").strip() for l in exp[i + 1:i + 60] if l.strip()][:len(opening)]
                    if nxt == opening:
                        found = i
                        break
        if found is None:
            report("FAIL", p["name"], "heading and opening lines not found in the new export")
            heads.append(None)
            continue
        heads.append(found + 1)
        cursor = found + 1
    print(f"new export: {len(exp)} lines")
    for k, p in enumerate(pages):
        if heads[k] is None:
            continue
        nxt = next((h for h in heads[k + 1:] if h is not None), len(exp) + 1)
        new = f"{heads[k]}-{nxt - 1}"
        old = p["export_lines"][0]
        offset = heads[k] - int(old.split("-")[0])
        kind = "ok" if new == old else "MOVED"
        print(f"{kind:<5} {p['name']}: {old} -> {new} (offset {offset:+d})")


if rebase:
    do_rebase()
    sys.exit(status)

snap = manifest["snapshot"]
compare("snapshot " + snap["name"], snap["url"], snap["name"], snap["sha256"], snap["bytes"], snap["lines"])

for p in manifest["page"]:
    compare(p["name"], p["url"], f"markdown/{p['name']}.md", p["sha256"], p["bytes"], p["lines"])

oas = manifest["openapi"]
oas_path = compare("openapi dhan-api-v2.yaml", oas["url"], "dhan-api-v2.yaml", oas["sha256"], oas["bytes"], oas["lines"])
if oas_path is not None:
    ops, schemas = oas_index(oas_path)
    cites = oas_citations()
    for cite, where in sorted(cites.items()):
        known = cite[len("#/components/schemas/"):] in schemas if cite.startswith("#") else cite in ops
        report("ok" if known else "DRIFT", f"OAS:{cite}", "" if known else f"not in the spec (cited in {where})")
    print(f"      {len(cites)} OAS citation(s) checked against {len(ops)} operations and {len(schemas)} schemas")

for leg in manifest.get("legacy", []):
    name = "legacy " + leg["name"]
    path = obtain(leg["url"], f"legacy/{leg['name']}.html")
    if path is None:
        report("FAIL", name, f"could not {'find' if offline else 'fetch'} {leg['url']}")
        continue
    text = article_text(open(path, encoding="utf-8", errors="replace").read())
    if text is None:
        report("FAIL", name, "no <article> element")
        continue
    got = hashlib.sha256(text.encode("utf-8")).hexdigest()
    if got == leg["text_sha256"]:
        report("ok", name)
    else:
        report("DRIFT", name, f"text_sha256 {got} (recorded {leg['text_sha256']})")

if out:
    print(f"fetched copies: {out}")
sys.exit(status)
PY
