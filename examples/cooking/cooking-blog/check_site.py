#!/usr/bin/env python3
"""Verify the immutable exported site without a source checkout or network."""

from html.parser import HTMLParser
from pathlib import Path


class Page(HTMLParser):
    def __init__(self):
        super().__init__()
        self.has_title = False
        self.has_heading = False

    def handle_starttag(self, tag, attrs):
        assert tag != "script", "unexpected script in the static recipe site"
        self.has_title |= tag == "title"
        self.has_heading |= tag == "h1"


def main():
    public = Path(__file__).resolve().parent.parent / "public"
    assert (public / "index.html").is_file(), "missing exported site index"
    pages = sorted(public.rglob("*.html"))
    assert 0 < len(pages) <= 10_000, "unexpected exported page count"
    for path in pages:
        assert not path.is_symlink(), "unexpected linked page"
        assert path.stat().st_size <= 1_048_576, "exported page exceeds size bound"
        page = Page()
        page.feed(path.read_text(encoding="utf-8"))
        page.close()
        assert page.has_title and page.has_heading, "missing exported page structure"
    print(f"Verified {len(pages)} immutable HTML pages.")


if __name__ == "__main__":
    main()
