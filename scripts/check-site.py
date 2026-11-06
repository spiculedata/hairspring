#!/usr/bin/env python3
"""Check the static site in site/ without a browser or any network access.

Checks, for every .html file: a <title>, exactly one <h1> (404.html included), local links, anchors and assets that
resolve, <img> with alt, width and height, no inline event handlers, and no external asset: nothing that makes the
browser fetch from another origin. That covers script, img, source, iframe, video, audio, track, embed, object and
input src or data attributes, link elements other than canonical and alternate (stylesheet, icon, preload, preconnect,
manifest and so on), and url(...) and @import with an http(s) or protocol-relative address in <style> text and in style
attributes. Plain <a href> links to other sites are allowed, as are canonical and Open Graph URLs.
Also checks the size budget (HTML, CSS and JS together; images are counted separately), and that 404.html,
robots.txt and index.html exist (robots.txt is harmless and stays so a crawler never has to guess).

Usage: python3 scripts/check-site.py [site-dir]
Exit status: 0 when every check passes, 1 otherwise.
"""
import re
import sys
from html.parser import HTMLParser
from pathlib import Path
from urllib.parse import unquote, urlparse

TEXT_BUDGET_BYTES = 60 * 1024
IMAGE_BUDGET_BYTES = 150 * 1024
TEXT_SUFFIXES = {".html", ".css", ".js", ".txt", ".svg"}
IMAGE_SUFFIXES = {".webp", ".png", ".jpg", ".jpeg", ".gif", ".avif"}


ASSET_SRC_TAGS = {"script", "img", "source", "iframe", "video", "audio", "track", "embed", "input"}
# <link rel> values that name a document, not something the browser fetches while rendering.
NON_ASSET_LINK_RELS = {"canonical", "alternate", "author", "license", "help", "next", "prev"}
CSS_URL = re.compile(r"url\(\s*(?:\"([^\"]*)\"|'([^']*)'|([^)\"'\s]*))\s*\)|@import\s+(?:\"([^\"]*)\"|'([^']*)')", re.I)


class PageParser(HTMLParser):
    def __init__(self):
        super().__init__(convert_charrefs=True)
        self.title = ""
        self.h1_count = 0
        self.ids = set()
        self.refs = []  # (tag, attribute, value)
        self.problems = []
        self._in_title = False
        self._in_style = False

    def handle_starttag(self, tag, attrs):
        a = dict(attrs)
        if tag == "title":
            self._in_title = True
        if tag == "style":
            self._in_style = True
        if a.get("style"):
            self._scan_css(a["style"], f"style attribute on <{tag}>")
        if tag == "h1":
            self.h1_count += 1
        if a.get("id"):
            self.ids.add(a["id"])
        for name in a:
            if name.startswith("on"):
                self.problems.append(f"inline event handler {name} on <{tag}>")
        if tag == "a" and a.get("href"):
            self.refs.append((tag, "href", a["href"]))
        if tag == "img":
            self._check_img(a)
        if tag in ASSET_SRC_TAGS:
            for attr in ("src", "srcset", "poster"):
                for part in (a.get(attr) or "").split(","):
                    if part.strip():
                        self.refs.append((tag, attr, part.strip().split()[0]))
        if tag == "object" and a.get("data"):
            self.refs.append((tag, "data", a["data"]))
        if tag == "link" and a.get("href"):
            rels = set((a.get("rel") or "").lower().split())
            if not rels <= NON_ASSET_LINK_RELS:
                self.refs.append((tag, "href", a["href"]))

    def _check_img(self, a):
        if "alt" not in a:
            self.problems.append(f"<img src={a.get('src')!r}> has no alt attribute")
        if not a.get("width") or not a.get("height"):
            self.problems.append(f"<img src={a.get('src')!r}> has no width and height")

    def handle_endtag(self, tag):
        if tag == "title":
            self._in_title = False
        if tag == "style":
            self._in_style = False

    def handle_data(self, data):
        if self._in_title:
            self.title += data
        if self._in_style:
            self._scan_css(data, "<style>")

    def _scan_css(self, css, where):
        for match in CSS_URL.finditer(css):
            value = (match.group(1) or match.group(2) or match.group(3)
                     or match.group(4) or match.group(5) or "").strip()
            if value.startswith("data:"):
                continue
            if is_external(value):
                self.problems.append(f"external asset in {where}: {value}")
            else:
                self.refs.append(("css", "url", value))


def is_external(value):
    return urlparse(value).scheme in ("http", "https") or value.startswith("//")


def check_page(path, site, ids_by_page, problems):
    parser = PageParser()
    parser.feed(path.read_text(encoding="utf-8"))
    rel = path.relative_to(site)
    if not parser.title.strip():
        problems.append(f"{rel}: no <title>")
    if parser.h1_count != 1:
        problems.append(f"{rel}: expected exactly one <h1>, found {parser.h1_count}")
    problems.extend(f"{rel}: {p}" for p in parser.problems)
    ids_by_page[path] = parser.ids
    return parser


def check_refs(path, parser, site, ids_by_page, problems):
    rel = path.relative_to(site)
    for tag, attr, value in parser.refs:
        if value.startswith("data:") or value.startswith(("mailto:", "tel:")):
            continue
        if is_external(value):
            if tag != "a":
                problems.append(f"{rel}: external {tag} {attr} {value}")
            continue
        parsed = urlparse(value)
        target = path if not parsed.path else (path.parent / unquote(parsed.path)).resolve()
        if parsed.path and target.is_dir():
            target = target / "index.html"
        if not target.exists():
            problems.append(f"{rel}: {tag} {attr} {value} does not resolve")
            continue
        if site.resolve() not in target.resolve().parents and target.resolve() != site.resolve():
            problems.append(f"{rel}: {value} points outside the site")
            continue
        if parsed.fragment and target.suffix == ".html":
            known = ids_by_page.get(target) or ids_by_page.get(target.resolve())
            if known is not None and parsed.fragment not in known:
                problems.append(f"{rel}: anchor #{parsed.fragment} not found in {target.name}")


def check_budget(site, problems):
    text_total = sum(f.stat().st_size for f in site.rglob("*") if f.suffix in TEXT_SUFFIXES)
    if text_total > TEXT_BUDGET_BYTES:
        problems.append(f"text files total {text_total} bytes, budget {TEXT_BUDGET_BYTES}")
    for f in site.rglob("*"):
        if f.suffix in IMAGE_SUFFIXES and f.stat().st_size > IMAGE_BUDGET_BYTES:
            problems.append(f"{f.relative_to(site)} is {f.stat().st_size} bytes, budget {IMAGE_BUDGET_BYTES}")
    return text_total


def main(argv):
    site = Path(argv[1] if len(argv) > 1 else "site").resolve()
    if not site.is_dir():
        print(f"error: {site} is not a directory", file=sys.stderr)
        return 2
    pages = sorted(site.rglob("*.html"))
    if not pages:
        print(f"error: no .html files in {site}", file=sys.stderr)
        return 2
    problems = []
    ids_by_page = {}
    parsed = {p: check_page(p, site, ids_by_page, problems) for p in pages}
    for p, parser in parsed.items():
        check_refs(p, parser, site, ids_by_page, problems)
    for required in ("404.html", "robots.txt", "index.html"):
        if not (site / required).exists():
            problems.append(f"missing {required}")
    text_total = check_budget(site, problems)
    if problems:
        print("site check FAILED:")
        for p in problems:
            print(f"  - {p}")
        return 1
    print(f"site check ok: {len(pages)} pages, {text_total} bytes of text")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
