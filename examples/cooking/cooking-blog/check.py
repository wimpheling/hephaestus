#!/usr/bin/env python3
"""Offline source check declared by the cooking blog; Hugo builds HTML separately."""
from pathlib import Path
import tomllib

root = Path('.')
tomllib.loads((root / 'hugo.toml').read_text())
for name in ('content/_index.md', 'content/recipes/_index.md', 'layouts/_default/baseof.html'):
    assert (root / name).is_file(), 'missing site source'
for page in (root / 'content/recipes').glob('recipe-*.md'):
    text = page.read_text()
    assert len(text.encode()) <= 16384 and text.startswith('+++\n'), 'invalid page'
    front, body = text[4:].split('\n+++\n', 1)
    metadata = tomllib.loads(front)
    assert isinstance(metadata['title'], str) and metadata['draft'] is False
    assert '{{' not in body and '<script' not in body.lower(), 'unsafe recipe body'
    assert '## Ingredients' in body and '## Method' in body, 'incomplete recipe'
