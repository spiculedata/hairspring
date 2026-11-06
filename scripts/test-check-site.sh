#!/usr/bin/env bash
# Tests for scripts/check-site.py: the real site passes, and each mutant of a good page (one deliberate fault each)
# must be caught. Runs offline in a temporary directory.
#
#   bash scripts/test-check-site.sh
set -u

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CHECKER="$HERE/check-site.py"
REAL_SITE="$HERE/../site"
TMP="$(mktemp -d "${TMPDIR:-/tmp}/hairspring-site-tests.XXXXXX")"
trap 'rm -rf "$TMP"' EXIT
PASS=0
FAIL=0

# Writes a minimal good site to $1, with $2 spliced into <head> and $3 into <body>.
make_site() {
  local dir="$1" head="$2" body="$3"
  mkdir -p "$dir"
  printf 'User-agent: *\nAllow: /\n' >"$dir/robots.txt"
  printf '<!doctype html><html><head><title>Gone</title></head><body><h1>Gone</h1></body></html>\n' >"$dir/404.html"
  printf '<!doctype html><html><head><meta charset="utf-8"><title>Test</title>%s</head><body><h1>Test</h1><p id="top">x</p>%s</body></html>\n' \
    "$head" "$body" >"$dir/index.html"
}

expect_pass() {
  local name="$1" dir="$2"
  if python3 "$CHECKER" "$dir" >"$TMP/out" 2>&1; then
    PASS=$((PASS + 1))
  else
    FAIL=$((FAIL + 1))
    printf 'FAIL %s: expected the check to pass:\n' "$name"
    cat "$TMP/out"
  fi
}

expect_fail() {
  local name="$1" head="$2" body="$3" dir="$TMP/$1"
  make_site "$dir" "$head" "$body"
  if python3 "$CHECKER" "$dir" >"$TMP/out" 2>&1; then
    FAIL=$((FAIL + 1))
    printf 'FAIL %s: expected the check to fail, but it passed\n' "$name"
  else
    PASS=$((PASS + 1))
  fi
}

expect_pass "the real site" "$REAL_SITE"
make_site "$TMP/good" '<link rel="canonical" href="https://example.org/"><style>a{background:url(data:image/png;base64,AAAA)}</style>' \
  '<a href="https://example.org/">other site</a><a href="#top">top</a>'
expect_pass "a good page with a canonical link, a data URI and an external anchor" "$TMP/good"

expect_fail "external-script" '<script src="https://cdn.example/a.js"></script>' ''
expect_fail "protocol-relative-script" '<script src="//cdn.example/a.js"></script>' ''
expect_fail "external-stylesheet" '<link rel="stylesheet" href="https://cdn.example/a.css">' ''
expect_fail "external-preconnect" '<link rel="preconnect" href="https://fonts.example">' ''
expect_fail "external-manifest" '<link rel="manifest" href="https://cdn.example/m.json">' ''
expect_fail "css-import" '<style>@import url(https://fonts.example/a.css);</style>' ''
expect_fail "css-import-string" '<style>@import "https://fonts.example/a.css";</style>' ''
expect_fail "css-font-face" '<style>@font-face{font-family:x;src:url("https://fonts.example/x.woff2")}</style>' ''
expect_fail "css-background" '<style>body{background:url(//img.example/a.png)}</style>' ''
expect_fail "inline-style-url" '' '<div style="background:url(https://img.example/a.png)">x</div>'
expect_fail "iframe" '' '<iframe src="https://embed.example/x"></iframe>'
expect_fail "video" '' '<video src="https://media.example/x.mp4"></video>'
expect_fail "audio" '' '<audio src="https://media.example/x.mp3"></audio>'
expect_fail "track" '' '<video><track src="https://media.example/x.vtt"></video>'
expect_fail "embed" '' '<embed src="https://media.example/x.swf">'
expect_fail "object" '' '<object data="https://media.example/x.swf"></object>'
expect_fail "external-image" '' '<img src="https://img.example/a.png" alt="a" width="1" height="1">'
expect_fail "missing-alt" '' '<img src="data:image/gif;base64,R0lGODlhAQABAAAAACw=" width="1" height="1">'
expect_fail "missing-size" '' '<img src="data:image/gif;base64,R0lGODlhAQABAAAAACw=" alt="a">'
expect_fail "broken-link" '' '<a href="missing.html">m</a>'
expect_fail "broken-anchor" '' '<a href="#nowhere">m</a>'
expect_fail "broken-image" '' '<img src="nope.webp" alt="a" width="1" height="1">'
expect_fail "second-h1" '' '<h1>Again</h1>'
expect_fail "inline-handler" '' '<button onclick="x()">b</button>'
make_site "$TMP/no-title" '' ''
sed -i.bak 's#<title>Test</title>##' "$TMP/no-title/index.html" && rm -f "$TMP/no-title/index.html.bak"
if python3 "$CHECKER" "$TMP/no-title" >/dev/null 2>&1; then
  FAIL=$((FAIL + 1))
  printf 'FAIL no-title: expected the check to fail\n'
else
  PASS=$((PASS + 1))
fi

big_dir="$TMP/too-big"
make_site "$big_dir" '' ''
head -c 70000 /dev/zero | tr '\0' 'x' >"$big_dir/big.txt"
if python3 "$CHECKER" "$big_dir" >/dev/null 2>&1; then
  FAIL=$((FAIL + 1))
  printf 'FAIL too-big: expected the size budget to fail\n'
else
  PASS=$((PASS + 1))
fi

printf '%s passed, %s failed\n' "$PASS" "$FAIL"
[ "$FAIL" -eq 0 ]
