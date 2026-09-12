#!/usr/bin/env python3
"""Put the shared charter back inside each document.

The three pages under `docs/` are meant to work on their own: opened from a
folder, saved from a browser, sent to somebody as one file. A page that
depends on a sibling stylesheet does none of those -- it renders as raw text
the moment it travels alone, which is how most people will first meet it.

So `valhsync.css` and `valhsync.js` are the source, and this copies them into
every page. Run it after editing either; `--check` fails if any page has
drifted, which is what CI asks.

    python scripts/docs-inline.py
    python scripts/docs-inline.py --check
"""

from __future__ import annotations

import io
import sys
from pathlib import Path

DOCS = Path(__file__).resolve().parent.parent / "docs"
PAGES = ["index.html", "server-guide.html", "player-guide.html"]

OPEN_STYLE = "<style>"
CLOSE_STYLE = "</style>"
OPEN_SCRIPT = "<script>"
CLOSE_SCRIPT = "</script>"

# What a page carries when it has not been inlined yet.
LINK_CSS = '<link rel="stylesheet" href="valhsync.css">'
LINK_JS = '<script src="valhsync.js" defer></script>'


def read(path: Path) -> str:
    return io.open(path, encoding="utf-8").read()


def blocks() -> tuple[str, str]:
    css = read(DOCS / "valhsync.css").strip()
    js = read(DOCS / "valhsync.js").strip()
    return (
        f"{OPEN_STYLE}\n{css}\n{CLOSE_STYLE}",
        f'<script defer>\n{js}\n{CLOSE_SCRIPT}',
    )


def replace_between(text: str, opener: str, closer: str, block: str) -> str:
    """Swap the first opener..closer run for `block`, or return text unchanged."""
    start = text.find(opener)
    if start == -1:
        return text
    end = text.find(closer, start)
    if end == -1:
        return text
    return text[:start] + block + text[end + len(closer) :]


def inlined(page: str, style: str, script: str) -> str:
    text = read(DOCS / page)
    # From a linked page, or from one already inlined: both end up the same.
    if LINK_CSS in text:
        text = text.replace(LINK_CSS, style, 1)
    else:
        text = replace_between(text, OPEN_STYLE, CLOSE_STYLE, style)
    if LINK_JS in text:
        text = text.replace(LINK_JS, script, 1)
    else:
        text = replace_between(text, "<script defer>", CLOSE_SCRIPT, script)
    return text


def main() -> int:
    check = "--check" in sys.argv
    style, script = blocks()
    stale: list[str] = []

    for page in PAGES:
        path = DOCS / page
        current = read(path)
        wanted = inlined(page, style, script)
        if current == wanted:
            continue
        if check:
            stale.append(page)
            continue
        io.open(path, "w", encoding="utf-8", newline="\n").write(wanted)
        print(f"inlined {page}")

    if stale:
        print("these pages no longer carry the shared charter:", file=sys.stderr)
        for page in stale:
            print(f"  docs/{page}", file=sys.stderr)
        print("run: python scripts/docs-inline.py", file=sys.stderr)
        return 1
    if check:
        print(f"{len(PAGES)} pages carry the charter as written")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
