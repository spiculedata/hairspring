#!/usr/bin/env python3
"""Check the static site in site/ without a browser or any network access.

Checks, for every .html file: a <title>, exactly one <h1> (404.html included), local links, anchors and images that
resolve, <img> with alt, width and height, no external script, stylesheet or font URLs, no inline event handlers.
Also checks the size budget (HTML, CSS and JS together; images are counted separately).

Usage: python3 scripts/check-site.py [site-dir]
Exit status: 0 when every check passes, 1 otherwise.
"""
import sys
from html.parser import HTMLParser
from pathlib import Path
from urllib.parse import unquote, urlparse

TEXT_BUDGET_BYTES = 60 * 1024
IMAGE_BUDGET_BYTES = 150 * 1024
TEXT_SUFFIXES = {".html", ".css", ".js", ".txt", ".svg"}
IMAGE_SUFFIXES = {".webp", ".png", ".jpg", ".jpeg", ".gif", ".avif"}


class PageParser(HTMLParser):
    def __init__(self):
        super().__init__(convert_charrefs=True)
        self.title = ""
        self.h1_count = 0
        self.ids = set()
        self.refs = []  # (tag, attribute, value)
        self.problems = []
        self._in_title = False

    def handle_starttag(self, tag, attrs):
        a = dict(attrs)
        if tag == "title":
            self._in_title = True
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
        if tag == "script":
            if a.get("src"):
                self.refs.append((tag, "src", a["src"]))
        if tag == "link":
            rel = (a.get("rel") or "").lower()
            if a.get("href") and ("stylesheet" in rel or "icon" in rel or "preload" in rel):
                self.refs.append((tag, "href", a["href"]))
        if tag in ("img", "source"):
            for attr in ("src", "srcset"):
                if a.get(attr):
                    for part in a[attr].split(","):
                        self.refs.append((tag, attr, part.strip().split()[0]))

    def _check_img(self, a):
        if "alt" not in a:
            self.problems.append(f"<img src={a.get('src')!r}> has no alt attribute")
        if not a.get("width") or not a.get("height"):
            self.problems.append(f"<img src={a.get('src')!r}> has no width and height")

    def handle_endtag(self, tag):
        if tag == "title":
            self._in_title = False

    def handle_data(self, data):
        if self._in_title:
            self.title += data


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
            if tag in ("script", "link", "img", "source"):
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
