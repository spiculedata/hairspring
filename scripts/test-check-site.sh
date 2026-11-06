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

# expect_fail NAME HEAD BODY MESSAGE: the check must fail and its output must contain MESSAGE, so a mutant that fails
# for an unrelated reason does not count as caught.
expect_fail() {
  local name="$1" head="$2" body="$3" message="$4" dir="$TMP/$1"
  make_site "$dir" "$head" "$body"
  check_fails "$name" "$dir" "$message"
}

check_fails() {
  local name="$1" dir="$2" message="$3"
  if python3 "$CHECKER" "$dir" >"$TMP/out" 2>&1; then
    FAIL=$((FAIL + 1))
    printf 'FAIL %s: expected the check to fail, but it passed\n' "$name"
  elif ! grep -qF -- "$message" "$TMP/out"; then
    FAIL=$((FAIL + 1))
    printf 'FAIL %s: it failed, but without the expected message %q:\n' "$name" "$message"
    cat "$TMP/out"
  else
    PASS=$((PASS + 1))
  fi
}

expect_pass "the real site" "$REAL_SITE"
make_site "$TMP/good" '<link rel="canonical" href="https://example.org/"><style>a{background:url(data:image/png;base64,AAAA)}</style>' \
  '<a href="https://example.org/">other site</a><a href="#top">top</a>'
expect_pass "a good page with a canonical link, a data URI and an external anchor" "$TMP/good"

expect_fail "external-script" '<script src="https://cdn.example/a.js"></script>' '' 'external script src https://cdn.example/a.js'
expect_fail "protocol-relative-script" '<script src="//cdn.example/a.js"></script>' '' 'external script src //cdn.example/a.js'
expect_fail "external-stylesheet" '<link rel="stylesheet" href="https://cdn.example/a.css">' '' 'external link href https://cdn.example/a.css'
expect_fail "external-preconnect" '<link rel="preconnect" href="https://fonts.example">' '' 'external link href https://fonts.example'
expect_fail "external-manifest" '<link rel="manifest" href="https://cdn.example/m.json">' '' 'external link href https://cdn.example/m.json'
expect_fail "base-href" '<base href="https://other.example/">' '' 'external base href https://other.example/'
expect_fail "meta-refresh" '<meta http-equiv="refresh" content="0; url=https://other.example/">' '' 'external meta content https://other.example/'
expect_fail "css-import" '<style>@import url(https://fonts.example/a.css);</style>' '' 'external asset in <style>: https://fonts.example/a.css'
expect_fail "css-import-string" '<style>@import "https://fonts.example/b.css";</style>' '' 'external asset in <style>: https://fonts.example/b.css'
expect_fail "css-font-face" '<style>@font-face{font-family:x;src:url("https://fonts.example/x.woff2")}</style>' '' 'external asset in <style>: https://fonts.example/x.woff2'
expect_fail "css-background" '<style>body{background:url(//img.example/a.png)}</style>' '' 'external asset in <style>: //img.example/a.png'
expect_fail "inline-style-url" '' '<div style="background:url(https://img.example/a.png)">x</div>' 'external asset in style attribute on <div>: https://img.example/a.png'
expect_fail "iframe" '' '<iframe src="https://embed.example/x"></iframe>' 'external iframe src https://embed.example/x'
expect_fail "video" '' '<video src="https://media.example/x.mp4"></video>' 'external video src https://media.example/x.mp4'
expect_fail "audio" '' '<audio src="https://media.example/x.mp3"></audio>' 'external audio src https://media.example/x.mp3'
expect_fail "track" '' '<video><track src="https://media.example/x.vtt"></video>' 'external track src https://media.example/x.vtt'
expect_fail "embed" '' '<embed src="https://media.example/x.swf">' 'external embed src https://media.example/x.swf'
expect_fail "object" '' '<object data="https://media.example/y.swf"></object>' 'external object data https://media.example/y.swf'
expect_fail "svg-image" '' '<svg><image href="https://img.example/s.png"/></svg>' 'external image href https://img.example/s.png'
expect_fail "svg-use" '' '<svg><use xlink:href="https://img.example/s.svg#a"/></svg>' 'external use xlink:href https://img.example/s.svg#a'
expect_fail "external-image" '' '<img src="https://img.example/a.png" alt="a" width="1" height="1">' 'external img src https://img.example/a.png'
expect_fail "missing-alt" '' '<img src="data:image/gif;base64,R0lGODlhAQABAAAAACw=" width="1" height="1">' 'has no alt attribute'
expect_fail "missing-size" '' '<img src="data:image/gif;base64,R0lGODlhAQABAAAAACw=" alt="a">' 'has no width and height'
expect_fail "broken-link" '' '<a href="missing.html">m</a>' 'a href missing.html does not resolve'
expect_fail "broken-anchor" '' '<a href="#nowhere">m</a>' 'anchor #nowhere not found'
expect_fail "broken-image" '' '<img src="nope.webp" alt="a" width="1" height="1">' 'img src nope.webp does not resolve'
expect_fail "second-h1" '' '<h1>Again</h1>' 'expected exactly one <h1>, found 2'
expect_fail "inline-handler" '' '<button onclick="x()">b</button>' 'inline event handler onclick'

make_site "$TMP/no-title" '' ''
sed -i.bak 's#<title>Test</title>##' "$TMP/no-title/index.html" && rm -f "$TMP/no-title/index.html.bak"
check_fails "no-title" "$TMP/no-title" 'no <title>'

make_site "$TMP/no-robots" '' ''
rm -f "$TMP/no-robots/robots.txt"
check_fails "no-robots" "$TMP/no-robots" 'missing robots.txt'

big_dir="$TMP/too-big"
make_site "$big_dir" '' ''
head -c 70000 /dev/zero | tr '\0' 'x' >"$big_dir/big.txt"
check_fails "too-big" "$big_dir" 'budget'

printf '%s passed, %s failed\n' "$PASS" "$FAIL"
[ "$FAIL" -eq 0 ]
